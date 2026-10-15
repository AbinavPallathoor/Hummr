//! A small additive "preview" synthesiser plus mixing helpers.
//!
//! This lets the editor play notes back before SoundFont rendering exists.
//! Each General MIDI family gets a rough harmonic recipe and envelope, enough
//! to tell a plucked guitar from a bowed violin.

use std::f32::consts::{FRAC_PI_4, TAU};

use crate::sequence::Sequence;
use crate::theory::midi_to_hz;

#[derive(Clone, Copy, Debug)]
struct Timbre {
    harmonics: [f32; 8],
    attack: f32,
    /// Exponential decay rate while held (per second); 0 means sustained.
    decay: f32,
    release: f32,
    vibrato_cents: f32,
}

fn timbre(program: u8) -> Timbre {
    let sustained = |h: [f32; 8], attack: f32, vib: f32| Timbre {
        harmonics: h,
        attack,
        decay: 0.0,
        release: 0.08,
        vibrato_cents: vib,
    };
    let plucked = |h: [f32; 8], decay: f32| Timbre {
        harmonics: h,
        attack: 0.003,
        decay,
        release: 0.12,
        vibrato_cents: 0.0,
    };
    match program / 8 {
        0 | 1 => plucked([1.0, 0.5, 0.3, 0.2, 0.12, 0.08, 0.05, 0.03], 2.5), // keys, chromatic perc.
        2 => sustained([1.0, 0.7, 0.5, 0.4, 0.3, 0.2, 0.15, 0.1], 0.01, 0.0), // organ
        3 => plucked([1.0, 0.6, 0.35, 0.2, 0.12, 0.06, 0.03, 0.02], 1.8),    // guitar
        4 => plucked([1.0, 0.4, 0.15, 0.08, 0.04, 0.02, 0.0, 0.0], 1.2),     // bass
        5 | 6 => sustained([1.0, 0.5, 0.33, 0.25, 0.2, 0.16, 0.14, 0.12], 0.06, 12.0), // strings
        7 => sustained([1.0, 0.8, 0.6, 0.45, 0.35, 0.25, 0.18, 0.12], 0.03, 6.0), // brass
        8 => sustained([1.0, 0.1, 0.55, 0.08, 0.35, 0.05, 0.2, 0.03], 0.03, 5.0), // reed
        9 => sustained([1.0, 0.25, 0.08, 0.03, 0.0, 0.0, 0.0, 0.0], 0.04, 8.0), // pipe/flute
        10 => sustained([1.0, 0.5, 0.33, 0.25, 0.2, 0.16, 0.14, 0.12], 0.005, 0.0), // synth lead
        _ => sustained([1.0, 0.4, 0.2, 0.1, 0.05, 0.03, 0.0, 0.0], 0.15, 3.0),
    }
}

/// Renders a sequence to mono at `sample_rate`, starting `offset_seconds` in.
pub fn render(seq: &Sequence, sample_rate: u32, offset_seconds: f64) -> Vec<f32> {
    let sr = sample_rate as f32;
    let t = timbre(seq.program.unwrap_or(0));
    let total = offset_seconds + seq.beats_to_seconds(seq.end_beats()) + f64::from(t.release) + 0.1;
    let mut out = vec![0.0f32; (total * f64::from(sample_rate)).ceil() as usize];
    let norm: f32 = t.harmonics.iter().sum();

    for note in &seq.notes {
        let start = ((offset_seconds + seq.beats_to_seconds(note.start_beats))
            * f64::from(sample_rate)) as usize;
        let held = (seq.beats_to_seconds(note.duration_beats) as f32 * sr) as usize;
        let rel = (t.release * sr) as usize;
        let f = midi_to_hz(f32::from(note.pitch));
        let amp = 0.3 * (f32::from(note.velocity) / 127.0).powf(1.5) / norm;
        let mut phase = 0.0f32;
        let mut level_at_release = 0.0f32;
        for i in 0..held + rel {
            let idx = start + i;
            if idx >= out.len() {
                break;
            }
            let time = i as f32 / sr;
            let env = if i < held {
                let a = (time / t.attack.max(1e-4)).min(1.0);
                let d = if t.decay > 0.0 {
                    (-t.decay * time).exp()
                } else {
                    1.0
                };
                level_at_release = a * d;
                level_at_release
            } else {
                level_at_release * (1.0 - (i - held) as f32 / rel.max(1) as f32)
            };
            // Vibrato fades in after 0.3 s, like a player would.
            let vib = t.vibrato_cents / 1200.0 * (time / 0.3).min(1.0) * (TAU * 5.5 * time).sin();
            phase = (phase + f * 2f32.powf(vib) / sr).fract();
            let mut s = 0.0;
            for (k, h) in t.harmonics.iter().enumerate() {
                if *h == 0.0 || f * (k + 1) as f32 >= sr / 2.0 {
                    continue;
                }
                s += h * (TAU * phase * (k + 1) as f32).sin();
            }
            out[idx] += amp * env * s;
        }
    }
    out
}

