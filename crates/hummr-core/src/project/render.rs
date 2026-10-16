//! Offline rendering of a [`Song`]: clips → tracks → master.
//!
//! Per clip: read the source region (or slices, or notes), apply speed,
//! pitch and reverse, tile repeats, fade, run the clip's effects, apply
//! gain. Clips are panned into their track's stereo buffer. Per track: the
//! track's effects. The mix applies each track's volume, balance, mute and
//! solo, then the master effects and a limiter.

use super::{Clip, ClipKind, Song, SourceBank, Stretch, Voice};
use crate::AudioBuffer;
use crate::dsp::db_to_amp;
use crate::fx;
use crate::preview::pan_gains;

/// A stereo buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stereo {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl Stereo {
    pub fn silent(len: usize) -> Self {
        Self {
            left: vec![0.0; len],
            right: vec![0.0; len],
        }
    }

    pub fn len(&self) -> usize {
        self.left.len().max(self.right.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn grow(&mut self, len: usize) {
        if self.left.len() < len {
            self.left.resize(len, 0.0);
        }
        if self.right.len() < len {
            self.right.resize(len, 0.0);
        }
    }

    /// Adds a mono signal at `offset` with per-channel gains.
    pub fn add_mono(&mut self, mono: &[f32], offset: i64, gl: f32, gr: f32) {
        let end = offset + mono.len() as i64;
        if end <= 0 {
            return;
        }
        self.grow(end as usize);
        for (i, &x) in mono.iter().enumerate() {
            let j = offset + i as i64;
            if j >= 0 {
                self.left[j as usize] += x * gl;
                self.right[j as usize] += x * gr;
            }
        }
    }

    /// Interleaved L/R samples, for WAV files.
    pub fn interleaved(&self) -> Vec<f32> {
        let n = self.len();
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            out.push(self.left.get(i).copied().unwrap_or(0.0));
            out.push(self.right.get(i).copied().unwrap_or(0.0));
        }
        out
    }
}

/// Options for a full mixdown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixOptions {
    /// Run the master limiter (ceiling -1 dBFS).
    pub limiter: bool,
}

impl Default for MixOptions {
    fn default() -> Self {
        Self { limiter: true }
    }
}

/// Master limiter ceiling.
pub const CEILING_DB: f64 = -1.0;

/// Linear stereo balance: centre is unity on both sides.
pub fn balance_gains(pan: f64) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0) as f32;
    if p < 0.0 {
        (1.0, 1.0 + p)
    } else {
        (1.0 - p, 1.0)
    }
}

/// Sample position of a beat.
pub fn beat_to_sample(song: &Song, beat: f64) -> i64 {
    (beat * song.seconds_per_beat() * f64::from(song.sample_rate)).round() as i64
}

/// Where a clip's first sample lands on the timeline.
pub fn clip_offset(song: &Song, clip: &Clip) -> i64 {
    beat_to_sample(song, clip.start_beat)
}

/// `len` seconds of `audio` from `start` seconds, zero-padded outside.
pub fn region(audio: &AudioBuffer, start: f64, len: f64) -> Vec<f32> {
    let sr = f64::from(audio.sample_rate);
    let s = (start * sr).round() as i64;
    let n = (len * sr).round().max(0.0) as usize;
    (0..n as i64)
        .map(|i| {
            let j = s + i;
            if j >= 0 {
                audio.samples.get(j as usize).copied().unwrap_or(0.0)
            } else {
                0.0
            }
        })
        .collect()
}

/// Linear fade-in over the first `n` samples.
pub fn fade_in(buf: &mut [f32], n: usize) {
    let n = n.min(buf.len());
    for (i, s) in buf.iter_mut().take(n).enumerate() {
        *s *= i as f32 / n as f32;
    }
}

/// Linear fade-out over the last `n` samples, complementary to
/// [`fade_in`]: over the same `n` samples the two gains sum to exactly 1.
pub fn fade_out(buf: &mut [f32], n: usize) {
    let len = buf.len();
    let n = n.min(len);
    for k in 0..n {
        buf[len - n + k] *= (n - k) as f32 / n as f32;
    }
}

