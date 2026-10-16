//! Procedural starter sounds: drums (kick, snare, clap, closed and open
//! hat), an 808 bass, a plucked string and a soft pad. Everything is
//! synthesised from scratch with seeded noise, so there are no third-party
//! samples and every render is identical.

use std::f32::consts::TAU;

use crate::dsp::Biquad;
use crate::synthetic::Rng;
use crate::theory::midi_to_hz;

/// A built-in sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    Kick,
    Snare,
    Clap,
    HatClosed,
    HatOpen,
    Bass808,
    Pluck,
    Pad,
}

impl Sound {
    pub const ALL: [Sound; 8] = [
        Sound::Kick,
        Sound::Snare,
        Sound::Clap,
        Sound::HatClosed,
        Sound::HatOpen,
        Sound::Bass808,
        Sound::Pluck,
        Sound::Pad,
    ];

    /// File-friendly name, also what the import picker shows.
    pub fn name(self) -> &'static str {
        match self {
            Sound::Kick => "kick",
            Sound::Snare => "snare",
            Sound::Clap => "clap",
            Sound::HatClosed => "closed hat",
            Sound::HatOpen => "open hat",
            Sound::Bass808 => "808",
            Sound::Pluck => "pluck",
            Sound::Pad => "pad",
        }
    }

    /// Whether the sound has a musical pitch (and so can play notes).
    pub fn is_pitched(self) -> bool {
        matches!(self, Sound::Bass808 | Sound::Pluck | Sound::Pad)
    }

    /// The General MIDI drum note for drum sounds.
    pub fn gm_drum(self) -> Option<u8> {
        match self {
            Sound::Kick => Some(36),
            Sound::Snare => Some(38),
            Sound::Clap => Some(39),
            Sound::HatClosed => Some(42),
            Sound::HatOpen => Some(46),
            _ => None,
        }
    }

    /// The drum sound for a General MIDI drum note (nearest family).
    pub fn from_gm_drum(note: u8) -> Sound {
        match note {
            35 | 36 | 41 | 43 | 45 => Sound::Kick,
            39 => Sound::Clap,
            42 | 44 => Sound::HatClosed,
            46 | 49 | 51 | 52 | 55 | 57 | 59 => Sound::HatOpen,
            _ if note >= 47 => Sound::HatClosed,
            _ => Sound::Snare,
        }
    }

    /// Renders the sound once at full velocity: drums at their natural
    /// length, pitched sounds as a one-second C3/C4 note.
    pub fn render(self, sample_rate: u32) -> Vec<f32> {
        match self {
            Sound::Bass808 => render_note(self, 36.0, 1.0, sample_rate),
            Sound::Pluck => render_note(self, 60.0, 1.0, sample_rate),
            Sound::Pad => render_note(self, 60.0, 2.0, sample_rate),
            drum => drum_hit(drum, 1.0, sample_rate, 1),
        }
    }
}

fn exp_env(t: f32, tau: f32) -> f32 {
    (-t / tau).exp()
}

/// Fades the last 15 ms so a sound never ends with a click.
fn tail_fade(mut x: Vec<f32>, sample_rate: u32) -> Vec<f32> {
    let n = ((sample_rate as f32 * 0.015) as usize).min(x.len());
    let len = x.len();
    for k in 0..n {
        x[len - n + k] *= 1.0 - (k + 1) as f32 / n as f32;
    }
    x
}

/// One drum hit at `velocity` (0..=1). `seed` varies the noise so repeated
/// hits are not bit-identical.
pub fn drum_hit(sound: Sound, velocity: f32, sample_rate: u32, seed: u64) -> Vec<f32> {
    tail_fade(raw_hit(sound, velocity, sample_rate, seed), sample_rate)
}

