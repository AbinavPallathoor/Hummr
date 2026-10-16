//! Voices that play the notes of a notes clip.
//!
//! Every voice implements [`InstrumentVoice`]: the preview synth (a General
//! MIDI instrument), a sampler that plays a recorded sound at other
//! pitches ("own sound"), and the procedural drum kit.

use std::collections::HashMap;

use crate::AudioBuffer;
use crate::kit::{Sound, drum_hit};
use crate::pitch::yin::yin_pitch;
use crate::preview;
use crate::project::{Clip, Note, Song, Voice};
use crate::sequence::{SeqNote, Sequence};
use crate::theory::hz_to_midi;

/// Something that turns notes into sound.
pub trait InstrumentVoice {
    /// What to call it in the inspector.
    fn describe(&self) -> String;
    /// Renders `notes` (beats from 0) at `bpm`, mono at `sample_rate`. The
    /// result may ring past the last note.
    fn render(&self, notes: &[Note], bpm: f64, sample_rate: u32) -> Vec<f32>;
}

/// The built-in additive synth, one sound per General MIDI family.
pub struct PreviewSynth {
    pub program: u8,
}

impl InstrumentVoice for PreviewSynth {
    fn describe(&self) -> String {
        format!(
            "{} (built-in synth)",
            crate::instruments::name(self.program)
        )
    }

    fn render(&self, notes: &[Note], bpm: f64, sample_rate: u32) -> Vec<f32> {
        let mut seq = Sequence::new("clip", bpm);
        seq.program = Some(self.program);
        seq.notes = notes
            .iter()
            .map(|n| SeqNote {
                pitch: n.pitch,
                start_beats: n.beat,
                duration_beats: n.len,
                velocity: n.vel,
            })
            .collect();
        preview::render(&seq, sample_rate, 0.0)
    }
}

/// Plays a recorded sound at other pitches by pitch-shifting it.
pub struct Sampler {
    /// The sound, trimmed of silence at either end.
    pub sample: Vec<f32>,
    /// Its pitch (fractional MIDI note).
    pub root: f64,
}

/// How far a sampler shifts before folding by octaves (semitones).
pub const SAMPLER_RANGE: f64 = 12.0;

/// The shift (semitones) the sampler uses for `pitch`: the distance from
/// the root, folded by octaves into ±12 (plus half a semitone, since the
/// root is rarely exactly on a note) so the sound never turns into a
/// chipmunk or a growl.
pub fn sampler_shift(pitch: u8, root: f64) -> f64 {
    let mut d = f64::from(pitch) - root;
    while d > SAMPLER_RANGE + 0.5 {
        d -= 12.0;
    }
    while d < -SAMPLER_RANGE - 0.5 {
        d += 12.0;
    }
    d
}

impl Sampler {
    /// Trims `sound` to where it is audible (above −40 dB of its peak).
    pub fn new(sound: &[f32], root: f64) -> Self {
        let peak = sound.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let gate = peak * 0.01;
        let first = sound.iter().position(|s| s.abs() > gate).unwrap_or(0);
        let last = sound
            .iter()
            .rposition(|s| s.abs() > gate)
            .map_or(0, |i| i + 1);
        let sample = if first < last {
            sound[first..last].to_vec()
        } else {
            Vec::new()
        };
        Self { sample, root }
    }
}

impl InstrumentVoice for Sampler {
    fn describe(&self) -> String {
        format!(
            "your sound (recorded at {})",
            crate::theory::note_name(self.root.round().clamp(0.0, 127.0) as u8)
        )
    }