/// How much longer than its source the audio of one repeat must become.
fn time_ratio(song: &Song, clip: &Clip) -> f64 {
    let follow = match clip.stretch {
        Stretch::Keep => 1.0,
        Stretch::Follow { bpm } => bpm / song.bpm,
    };
    follow / clip.speed.max(1e-3)
}

/// One repeat of a clip, mono, before fades, effects and gain. May run past
/// the unit length (notes ring out).
/// The source with the clip's region (plus a little either side, for
/// slice crossfades) noise-cleaned, if the clip asks for it.
pub(crate) fn cleaned_source(audio: &AudioBuffer, clip: &Clip) -> Option<AudioBuffer> {
    if clip.clean <= 0.0 || audio.samples.is_empty() {
        return None;
    }
    let sr = audio.sample_rate;
    let profile = match clip.noise_region {
        Some([start, len]) if len > 0.0 => crate::denoise::learn(&region(audio, start, len), sr),
        _ => crate::denoise::learn_default(&region(audio, clip.source_start, clip.source_len), sr),
    };
    let margin = 0.05;
    let from = ((clip.source_start - margin).max(0.0) * f64::from(sr)).round() as usize;
    let to = (((clip.source_start + clip.source_len + margin) * f64::from(sr)).round() as usize)
        .min(audio.samples.len());
    if from >= to {
        return None;
    }
    let mut out = audio.clone();
    let cleaned = crate::denoise::denoise(&audio.samples[from..to], sr, &profile, clip.clean);
    out.samples[from..to].copy_from_slice(&cleaned);
    Some(out)
}

fn render_unit(song: &Song, clip: &Clip, bank: &SourceBank) -> Vec<f32> {
    let sr = song.sample_rate;
    let empty = AudioBuffer::new(Vec::new(), sr);
    let original = bank.get(&clip.source).map_or(&empty, |a| a.as_ref());
    let cleaned = match clip.kind {
        // Only the sampler plays the source itself.
        ClipKind::Notes {
            voice: Voice::Sampler { .. },
            ..
        }
        | ClipKind::Audio
        | ClipKind::Sliced { .. } => cleaned_source(original, clip),
        ClipKind::Notes { .. } => None,
    };
    let audio = cleaned.as_ref().unwrap_or(original);
    match &clip.kind {
        ClipKind::Audio => {
            let mut buf = region(audio, clip.source_start, clip.source_len);
            if clip.reverse {
                buf.reverse();
            }
            let ratio = time_ratio(song, clip);
            crate::stretch::stretch_and_shift(&buf, sr, ratio, clip.pitch)
        }
        ClipKind::Sliced { slices, .. } => crate::slice::render_slices(
            audio,
            slices,
            song.seconds_per_beat(),
            clip.pitch,
            clip.reverse,
        ),
        ClipKind::Notes { notes, voice, .. } => {
            crate::voice::render_notes(song, clip, audio, notes, voice)
        }
    }
}

/// Renders a clip to mono: sample 0 is the clip start. Includes fades,
/// effects (with their tails) and gain, but not pan.
pub fn render_clip(song: &Song, clip: &Clip, bank: &SourceBank) -> Vec<f32> {
    let sr = f64::from(song.sample_rate);
    let unit = render_unit(song, clip, bank);
    let unit_beats = clip.unit_beats(song.bpm);
    let total = (clip.len_beats(song.bpm) * song.seconds_per_beat() * sr).round() as usize;
    let mut out = if clip.repeat <= 1 {
        let mut u = unit;
        // Audio clips are exactly their length; notes may ring out.
        if matches!(clip.kind, ClipKind::Audio) {
            u.resize(total, 0.0);
        }
        u
    } else {
        let mut out = vec![0.0f32; total];
        for k in 0..clip.repeat {
            let at = (f64::from(k) * unit_beats * song.seconds_per_beat() * sr).round() as usize;
            if out.len() < at + unit.len() {
                out.resize(at + unit.len(), 0.0);
            }
            for (i, &x) in unit.iter().enumerate() {
                out[at + i] += x;
            }
        }
        out
    };
    // Fades are measured against the clip's length on the timeline.
    let fi = (clip.fade_in * sr) as usize;
    if fi > 0 {
        fade_in(&mut out, fi);
    }
    let fo = (clip.fade_out * sr) as usize;
    if fo > 0 {
        let end = total.min(out.len());
        fade_out(&mut out[..end], fo);
        out.truncate(end);
    }
    if !clip.fx.is_empty() {
        out = fx::apply_chain(&clip.fx, &out, song.sample_rate, song.bpm, 0);
    }
    let g = db_to_amp(clip.gain_db as f32);
    if g != 1.0 {
        for s in &mut out {
            *s *= g;
        }
    }
    out
}