/// Linear-interpolation resampler, adequate for previews.
pub fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    // Integer arithmetic so e.g. 24000 samples at 48 kHz is exactly 22050 at 44.1 kHz.
    let n = (input.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = x.floor() as usize;
            let frac = (x - j as f64) as f32;
            let a = input[j];
            let b = *input.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

/// Constant-power pan law: `pan` in -1 (left) ..= 1 (right).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// One mono source in a stereo mix.
#[derive(Clone, Debug, PartialEq)]
pub struct MixSource {
    pub samples: Vec<f32>,
    pub gain: f32,
    pub pan: f32,
    pub muted: bool,
}

/// Mixes sources to interleaved stereo, soft-clipping the result.
pub fn mix_stereo(sources: &[MixSource]) -> Vec<f32> {
    let len = sources.iter().map(|s| s.samples.len()).max().unwrap_or(0);
    let mut out = vec![0.0f32; len * 2];
    for s in sources.iter().filter(|s| !s.muted) {
        let (l, r) = pan_gains(s.pan);
        for (i, &x) in s.samples.iter().enumerate() {
            out[2 * i] += x * s.gain * l;
            out[2 * i + 1] += x * s.gain * r;
        }
    }
    for x in &mut out {
        *x = soft_clip(*x);
    }
    out
}

#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.9 {
        x
    } else {
        x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) / 0.1).tanh())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::yin::yin_pitch;
    use crate::sequence::SeqNote;
    use crate::theory::hz_to_midi;

    #[test]
    fn rendered_notes_have_the_right_pitch() {
        for program in [0u8, 24, 40, 56, 73] {
            let mut seq = Sequence::new("t", 120.0);
            seq.program = Some(program);
            seq.notes.push(SeqNote {
                pitch: 69,
                start_beats: 0.0,
                duration_beats: 2.0,
                velocity: 100,
            });
            let audio = render(&seq, 44_100, 0.0);
            let frame = &audio[4_410..4_410 + 4_096];
            let hz = yin_pitch(frame, 44_100, 60.0, 1500.0, 0.2).expect("pitched");
            assert!(
                (hz_to_midi(hz) - 69.0).abs() < 0.3,
                "program {program}: {hz} Hz"
            );
            assert!(audio.iter().all(|s| s.abs() <= 1.0));
        }
    }

    #[test]
    fn offset_delays_the_first_note() {
        let mut seq = Sequence::new("t", 120.0);
        seq.notes.push(SeqNote {
            pitch: 60,
            start_beats: 0.0,
            duration_beats: 1.0,
            velocity: 100,
        });
        let a = render(&seq, 8_000, 0.5);
        assert!(a[..3_900].iter().all(|s| *s == 0.0));
        assert!(a[4_100..4_400].iter().any(|s| s.abs() > 0.01));
    }

    #[test]
    fn resampling_preserves_duration_and_pitch() {
        let x = crate::synthetic::sine(440.0, 48_000, 0.5, 0.5);
        let y = resample_linear(&x, 48_000, 44_100);
        assert_eq!(y.len(), 22_050);
        let hz = yin_pitch(&y[1_000..5_096], 44_100, 60.0, 1500.0, 0.2).unwrap();
        assert!((hz - 440.0).abs() < 2.0);
    }

    #[test]
    fn pan_and_mix() {
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-6 && (l * l + r * r - 1.0).abs() < 1e-5);
        assert!(pan_gains(-1.0).1.abs() < 1e-6);
        let mix = mix_stereo(&[
            MixSource {
                samples: vec![0.5; 4],
                gain: 1.0,
                pan: -1.0,
                muted: false,
            },
            MixSource {
                samples: vec![0.5; 2],
                gain: 1.0,
                pan: 1.0,
                muted: true,
            },
        ]);
        assert_eq!(mix.len(), 8);
        assert!((mix[0] - 0.5).abs() < 1e-6 && mix[1].abs() < 1e-6);
    }
}
