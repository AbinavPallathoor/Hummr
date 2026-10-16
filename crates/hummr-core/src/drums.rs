//! Telling kicks, snares and hats apart, for turning a beatbox take into
//! drums.
//!
//! Rule-based on where a hit's energy sits in its first 60 ms: a kick
//! ("b", a thump) is mostly below 250 Hz with little above it (a snare's
//! body can be low too, but its noise lifts the average frequency), a hat
//! ("ts") is mostly above 5 kHz, and everything in between (a "pf", "k" or
//! clap) counts as a snare.

use realfft::RealFftPlanner;

/// The drum a hit sounds most like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumClass {
    Kick,
    Snare,
    Hat,
}

impl DrumClass {
    /// The General MIDI drum note it becomes.
    pub fn gm_note(self) -> u8 {
        match self {
            DrumClass::Kick => 36,
            DrumClass::Snare => 38,
            DrumClass::Hat => 42,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DrumClass::Kick => "kick",
            DrumClass::Snare => "snare",
            DrumClass::Hat => "hat",
        }
    }
}

/// Where a hit's energy is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitFeatures {
    /// Share of energy below 250 Hz.
    pub low: f32,
    /// Share of energy above 5 kHz.
    pub high: f32,
    /// Spectral centroid (Hz).
    pub centroid: f32,
}

/// Measures the first 60 ms of `hit` (which should start at the attack).
pub fn features(hit: &[f32], sample_rate: u32) -> HitFeatures {
    let n = 4_096.max((sample_rate as usize * 60 / 1000).next_power_of_two());
    let take = (sample_rate as usize * 60 / 1000).min(hit.len());
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    // Keep the attack, taper the end (half a Hann window).
    for (i, x) in input.iter_mut().enumerate().take(take) {
        let w = 0.5 + 0.5 * (std::f32::consts::PI * i as f32 / take as f32).cos();
        *x = hit[i] * w;
    }
    let mut spec = fft.make_output_vec();
    fft.process(&mut input, &mut spec).expect("fft sizes match");
    let bin_hz = sample_rate as f32 / n as f32;
    let (mut total, mut low, mut high, mut weighted) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (k, c) in spec.iter().enumerate() {
        let hz = k as f32 * bin_hz;
        if hz < 30.0 {
            continue;
        }
        let p = c.norm_sqr();
        total += p;
        weighted += p * hz;
        if hz < 250.0 {
            low += p;
        } else if hz > 5_000.0 {
            high += p;
        }
    }
    if total <= 0.0 {
        return HitFeatures {
            low: 0.0,
            high: 0.0,
            centroid: 0.0,
        };
    }
    HitFeatures {
        low: low / total,
        high: high / total,
        centroid: weighted / total,
    }
}

/// Classifies one hit (audio starting at its attack).
pub fn classify(hit: &[f32], sample_rate: u32) -> DrumClass {
    let f = features(hit, sample_rate);
    if f.low > 0.4 && f.centroid < 800.0 {
        DrumClass::Kick
    } else if f.high > 0.5 || f.centroid > 6_000.0 {
        DrumClass::Hat
    } else {
        DrumClass::Snare
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Sound, drum_hit};
    use crate::synthetic::{Drum, beatbox_hit};

    const SR: u32 = 48_000;

    fn class(d: Drum) -> DrumClass {
        match d {
            Drum::Kick => DrumClass::Kick,
            Drum::Snare => DrumClass::Snare,
            Drum::Hat => DrumClass::Hat,
        }
    }

    #[test]
    fn beatboxed_hits_are_told_apart() {
        let mut right = 0;
        let mut total = 0;
        let mut confusion = [[0usize; 3]; 3];
        for (row, d) in [Drum::Kick, Drum::Snare, Drum::Hat].into_iter().enumerate() {
            for seed in 0..100 {
                let hit = beatbox_hit(d, seed, SR);
                let got = classify(&hit, SR);
                let col = got as usize;
                confusion[row][col] += 1;
                total += 1;
                if got == class(d) {
                    right += 1;
                }
            }
        }
        let acc = right as f64 / total as f64;
        eprintln!(
            "beatbox: {:.1}% right, confusion (rows kick/snare/hat) {confusion:?}",
            acc * 100.0
        );
        assert!(acc >= 0.9, "{acc}");
    }

    #[test]
    fn kit_drums_are_told_apart() {
        let cases = [
            (Sound::Kick, DrumClass::Kick),
            (Sound::Snare, DrumClass::Snare),
            (Sound::Clap, DrumClass::Snare),
            (Sound::HatClosed, DrumClass::Hat),
            (Sound::HatOpen, DrumClass::Hat),
        ];
        let mut right = 0;
        let mut total = 0;
        for (sound, want) in cases {
            for seed in 0..20 {
                let vel = 0.3 + 0.035 * seed as f32;
                total += 1;
                if classify(&drum_hit(sound, vel, SR, seed), SR) == want {
                    right += 1;
                }
            }
        }
        let acc = right as f64 / total as f64;
        eprintln!("kit: {:.1}% right", acc * 100.0);
        assert!(acc >= 0.9, "{acc}");
    }

    #[test]
    fn silence_is_a_snare_not_a_crash() {
        let f = features(&[0.0; 100], SR);
        assert_eq!(f.low, 0.0);
        let _ = classify(&[], SR);
    }
}