/// Renders one track (all its clips, panned, then the track's effects).
/// Volume, balance, mute and solo are left to the mix so the mixer can
/// change them without re-rendering.
pub fn render_track(song: &Song, track: usize, bank: &SourceBank) -> Stereo {
    let mut out = Stereo::default();
    let Some(t) = song.tracks.get(track) else {
        return out;
    };
    for clip in &t.clips {
        let mono = render_clip(song, clip, bank);
        let (gl, gr) = pan_gains(clip.pan as f32);
        out.add_mono(&mono, clip_offset(song, clip), gl, gr);
    }
    if !t.fx.is_empty() && !out.is_empty() {
        out.left = fx::apply_chain(&t.fx, &out.left, song.sample_rate, song.bpm, 0);
        out.right = fx::apply_chain(&t.fx, &out.right, song.sample_rate, song.bpm, 1);
    }
    out
}

/// Mixes pre-rendered tracks (one per song track, as from
/// [`render_track`]) applying volume, balance, mute/solo, master effects and
/// the limiter.
pub fn mix_tracks(song: &Song, tracks: &[Stereo], opts: MixOptions) -> Stereo {
    let len = tracks.iter().map(Stereo::len).max().unwrap_or(0);
    let mut out = Stereo::silent(len);
    for (i, t) in tracks.iter().enumerate() {
        if !song.audible(i) {
            continue;
        }
        let tr = &song.tracks[i];
        let g = db_to_amp(tr.volume_db as f32);
        let (bl, br) = balance_gains(tr.pan);
        for (o, x) in out.left.iter_mut().zip(&t.left) {
            *o += x * g * bl;
        }
        for (o, x) in out.right.iter_mut().zip(&t.right) {
            *o += x * g * br;
        }
    }
    if !song.master_fx.is_empty() && !out.is_empty() {
        out.left = fx::apply_chain(&song.master_fx, &out.left, song.sample_rate, song.bpm, 0);
        out.right = fx::apply_chain(&song.master_fx, &out.right, song.sample_rate, song.bpm, 1);
    }
    if opts.limiter {
        fx::limit_linked(
            &mut [&mut out.left, &mut out.right],
            song.sample_rate,
            CEILING_DB,
        );
    }
    out
}