fn raw_hit(sound: Sound, velocity: f32, sample_rate: u32, seed: u64) -> Vec<f32> {
    let sr = sample_rate as f32;
    let mut rng = Rng::new(seed.wrapping_add(sound as u64 * 7919));
    let v = velocity.clamp(0.0, 1.0);
    match sound {
        Sound::Kick => {
            let n = (0.45 * sr) as usize;
            let mut phase = 0.0f32;
            (0..n)
                .map(|i| {
                    let t = i as f32 / sr;
                    let f = 48.0 + 110.0 * exp_env(t, 0.035);
                    phase = (phase + f / sr).fract();
                    let body = (TAU * phase).sin() * exp_env(t, 0.16);
                    let click = if t < 0.003 {
                        rng.bipolar() * 0.4 * (1.0 - t / 0.003)
                    } else {
                        0.0
                    };
                    0.7 * v * (body + click)
                })
                .collect()
        }
        Sound::Snare => {
            let n = (0.28 * sr) as usize;
            let mut bp = Biquad::bandpass(sr, 1_800.0, 0.8);
            let mut phase = 0.0f32;
            (0..n)
                .map(|i| {
                    let t = i as f32 / sr;
                    let noise = bp.process(rng.bipolar()) * 1.6 * exp_env(t, 0.07);
                    phase = (phase + 190.0 / sr).fract();
                    let body = (TAU * phase).sin() * 0.5 * exp_env(t, 0.05);
                    0.8 * v * (noise + body)
                })
                .collect()
        }
        Sound::Clap => {
            let n = (0.3 * sr) as usize;
            let mut bp = Biquad::bandpass(sr, 1_200.0, 1.2);
            (0..n)
                .map(|i| {
                    let t = i as f32 / sr;
                    // Three quick bursts, then a short tail.
                    let env = [0.0f32, 0.011, 0.022]
                        .iter()
                        .map(|&s| if t >= s { exp_env(t - s, 0.006) } else { 0.0 })
                        .fold(0.0, f32::max)
                        .max(if t > 0.03 {
                            0.5 * exp_env(t - 0.03, 0.08)
                        } else {
                            0.0
                        });
                    0.9 * v * bp.process(rng.bipolar()) * 2.0 * env
                })
                .collect()
        }
        Sound::HatClosed | Sound::HatOpen => {
            let (len, tau) = if sound == Sound::HatClosed {
                (0.09, 0.018)
            } else {
                (0.5, 0.15)
            };
            let n = (len * sr) as usize;
            let mut hp = Biquad::highpass(sr, 7_500.0, 0.7);
            let mut hp2 = Biquad::highpass(sr, 7_500.0, 0.7);
            (0..n)
                .map(|i| {
                    let t = i as f32 / sr;
                    0.6 * v * hp2.process(hp.process(rng.bipolar())) * exp_env(t, tau)
                })
                .collect()
        }
        pitched => pitched.render(sample_rate),
    }
}

/// An 808-style sub bass: a sine with a quick downward pitch blip and a
/// long decay, gently saturated.
pub fn bass808(midi: f32, seconds: f32, sample_rate: u32) -> Vec<f32> {
    let sr = sample_rate as f32;
    let f0 = midi_to_hz(midi);
    let n = ((seconds + 0.6) * sr) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let f = f0 * (1.0 + 0.5 * exp_env(t, 0.02));
            phase = (phase + f / sr).fract();
            let hold = if t < seconds {
                1.0
            } else {
                exp_env(t - seconds, 0.08)
            };
            let x = (TAU * phase).sin() * exp_env(t, 1.2) * hold;
            0.8 * (1.6 * x).tanh()
        })
        .collect()
}