    fn render(&self, notes: &[Note], bpm: f64, sample_rate: u32) -> Vec<f32> {
        let spb = 60.0 / bpm;
        let sr = f64::from(sample_rate);
        let fade = (0.01 * sr) as usize;
        let mut out: Vec<f32> = Vec::new();
        if self.sample.is_empty() {
            return out;
        }
        // Notes that repeat (same pitch and length) are rendered once.
        let mut cache: HashMap<(u8, usize), Vec<f32>> = HashMap::new();
        for n in notes {
            let len = ((n.len * spb * sr).round() as usize).max(1);
            let shifted = cache.entry((n.pitch, len)).or_insert_with(|| {
                let shift = sampler_shift(n.pitch, self.root);
                let natural = self.sample.len();
                // Longer notes stretch the sound (up to 4×); shorter ones
                // cut it, keeping the attack, with a short fade.
                let ratio = if len > natural {
                    (len as f64 / natural as f64).min(4.0)
                } else {
                    1.0
                };
                let mut v =
                    crate::stretch::stretch_and_shift(&self.sample, sample_rate, ratio, shift);
                if v.len() > len {
                    v.truncate(len + fade);
                    let n = v.len();
                    for k in 0..fade.min(n) {
                        v[n - fade.min(n) + k] *= 1.0 - (k + 1) as f32 / fade.min(n) as f32;
                    }
                }
                v
            });
            let at = (n.beat * spb * sr).round().max(0.0) as usize;
            let g = f32::from(n.vel) / 127.0;
            if out.len() < at + shifted.len() {
                out.resize(at + shifted.len(), 0.0);
            }
            for (o, s) in out[at..].iter_mut().zip(shifted.iter()) {
                *o += s * g;
            }
        }
        out
    }
}

/// The procedural drum kit; note pitches follow General MIDI drums.
pub struct DrumKit;

impl InstrumentVoice for DrumKit {
    fn describe(&self) -> String {
        "drum kit (built-in)".into()
    }

    fn render(&self, notes: &[Note], bpm: f64, sample_rate: u32) -> Vec<f32> {
        let spb = 60.0 / bpm;
        let sr = f64::from(sample_rate);
        let mut out: Vec<f32> = Vec::new();
        for (i, n) in notes.iter().enumerate() {
            let sound = Sound::from_gm_drum(n.pitch);
            let hit = drum_hit(sound, f32::from(n.vel) / 127.0, sample_rate, i as u64);
            let at = (n.beat * spb * sr).round().max(0.0) as usize;
            if out.len() < at + hit.len() {
                out.resize(at + hit.len(), 0.0);
            }
            for (o, s) in out[at..].iter_mut().zip(&hit) {
                *o += s;
            }
        }
        out
    }
}

/// The pitch of a recorded sound: the median of YIN estimates over its
/// loud frames, as a fractional MIDI note. `None` if it has no clear pitch
/// (fewer than three voiced frames).
pub fn root_pitch(samples: &[f32], sample_rate: u32) -> Option<f64> {
    let frame = 4_096.min(samples.len());
    let hop = 1_024;
    if frame < 2_048 {
        return None;
    }
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak <= 0.0 {
        return None;
    }
    let gate = crate::dsp::rms(samples) * 0.5;
    let mut found: Vec<f32> = Vec::new();
    let mut start = 0;
    while start + frame <= samples.len() {
        let w = &samples[start..start + frame];
        if crate::dsp::rms(w) >= gate {
            if let Some(hz) = yin_pitch(w, sample_rate, 50.0, 1_500.0, 0.15) {
                found.push(hz_to_midi(hz));
            }
        }
        start += hop;
    }
    if found.len() < 3 {
        return None;
    }
    found.sort_by(f32::total_cmp);
    Some(f64::from(found[found.len() / 2]))
}

/// The voice for a notes clip. `sound` is the clip's own source region
/// (used by the sampler).
pub fn voice_for(voice: &Voice, sound: &[f32]) -> Box<dyn InstrumentVoice> {
    match voice {
        Voice::Instrument { program } => crate::soundfont::instrument(*program),
        Voice::Sampler { root } => Box::new(Sampler::new(sound, *root)),
        Voice::Drums => Box::new(DrumKit),
    }
}