/// Renders the whole song to stereo at the song's sample rate.
pub fn mixdown(song: &Song, bank: &SourceBank, opts: MixOptions) -> Stereo {
    let tracks: Vec<Stereo> = (0..song.tracks.len())
        .map(|i| render_track(song, i, bank))
        .collect();
    mix_tracks(song, &tracks, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{PROJECT_RATE, Slice};
    use std::sync::Arc;

    /// A ramp source so any misplacement shows up as a wrong value.
    fn bank() -> SourceBank {
        let mut b = SourceBank::default();
        let ramp: Vec<f32> = (0..96_000).map(|i| (i % 1000) as f32 / 4000.0).collect();
        b.insert("ramp", Arc::new(AudioBuffer::new(ramp, PROJECT_RATE)));
        let dc: Vec<f32> = vec![0.1; 96_000];
        b.insert("dc", Arc::new(AudioBuffer::new(dc, PROJECT_RATE)));
        b
    }

    #[test]
    fn two_tracks_with_overlapping_clips_mix_to_the_hand_computed_sum() {
        let bank = bank();
        let mut song = Song::new("t");
        song.bpm = 120.0; // 24 000 samples per beat
        let a = song.add_track("A");
        let b = song.add_track("B");
        // Track A: two overlapping clips (the second sits in lane 2).
        let c1 = Clip::audio(song.alloc_id(), "r1", "ramp", 0.0, 0.5, 0.0); // beats 0..1
        let mut c2 = Clip::audio(song.alloc_id(), "r2", "ramp", 0.25, 0.5, 0.5); // beats 0.5..1.5
        c2.gain_db = -6.0;
        song.tracks[a].clips = vec![c1, c2];
        assert_eq!(song.lane_count(a), 2);
        // Track B: a DC clip panned hard left, track at -6 dB and panned right.
        let mut c3 = Clip::audio(song.alloc_id(), "d", "dc", 0.0, 0.25, 1.0); // beats 1..1.5
        c3.pan = -1.0;
        song.tracks[b].clips = vec![c3];
        song.tracks[b].volume_db = -6.0;
        song.tracks[b].pan = 0.5;

        let mix = mixdown(&song, &bank, MixOptions { limiter: false });
        assert_eq!(mix.len(), 36_000);

        let ramp = |i: usize| (i % 1000) as f32 / 4000.0;
        let centre = std::f32::consts::FRAC_1_SQRT_2;
        let g6 = db_to_amp(-6.0);
        for i in 0..36_000 {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            if i < 24_000 {
                l += ramp(i) * centre;
                r += ramp(i) * centre;
            }
            if (12_000..36_000).contains(&i) {
                let x = ramp(i - 12_000 + 12_000) * g6; // source starts at 0.25 s = 12 000
                l += x * centre;
                r += x * centre;
            }
            if (24_000..36_000).contains(&i) {
                // pan -1: all left; track balance 0.5 keeps half the left.
                l += 0.1 * g6 * 0.5;
            }
            assert!(
                (mix.left[i] - l).abs() < 1e-6,
                "L[{i}]: {} vs {l}",
                mix.left[i]
            );
            assert!(
                (mix.right[i] - r).abs() < 1e-6,
                "R[{i}]: {} vs {r}",
                mix.right[i]
            );
        }
    }

    #[test]
    fn mute_and_solo() {
        let bank = bank();
        let mut song = Song::new("t");
        song.bpm = 120.0;
        for name in ["A", "B"] {
            let t = song.add_track(name);
            let id = song.alloc_id();
            song.tracks[t]
                .clips
                .push(Clip::audio(id, "d", "dc", 0.0, 0.1, 0.0));
        }
        let full = mixdown(&song, &bank, MixOptions { limiter: false });
        song.tracks[0].mute = true;
        let muted = mixdown(&song, &bank, MixOptions { limiter: false });
        assert!((muted.left[100] - full.left[100] / 2.0).abs() < 1e-6);
        song.tracks[0].mute = false;
        song.tracks[1].solo = true;
        let solo = mixdown(&song, &bank, MixOptions { limiter: false });
        assert_eq!(solo.left[100], muted.left[100]);
    }

    #[test]
    fn split_then_play_equals_the_original() {
        let bank = bank();
        let mut song = Song::new("t");
        song.bpm = 96.0;
        let t = song.add_track("A");
        let id = song.alloc_id();
        song.tracks[t]
            .clips
            .push(Clip::audio(id, "r", "ramp", 0.1, 1.5, 0.5));
        let before = mixdown(&song, &bank, MixOptions { limiter: false });
        song.split_clip(id, 1.75).unwrap();
        assert_eq!(song.tracks[t].clips.len(), 2);
        let after = mixdown(&song, &bank, MixOptions { limiter: false });
        assert_eq!(before.len(), after.len());
        for i in 0..before.len() {
            assert!((before.left[i] - after.left[i]).abs() < 1e-6, "sample {i}");
        }
    }

    #[test]
    fn repeats_tile_and_reverse_flips() {
        let bank = bank();
        let mut song = Song::new("t");
        song.bpm = 120.0;
        let t = song.add_track("A");
        let mut c = Clip::audio(song.alloc_id(), "r", "ramp", 0.0, 0.01, 0.0); // 480 samples
        c.repeat = 3;
        song.tracks[t].clips.push(c.clone());
        let m = render_clip(&song, &c, &bank);
        assert_eq!(m.len(), 1_440);
        assert_eq!(m[5], m[485]);
        assert_eq!(m[5], m[965]);
        c.repeat = 1;
        c.reverse = true;
        let r = render_clip(&song, &c, &bank);
        assert_eq!(r[0], 479.0 / 4000.0);
    }

    #[test]
    fn clean_turns_down_hiss_in_the_rendered_clip() {
        // Half a second of hiss, then a tone over it.
        let mut state = 9u64;
        let audio: Vec<f32> = (0..96_000)
            .map(|i| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let n = ((state >> 33) as f32 / (1u64 << 31) as f32 * 2.0 - 1.0) * 0.02;
                let t = if i >= 24_000 {
                    0.3 * (std::f32::consts::TAU * 330.0 * i as f32 / 48_000.0).sin()
                } else {
                    0.0
                };
                n + t
            })
            .collect();
        let mut bank = SourceBank::default();
        bank.insert("take", Arc::new(AudioBuffer::new(audio, PROJECT_RATE)));
        let mut song = Song::new("t");
        let mut c = Clip::audio(1, "c", "take", 0.0, 2.0, 0.0);
        let dry = render_clip(&song, &c, &bank);
        c.clean = 0.8;
        let wet = render_clip(&song, &c, &bank);
        assert_eq!(dry.len(), wet.len());
        let floor = |x: &[f32]| crate::dsp::amp_to_db(crate::dsp::rms(&x[2_000..22_000]));
        assert!(floor(&dry) - floor(&wet) >= 12.0);
        // A hand-picked noise region does the same.
        c.noise_region = Some([0.05, 0.4]);
        let picked = render_clip(&song, &c, &bank);
        assert!(floor(&dry) - floor(&picked) >= 12.0);
        // Notes clips ignore it; sliced clips use the cleaned audio.
        song.bpm = 120.0;
        c.kind = ClipKind::Sliced {
            length_beats: 4.0,
            slices: vec![Slice::new(0.0, 2.0, 0.0)],
            grid: 0.5,
            strength: 1.0,
        };
        let sliced = render_clip(&song, &c, &bank);
        assert!(floor(&dry) - floor(&sliced) >= 12.0);
    }

    #[test]
    fn sliced_clip_at_original_positions_matches_the_audio() {
        let bank = bank();
        let mut song = Song::new("t");
        song.bpm = 120.0;
        let t = song.add_track("A");
        let audio = Clip::audio(song.alloc_id(), "r", "ramp", 0.0, 1.0, 0.0);
        let mut sliced = audio.clone();
        sliced.kind = ClipKind::Sliced {
            length_beats: 2.0,
            slices: vec![
                Slice::new(0.0, 0.3, 0.0),
                Slice::new(0.3, 0.45, 0.6),
                Slice::new(0.75, 0.25, 1.5),
            ],
            grid: 0.0,
            strength: 0.0,
        };
        song.tracks[t].clips.push(audio.clone());
        let a = render_clip(&song, &audio, &bank);
        let s = render_clip(&song, &sliced, &bank);
        for i in 0..a.len() {
            assert!(
                (a[i] - s[i]).abs() < 1e-5,
                "sample {i}: {} vs {}",
                a[i],
                s[i]
            );
        }
    }

    #[test]
    fn limiter_keeps_the_master_under_the_ceiling() {
        let mut bank = SourceBank::default();
        bank.insert(
            "loud",
            Arc::new(AudioBuffer::new(vec![0.9; 4_800], PROJECT_RATE)),
        );
        let mut song = Song::new("t");
        for _ in 0..4 {
            let t = song.add_track("x");
            let id = song.alloc_id();
            song.tracks[t]
                .clips
                .push(Clip::audio(id, "l", "loud", 0.0, 0.1, 0.0));
        }
        let m = mixdown(&song, &bank, MixOptions::default());
        let ceiling = db_to_amp(CEILING_DB as f32);
        assert!(
            m.left
                .iter()
                .chain(&m.right)
                .all(|s| s.abs() <= ceiling + 1e-6)
        );
    }
}
