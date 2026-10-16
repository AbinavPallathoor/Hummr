//! Slicing a clip at its hits and placing the slices on a beat grid.
//!
//! A clip is cut a few milliseconds before each hit. Every slice is anchored
//! to a beat position, so the slices follow the song when its tempo changes
//! while each keeps its natural sound (no stretching). Snapping moves each
//! slice so its *attack* lands on (or, at lower strength, towards) the
//! nearest grid line. Neighbouring slices overlap by a 5 ms crossfade, so
//! slices left where they were sum back to the original exactly.

use crate::AudioBuffer;
use crate::project::render::{fade_in, fade_out, region};
use crate::project::{Clip, ClipKind, Slice};

/// Crossfade length at each cut (seconds).
pub const XFADE: f64 = 0.005;

/// Cuts are made this long before each attack, so the crossfade has
/// finished before the hit starts.
pub const PRE: f64 = 0.004;

/// Cuts a source region into slices at `onsets` (attack times in source
/// seconds). `to_beat` maps a source time to a clip-relative beat; it gives
/// each slice its natural position. Cuts fall on whole samples at
/// `sample_rate`.
pub fn cut_slices(
    region_start: f64,
    region_len: f64,
    onsets: &[f64],
    sample_rate: u32,
    to_beat: impl Fn(f64) -> f64,
) -> Vec<Slice> {
    let sr = f64::from(sample_rate);
    let on_sample = |t: f64| (t * sr).round() / sr;
    let end = region_start + region_len;
    let mut inside: Vec<f64> = onsets
        .iter()
        .copied()
        .filter(|&t| t >= region_start && t < end - 0.01)
        .collect();
    inside.sort_by(f64::total_cmp);

    // (cut time, attack time)
    let mut cuts: Vec<(f64, Option<f64>)> = vec![(region_start, None)];
    for t in inside {
        let cut = on_sample(t - PRE);
        let last = cuts.last_mut().expect("starts non-empty");
        if cut <= last.0 + 0.01 {
            // Too close to the previous cut: this hit belongs to that slice.
            if last.1.is_none() {
                last.1 = Some(t);
            }
            continue;
        }
        cuts.push((cut, Some(t)));
    }
    let mut out = Vec::with_capacity(cuts.len());
    for (i, &(cut, attack)) in cuts.iter().enumerate() {
        let next = cuts.get(i + 1).map_or(end, |c| c.0);
        let beat = to_beat(cut);
        out.push(Slice {
            src: cut,
            len: next - cut,
            beat,
            hit: attack.map(|a| (a - cut).max(0.0)),
            natural: Some(beat),
        });
    }
    out
}

/// Moves slices towards the grid: each hit's attack goes `strength` (0..=1)
/// of the way to the nearest grid line. `clip_start` is the clip's song
/// beat (grid lines are song-wide); `seconds_per_beat` is the song's. A grid
/// of 0 puts every slice back at its natural position.
pub fn snap_slices(
    slices: &mut [Slice],
    clip_start: f64,
    grid: f64,
    strength: f64,
    seconds_per_beat: f64,
) {
    let strength = strength.clamp(0.0, 1.0);
    for s in slices {
        let natural = *s.natural.get_or_insert(s.beat);
        s.beat = match s.hit {
            Some(h) if grid > 0.0 && strength > 0.0 => {
                let hit = clip_start + natural + h / seconds_per_beat;
                let target = (hit / grid).round() * grid;
                natural + strength * (target - hit)
            }
            _ => natural,
        };
    }
}

/// Turns an audio clip into a sliced clip that sits on the song's beat
/// grid. `natural_bpm` is the clip's own tempo and `beat_time` the source
/// time of one of its beats; `onsets` are attack times in source seconds.
///
/// Slice positions are measured in the clip's own beats, so playing the
/// clip at the song tempo re-times it without stretching. The clip moves
/// (by under half a beat) so its beats line up with the song's, then hits
/// are snapped with `strength` to `grid` (beats; 0 for none).
#[allow(clippy::too_many_arguments)]
pub fn slice_clip(
    clip: &Clip,
    sample_rate: u32,
    onsets: &[f64],
    natural_bpm: f64,
    beat_time: f64,
    grid: f64,
    strength: f64,
    song_bpm: f64,
) -> Clip {
    let spb = 60.0 / natural_bpm;
    let r0 = clip.source_start;
    // Whole beats from the region start to the first beat at or after it.
    let k0 = ((beat_time - r0) / spb - 1e-9).ceil();
    let rel = |t: f64| (t - beat_time) / spb + k0;
    let mut slices = cut_slices(r0, clip.source_len, onsets, sample_rate, rel);
    let start = (clip.start_beat - rel(r0)).round().max(0.0);
    let length_beats = (rel(r0 + clip.source_len) - 1e-3).ceil().max(1.0);
    snap_slices(&mut slices, start, grid, strength, 60.0 / song_bpm);
    let mut out = clip.clone();
    out.start_beat = start;
    out.kind = ClipKind::Sliced {
        length_beats,
        slices,
        grid,
        strength,
    };
    out
}