/// Renders `notes` (beats from the clip start) with `voice`, mono at the
/// song's sample rate.
pub fn render_notes(
    song: &Song,
    clip: &Clip,
    source: &AudioBuffer,
    notes: &[Note],
    voice: &Voice,
) -> Vec<f32> {
    let sound = match voice {
        Voice::Sampler { .. } => {
            crate::project::render::region(source, clip.source_start, clip.source_len)
        }
        _ => Vec::new(),
    };
    voice_for(voice, &sound).render(notes, song.bpm, song.sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{HumParams, Tone, hum};

    const SR: u32 = 48_000;

    fn note(pitch: u8, beat: f64, len: f64) -> Note {
        Note {
            pitch,
            beat,
            len,
            vel: 100,
        }
    }

    #[test]
    fn root_pitch_of_a_hummed_note_is_within_25_cents() {
        let mut worst: f64 = 0.0;
        for (i, midi) in [43.0f32, 50.3, 57.0, 62.8, 69.0, 74.5]
            .into_iter()
            .enumerate()
        {
            let x = hum(&[Tone::new(midi, 0.1, 1.0)], &HumParams::new(SR), i as u64);
            let got = root_pitch(&x, SR).expect("pitch");
            let cents = (got - f64::from(midi)) * 100.0;
            worst = worst.max(cents.abs());
            assert!(cents.abs() <= 25.0, "{midi}: {got}");
        }
        eprintln!("root pitch: worst error {worst:.1} cents");
        assert_eq!(root_pitch(&[0.0; 10_000], SR), None);
        let noise = crate::synthetic::white_noise(20_000, 0.3, 1);
        assert_eq!(root_pitch(&noise, SR), None);
    }

    #[test]
    fn sampler_plays_its_sound_at_other_pitches() {
        let p = HumParams {
            vibrato_cents: 0.0,
            drift_cents: 0.0,
            ..HumParams::new(SR)
        };
        let x = hum(&[Tone::new(57.0, 0.05, 0.8)], &p, 3);
        let root = root_pitch(&x, SR).unwrap();
        let s = Sampler::new(&x, root);
        let mut worst: f64 = 0.0;
        for target in [45u8, 50, 57, 64, 69] {
            let y = s.render(&[note(target, 0.0, 1.0)], 60.0, SR);
            assert!(y.iter().all(|v| v.is_finite()));
            let got = root_pitch(&y, SR).unwrap();
            let err = (got - root) - (f64::from(target) - root);
            worst = worst.max(err.abs() * 100.0);
            assert!(err.abs() < 0.25, "{target}: got {got}, root {root}");
        }
        eprintln!("sampler: worst pitch error {worst:.1} cents over ±12 semitones");
        // Beyond an octave folds back.
        assert_eq!(sampler_shift(57 + 19, 57.0), 7.0);
        assert_eq!(sampler_shift(57 - 14, 57.0), -2.0);
    }

    #[test]
    fn sampler_note_lengths() {
        let x: Vec<f32> = (0..24_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 220.0 * i as f32 / SR as f32).sin())
            .collect();
        let s = Sampler::new(&x, 57.0);
        // 0.25 s note: cut (plus 10 ms fade); 1 s note: stretched.
        let short = s.render(&[note(57, 0.0, 0.25)], 60.0, SR);
        assert_eq!(short.len(), 12_000 + 480);
        let long = s.render(&[note(60, 0.0, 1.0)], 60.0, SR);
        assert_eq!(long.len(), 48_000);
        // Two notes in a row land where they should.
        let two = s.render(&[note(57, 0.0, 0.25), note(57, 1.0, 0.25)], 60.0, SR);
        assert!(two[48_000 + 100].abs() > 0.0);
        assert_eq!(two[30_000], 0.0);
    }

    #[test]
    fn drum_kit_places_hits() {
        let notes = [
            note(36, 1.0, 0.25),
            note(38, 2.0, 0.25),
            note(42, 2.5, 0.25),
        ];
        let y = DrumKit.render(&notes, 120.0, SR);
        let hits = crate::onset::detect_onsets(&y, SR);
        assert_eq!(hits.len(), 3, "{hits:?}");
        for (h, t) in hits.iter().zip([0.5, 1.0, 1.25]) {
            assert!((h.time - t).abs() < 0.003, "{} vs {t}", h.time);
        }
    }

    #[test]
    fn preview_synth_is_the_old_render() {
        let notes = [note(60, 0.0, 1.0), note(64, 1.0, 1.0)];
        let mut seq = Sequence::new("clip", 100.0);
        seq.program = Some(40);
        seq.notes = notes
            .iter()
            .map(|n| SeqNote {
                pitch: n.pitch,
                start_beats: n.beat,
                duration_beats: n.len,
                velocity: n.vel,
            })
            .collect();
        assert_eq!(
            PreviewSynth { program: 40 }.render(&notes, 100.0, SR),
            preview::render(&seq, SR, 0.0)
        );
    }
}