/// A Karplus–Strong plucked string.
pub fn pluck(midi: f32, seconds: f32, sample_rate: u32, seed: u64) -> Vec<f32> {
    let sr = sample_rate as f32;
    let period = (sr / midi_to_hz(midi)).max(2.0);
    let len = period.floor() as usize;
    let frac = period - len as f32;
    let mut rng = Rng::new(seed);
    let mut line: Vec<f32> = (0..len.max(2)).map(|_| rng.bipolar() * 0.5).collect();
    let n = ((seconds + 0.3) * sr) as usize;
    let mut out = Vec::with_capacity(n);
    let mut idx = 0usize;
    let mut prev = 0.0f32;
    for i in 0..n {
        let a = line[idx];
        let b = line[(idx + 1) % line.len()];
        // Fractional delay by linear interpolation keeps the tuning right.
        let y = a + (b - a) * frac;
        let damped = 0.996 * 0.5 * (y + prev);
        prev = y;
        line[idx] = damped;
        idx = (idx + 1) % line.len();
        let t = i as f32 / sr;
        let release = if t < seconds {
            1.0
        } else {
            exp_env(t - seconds, 0.05)
        };
        out.push(a * release);
    }
    out
}

/// A soft pad: three detuned saws through a low-pass, slow attack.
pub fn pad(midi: f32, seconds: f32, sample_rate: u32, seed: u64) -> Vec<f32> {
    let sr = sample_rate as f32;
    let f0 = midi_to_hz(midi);
    let mut rng = Rng::new(seed);
    let mut phases: Vec<f32> = (0..3).map(|_| rng.bipolar().abs()).collect();
    let detune = [-0.08f32, 0.0, 0.07];
    let mut lp = Biquad::lowpass(sr, 1_400.0, 0.7);
    let release = 0.5;
    let n = ((seconds + release) * sr) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let mut s = 0.0;
            for (p, d) in phases.iter_mut().zip(detune) {
                *p = (*p + f0 * 2f32.powf(d / 12.0) / sr).fract();
                s += 2.0 * *p - 1.0;
            }
            let attack = (t / 0.35).min(1.0);
            let rel = if t < seconds {
                1.0
            } else {
                1.0 - ((t - seconds) / release).min(1.0)
            };
            0.18 * lp.process(s) * attack * rel
        })
        .collect()
}

/// Renders a pitched sound at `midi` for `seconds` (drums ignore pitch).
pub fn render_note(sound: Sound, midi: f32, seconds: f32, sample_rate: u32) -> Vec<f32> {
    let x = match sound {
        Sound::Bass808 => bass808(midi, seconds, sample_rate),
        Sound::Pluck => pluck(midi, seconds, sample_rate, midi as u64),
        Sound::Pad => pad(midi, seconds, sample_rate, 11),
        drum => return drum_hit(drum, 1.0, sample_rate, midi as u64),
    };
    tail_fade(x, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::yin::yin_pitch;
    use crate::theory::hz_to_midi;

    #[test]
    fn every_sound_is_finite_bounded_and_not_silent() {
        for s in Sound::ALL {
            let x = s.render(48_000);
            assert!(!x.is_empty(), "{s:?}");
            assert!(x.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{s:?}");
            assert!(x.iter().fold(0.0f32, |m, v| m.max(v.abs())) > 0.1, "{s:?}");
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(Sound::Snare.render(48_000), Sound::Snare.render(48_000));
    }

    #[test]
    fn pitched_sounds_are_in_tune() {
        for (s, midi) in [
            (Sound::Pluck, 57.0),
            (Sound::Bass808, 45.0),
            (Sound::Pad, 60.0),
        ] {
            let x = render_note(s, midi, 1.0, 48_000);
            let frame = &x[12_000..12_000 + 8_192];
            let hz = yin_pitch(frame, 48_000, 30.0, 1_000.0, 0.3).expect("pitched");
            assert!(
                (hz_to_midi(hz) - midi).abs() < 0.3,
                "{s:?}: {hz} Hz for midi {midi}"
            );
        }
    }

    #[test]
    fn gm_drum_mapping_round_trips() {
        for s in [
            Sound::Kick,
            Sound::Snare,
            Sound::Clap,
            Sound::HatClosed,
            Sound::HatOpen,
        ] {
            assert_eq!(Sound::from_gm_drum(s.gm_drum().unwrap()), s);
        }
    }
}