/// Changes the snap grid and strength of a sliced clip in place.
pub fn resnap(clip: &mut Clip, grid: f64, strength: f64, song_bpm: f64) {
    let start = clip.start_beat;
    if let ClipKind::Sliced {
        slices,
        grid: g,
        strength: st,
        ..
    } = &mut clip.kind
    {
        *g = grid;
        *st = strength;
        snap_slices(slices, start, grid, strength, 60.0 / song_bpm);
    }
}

/// Renders slices (positions in beats from the clip start) into one mono
/// buffer. Neighbouring slices overlap by [`XFADE`] with complementary
/// linear fades centred on the cut, so slices left at their original
/// positions sum back to the original audio exactly. A slice that would run
/// into the next one (after a tempo increase or snapping) is cut short there.
pub fn render_slices(
    audio: &AudioBuffer,
    slices: &[Slice],
    seconds_per_beat: f64,
    semitones: f64,
    reverse: bool,
) -> Vec<f32> {
    let sr = f64::from(audio.sample_rate);
    let h = (XFADE * sr / 2.0).round() as i64; // half the crossfade, samples
    let mut order: Vec<&Slice> = slices.iter().collect();
    order.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let mut out: Vec<f32> = Vec::new();
    for (i, s) in order.iter().enumerate() {
        let first = i == 0;
        let last = i + 1 == order.len();
        let at = (s.beat * seconds_per_beat * sr).round() as i64;
        let mut len = (s.len * sr).round() as i64;
        let mut cut_short = false;
        if let Some(next) = order.get(i + 1) {
            let next_at = (next.beat * seconds_per_beat * sr).round() as i64;
            if next_at - at < len {
                len = (next_at - at).max(0);
                cut_short = true;
            }
        }
        if len == 0 {
            continue;
        }
        let pre = if first { 0 } else { h };
        let post = if last && !cut_short { 0 } else { h };
        let src0 = (s.src * sr).round() as i64 - pre;
        let n = (len + pre + post) as usize;
        let mut seg = region(audio, src0 as f64 / sr, n as f64 / sr);
        seg.resize(n, 0.0);
        if reverse {
            seg[pre as usize..(pre + len) as usize].reverse();
        }
        if semitones != 0.0 {
            seg = crate::stretch::stretch_and_shift(&seg, audio.sample_rate, 1.0, semitones);
            seg.resize(n, 0.0);
        }
        if pre > 0 {
            fade_in(&mut seg, (2 * pre) as usize);
        }
        if post > 0 {
            fade_out(&mut seg, (2 * post) as usize);
        }
        let start = at - pre;
        let end = start + n as i64;
        if end > out.len() as i64 {
            out.resize(end.max(0) as usize, 0.0);
        }
        for (k, &x) in seg.iter().enumerate() {
            let j = start + k as i64;
            if j >= 0 {
                out[j as usize] += x;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beat::phase_for_tempo;
    use crate::onset::detect_onsets;
    use crate::project::render::render_clip;
    use crate::project::{PROJECT_RATE, Song, SourceBank};
    use crate::synthetic::{Rng, drum_groove, drum_loop, sine};
    use std::sync::Arc;

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&v| f64::from(v) * f64::from(v)).sum()
    }

    fn max_jump(x: &[f32]) -> f32 {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    /// A drum loop as a project clip, sliced and snapped.
    fn snapped(bpm: f64, which: usize, strength: f64, seed: u64) -> (Vec<f32>, Vec<f32>, Clip) {
        let (audio, _) = drum_groove(bpm, 4, which, 0.23, 0.015, seed, PROJECT_RATE);
        let mut bank = SourceBank::default();
        bank.insert("s", Arc::new(AudioBuffer::new(audio.clone(), PROJECT_RATE)));
        let mut song = Song::new("t");
        song.bpm = bpm;
        let dur = audio.len() as f64 / f64::from(PROJECT_RATE);
        let clip = Clip::audio(1, "loop", "s", 0.0, dur, 0.0);
        let onsets: Vec<f64> = detect_onsets(&audio, PROJECT_RATE)
            .iter()
            .map(|o| o.time)
            .collect();
        let phase = phase_for_tempo(&audio, PROJECT_RATE, bpm);
        let sliced = slice_clip(&clip, PROJECT_RATE, &onsets, bpm, phase, 0.5, strength, bpm);
        let out = render_clip(&song, &sliced, &bank);
        (audio, out, sliced)
    }

    /// M3 acceptance: at strength 1 every hit lands within 5 ms of a grid
    /// line; energy stays within 1 dB; no jump bigger than the original's.
    #[test]
    fn full_strength_puts_every_hit_on_the_grid() {
        let mut worst: f64 = 0.0;
        let mut worst_db: f64 = 0.0;
        for (i, (bpm, which)) in [(96.0, 0), (120.0, 1), (84.0, 2), (140.0, 0)]
            .into_iter()
            .enumerate()
        {
            let (input, out, clip) = snapped(bpm, which, 1.0, 40 + i as u64);
            let step = 0.5 * 60.0 / bpm; // 8th notes, clip starts on a beat
            assert_eq!(clip.start_beat.fract(), 0.0);
            let hits = detect_onsets(&out, PROJECT_RATE);
            assert!(hits.len() >= 20, "{bpm}: only {} hits", hits.len());
            for h in &hits {
                let d = (h.time / step).fract();
                let err = d.min(1.0 - d) * step;
                worst = worst.max(err);
                assert!(
                    err <= 0.005,
                    "{bpm} BPM: hit at {:.4} is {:.1} ms off",
                    h.time,
                    err * 1e3
                );
            }
            let db = 10.0 * (energy(&out) / energy(&input)).log10();
            worst_db = worst_db.max(db.abs());
            assert!(db.abs() <= 1.0, "{bpm} BPM: energy changed {db:.2} dB");
            assert!(
                max_jump(&out) <= max_jump(&input) * 1.05,
                "{bpm} BPM: jump {} vs {}",
                max_jump(&out),
                max_jump(&input)
            );
        }
        eprintln!(
            "snap: worst hit {:.2} ms from the grid, worst energy change {:.2} dB",
            worst * 1e3,
            worst_db
        );
    }

    #[test]
    fn zero_strength_leaves_the_clip_unchanged() {
        let (input, out, clip) = snapped(96.0, 1, 0.0, 7);
        // The clip moved by under half a beat to sit on the grid; undo that.
        let spb = 60.0 / 96.0;
        let ClipKind::Sliced { slices, .. } = &clip.kind else {
            panic!("not sliced");
        };
        let first = slices[0];
        let shift = ((first.beat * spb - first.src) * f64::from(PROJECT_RATE)).round() as i64;
        let mut worst = 0.0f32;
        for (i, &x) in input.iter().enumerate() {
            let j = i as i64 + shift;
            if j >= 0 && (j as usize) < out.len() {
                worst = worst.max((out[j as usize] - x).abs());
            }
        }
        assert!(worst < 1e-5, "max difference {worst}");
    }

    #[test]
    fn moved_slices_of_a_sine_do_not_click() {
        // A continuous tone is the worst case for clicks at cuts.
        let audio = AudioBuffer::new(sine(220.0, PROJECT_RATE, 3.0, 0.5), PROJECT_RATE);
        let mut rng = Rng::new(5);
        let onsets: Vec<f64> = (1..7)
            .map(|i| f64::from(i) * 0.4 + 0.02 * f64::from(rng.bipolar()))
            .collect();
        let spb = 0.5;
        let mut slices = cut_slices(0.0, 3.0, &onsets, PROJECT_RATE, |t| t / spb);
        snap_slices(&mut slices, 0.0, 0.25, 1.0, spb);
        let out = render_slices(&audio, &slices, spb, 0.0, false);
        let bound = max_jump(&audio.samples);
        let jump = max_jump(&out);
        eprintln!("sine: max jump {jump:.4} (tone alone {bound:.4})");
        assert!(jump < bound * 2.5, "click: jump {jump} vs {bound}");
    }

    #[test]
    fn slices_follow_tempo_changes_without_stretching() {
        let (audio, _) = drum_loop(100.0, 2, 0, 0.0, 0.0, 3, PROJECT_RATE);
        let mut bank = SourceBank::default();
        bank.insert("s", Arc::new(AudioBuffer::new(audio.clone(), PROJECT_RATE)));
        let onsets: Vec<f64> = detect_onsets(&audio, PROJECT_RATE)
            .iter()
            .map(|o| o.time)
            .collect();
        let dur = audio.len() as f64 / f64::from(PROJECT_RATE);
        let clip = Clip::audio(1, "l", "s", 0.0, dur, 0.0);
        let sliced = slice_clip(&clip, PROJECT_RATE, &onsets, 100.0, 0.0, 0.5, 1.0, 100.0);
        let mut song = Song::new("t");
        song.bpm = 120.0;
        let out = render_clip(&song, &sliced, &bank);
        // Hits now fall on 120 BPM 8th notes (0.25 s apart).
        for h in detect_onsets(&out, PROJECT_RATE) {
            let d = (h.time / 0.25).fract();
            assert!(d.min(1.0 - d) * 0.25 < 0.005, "hit at {}", h.time);
        }
        assert_eq!(sliced.len_beats(120.0), sliced.len_beats(100.0));
    }

    #[test]
    fn resnap_changes_strength_without_losing_positions() {
        let (_, _, mut clip) = snapped(110.0, 0, 1.0, 9);
        let full = clip.clone();
        resnap(&mut clip, 0.5, 0.0, 110.0);
        resnap(&mut clip, 0.5, 1.0, 110.0);
        assert_eq!(clip, full);
    }
}
