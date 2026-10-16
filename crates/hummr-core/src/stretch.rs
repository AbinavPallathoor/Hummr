//! Time-stretching (WSOLA) and pitch-shifting.
//!
//! WSOLA, "waveform-similarity overlap-add" (Verhelst & Roelands, 1993):
//! the output is built from ~40 ms Hann-windowed pieces of the input laid
//! down every 20 ms. To make the output longer, the read position advances
//! more slowly than the write position (shorter: faster). Each piece is
//! taken from near where it "should" come from, shifted by up to ±12 ms to
//! the spot whose waveform best continues the previous piece, so the
//! overlaps add up in phase and the pitch is kept.
//!
//! Pitch-shifting stretches by the pitch ratio and then resamples back to
//! the original length with the windowed-sinc resampler, which scales every
//! frequency by the ratio.

use crate::resample::resample_positions;

/// Analysis frame length (~42 ms).
fn frame_len(sample_rate: u32) -> usize {
    let n = (sample_rate as usize / 24).max(64);
    n + n % 2
}

#[inline]
fn at(x: &[f32], i: i64) -> f32 {
    if i >= 0 {
        x.get(i as usize).copied().unwrap_or(0.0)
    } else {
        0.0
    }
}

/// Cross-correlation of `x[a..]` with `x[b..]` over `n` samples, every
/// `step`-th sample.
fn corr(x: &[f32], a: i64, b: i64, n: usize, step: usize) -> f64 {
    let mut s = 0.0f64;
    let mut j = 0;
    while j < n {
        s += f64::from(at(x, a + j as i64)) * f64::from(at(x, b + j as i64));
        j += step;
    }
    s
}

/// Makes `input` `ratio` times as long (0.25..=4) without changing its
/// pitch. The output has exactly `round(len * ratio)` samples.
pub fn time_stretch(input: &[f32], sample_rate: u32, ratio: f64) -> Vec<f32> {
    let ratio = ratio.clamp(0.25, 4.0);
    let out_len = (input.len() as f64 * ratio).round() as usize;
    if (ratio - 1.0).abs() < 1e-9 || input.is_empty() {
        let mut v = input.to_vec();
        v.resize(out_len, 0.0);
        return v;
    }
    let n = frame_len(sample_rate);
    let hs = n / 2;
    let ha = hs as f64 / ratio;
    let tol = (sample_rate as f64 * 0.012) as i64;
    // Periodic Hann: overlapping by half, the windows sum to exactly 1.
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect();
    let half = (n / 2) as i64;
    let mut out = vec![0.0f32; out_len + n];
    let mut prev: Option<i64> = None;
    let mut k = 0usize;
    loop {
        let out_start = (k * hs) as i64 - half;
        if out_start >= out_len as i64 {
            break;
        }
        let nominal = (k as f64 * ha).round() as i64 - half;
        let pos = match prev {
            None => nominal,
            Some(p) => {
                // Where the previous piece naturally continues.
                let target = p + hs as i64;
                // Coarse search on a decimated grid, then refine.
                let mut best = (f64::NEG_INFINITY, 0i64);
                let mut d = -tol;
                while d <= tol {
                    let c = corr(input, nominal + d, target, n, 4);
                    if c > best.0 {
                        best = (c, d);
                    }
                    d += 8;
                }
                let centre = best.1;
                let mut best = (f64::NEG_INFINITY, centre);
                for d in (centre - 8).max(-tol)..=(centre + 8).min(tol) {
                    let c = corr(input, nominal + d, target, n, 1);
                    if c > best.0 {
                        best = (c, d);
                    }
                }
                nominal + best.1
            }
        };
        for (j, w) in window.iter().enumerate() {
            let o = out_start + j as i64;
            if o >= 0 {
                if let Some(slot) = out.get_mut(o as usize) {
                    *slot += w * at(input, pos + j as i64);
                }
            }
        }
        prev = Some(pos);
        k += 1;
    }
    out.truncate(out_len);
    out
}

/// Shifts pitch by `semitones` (−24..=24) keeping the length.
pub fn pitch_shift(input: &[f32], sample_rate: u32, semitones: f64) -> Vec<f32> {
    stretch_and_shift(input, sample_rate, 1.0, semitones)
}

/// Makes `input` `ratio` times as long and shifts it by `semitones`, in one
/// pass (stretch by `ratio × pitch ratio`, then resample).
pub fn stretch_and_shift(input: &[f32], sample_rate: u32, ratio: f64, semitones: f64) -> Vec<f32> {
    let semitones = semitones.clamp(-24.0, 24.0);
    let out_len = (input.len() as f64 * ratio.clamp(0.25, 4.0)).round() as usize;
    if semitones.abs() < 1e-9 {
        return time_stretch(input, sample_rate, ratio);
    }
    let p = 2f64.powf(semitones / 12.0);
    let stretched = time_stretch(input, sample_rate, ratio * p);
    // Reading the stretched audio p times faster scales pitch by p.
    let mut out = resample_positions(&stretched, out_len, |i| i as f64 * p, (1.0 / p).min(1.0));
    out.resize(out_len, 0.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::yin::yin_pitch;
    use crate::synthetic::{HumParams, Tone, hum};

    const SR: u32 = 48_000;

    /// A steady sung-like tone (harmonics, a little breath noise, no
    /// vibrato) at `midi`.
    fn tone(midi: f32, seconds: f64) -> Vec<f32> {
        let p = HumParams {
            vibrato_cents: 0.0,
            drift_cents: 0.0,
            tail_seconds: 0.0,
            ..HumParams::new(SR)
        };
        hum(&[Tone::new(midi, 0.0, seconds)], &p, 3)
    }

    /// Median pitch (in fractional MIDI) over the middle of a signal.
    fn median_midi(x: &[f32]) -> f32 {
        let n = 4_096;
        let mut v: Vec<f32> = (0..20)
            .filter_map(|i| {
                let start = x.len() / 5 + i * (x.len() * 3 / 5 - n) / 20;
                yin_pitch(&x[start..start + n], SR, 50.0, 1_500.0, 0.2)
            })
            .map(crate::theory::hz_to_midi)
            .collect();
        assert!(v.len() >= 10, "pitch not found");
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    }

    #[test]
    fn stretching_keeps_length_and_pitch() {
        let mut worst: f32 = 0.0;
        for midi in [45.0f32, 57.0, 69.0] {
            let x = tone(midi, 1.5);
            for ratio in [0.5, 0.75, 1.25, 1.5, 2.0] {
                let y = time_stretch(&x, SR, ratio);
                let target = x.len() as f64 * ratio;
                assert!(
                    (y.len() as f64 / target - 1.0).abs() < 0.01,
                    "length {} vs {target}",
                    y.len()
                );
                assert!(y.iter().all(|s| s.is_finite()));
                let cents = (median_midi(&y) - median_midi(&x)) * 100.0;
                worst = worst.max(cents.abs());
                assert!(
                    cents.abs() < 20.0,
                    "midi {midi} ratio {ratio}: {cents} cents"
                );
            }
        }
        eprintln!("stretch: worst pitch change {worst:.1} cents");
    }

    #[test]
    fn shifting_moves_pitch_by_the_right_amount() {
        let mut worst: f64 = 0.0;
        let x = tone(57.0, 1.5);
        let base = median_midi(&x);
        for semis in [-12.0, -7.0, -3.0, 2.0, 5.0, 12.0] {
            let y = pitch_shift(&x, SR, semis);
            assert_eq!(y.len(), x.len());
            assert!(y.iter().all(|s| s.is_finite()));
            let got = f64::from(median_midi(&y) - base);
            let err = (got - semis) * 100.0;
            worst = worst.max(err.abs());
            assert!(err.abs() < 15.0, "{semis} semitones: moved {got}");
        }
        eprintln!("shift: worst pitch error {worst:.1} cents");
    }

    #[test]
    fn stretch_and_shift_together() {
        let x = tone(60.0, 1.0);
        let y = stretch_and_shift(&x, SR, 1.5, 4.0);
        assert_eq!(y.len(), (x.len() as f64 * 1.5).round() as usize);
        let got = median_midi(&y) - median_midi(&x);
        assert!((got - 4.0).abs() < 0.15, "moved {got}");
    }

    #[test]
    fn level_is_kept() {
        let x = tone(57.0, 1.0);
        let y = time_stretch(&x, SR, 1.7);
        let rms = |v: &[f32]| crate::dsp::rms(&v[v.len() / 4..v.len() * 3 / 4]);
        let db = crate::dsp::amp_to_db(rms(&y) / rms(&x));
        assert!(db.abs() < 1.5, "level changed {db} dB");
    }

    #[test]
    fn edge_cases() {
        assert!(time_stretch(&[], SR, 2.0).is_empty());
        assert_eq!(time_stretch(&[0.5; 10], SR, 1.0), vec![0.5; 10]);
        assert_eq!(time_stretch(&[0.0; 1_000], SR, 2.0).len(), 2_000);
        assert!(
            pitch_shift(&[1.0; 5_000], SR, 7.0)
                .iter()
                .all(|s| s.is_finite())
        );
    }
}
