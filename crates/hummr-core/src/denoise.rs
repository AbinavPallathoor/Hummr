//! Noise cleanup by spectral gating.
//!
//! Works on *steady* background noise: hiss, fan or air-conditioning rush,
//! mains hum. It learns what the noise sounds like from a stretch of the
//! recording where nobody is making a sound (picked by hand, or the
//! quietest half second), then turns down every frequency band of every
//! ~43 ms piece that is not clearly louder than that noise. It cannot remove
//! voices, music, traffic or anything else that changes over time: those
//! are louder than the learned noise, so they are treated as the sound you
//! want to keep.
//!
//! The method is power spectral subtraction with over-subtraction and a
//! gain floor (Berouti, Schwartz & Makhoul, 1979), with gains smoothed
//! across neighbouring bands and over time to avoid "musical noise"
//! (short, random chirps left behind by the gate).

use crate::dsp::db_to_amp;
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// What to say about the cleanup in help text and the inspector.
pub const HELP: &str = "Clean noise turns down steady background hiss, fan noise and mains hum. \
It learns the noise from a quiet moment (no humming or singing). \
It cannot remove voices, music or sudden sounds.";

/// Length of the learned-noise window when picked automatically (seconds).
pub const AUTO_NOISE_SECONDS: f64 = 0.5;

/// FFT length: about 43 ms, a power of two.
fn frame_len(sample_rate: u32) -> usize {
    (sample_rate as usize * 43 / 1000)
        .next_power_of_two()
        .max(256)
}

/// The average power of steady noise in each frequency band.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseProfile {
    /// Mean power per FFT bin (`frame / 2 + 1` bins).
    pub power: Vec<f32>,
    pub frame: usize,
}

impl NoiseProfile {
    /// Overall level of the learned noise in dBFS (RMS).
    pub fn level_db(&self) -> f32 {
        // White noise of variance σ² through a Hann window has an
        // expected bin power of σ² · 3n/8; summing both halves of the
        // spectrum gives σ² · 3n²/8.
        let n = self.frame as f32;
        let total: f32 = self.power.iter().sum::<f32>() * 2.0;
        10.0 * (total / (n * n * 3.0 / 8.0)).max(1e-12).log10()
    }
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect()
}

/// Learns the noise from `noise` (a stretch with only the background).
pub fn learn(noise: &[f32], sample_rate: u32) -> NoiseProfile {
    let n = frame_len(sample_rate);
    let hop = n / 4;
    let w = hann(n);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut power = vec![0.0f32; n / 2 + 1];
    let mut frames = 0usize;
    let mut start = 0usize;
    loop {
        for (j, x) in input.iter_mut().enumerate() {
            *x = noise.get(start + j).copied().unwrap_or(0.0) * w[j];
        }
        fft.process(&mut input, &mut spec).expect("fft sizes match");
        for (p, c) in power.iter_mut().zip(&spec) {
            *p += c.norm_sqr();
        }
        frames += 1;
        start += hop;
        if start + n > noise.len() {
            break;
        }
    }
    for p in &mut power {
        *p /= frames as f32;
    }
    NoiseProfile { power, frame: n }
}

/// The quietest `seconds`-long stretch of `samples` (by RMS), as a sample
/// range. Steps by 50 ms. The whole input if it is shorter than that.
pub fn quietest(samples: &[f32], sample_rate: u32, seconds: f64) -> std::ops::Range<usize> {
    let len = (seconds * f64::from(sample_rate)) as usize;
    if samples.len() <= len || len == 0 {
        return 0..samples.len();
    }
    let step = (sample_rate as usize / 20).max(1);
    // Prefix sums of squares make every window O(1).
    let mut acc = Vec::with_capacity(samples.len() + 1);
    acc.push(0.0f64);
    for &s in samples {
        acc.push(acc.last().copied().unwrap_or(0.0) + f64::from(s) * f64::from(s));
    }
    let mut best = (f64::INFINITY, 0usize);
    let mut s = 0usize;
    while s + len <= samples.len() {
        let e = acc[s + len] - acc[s];
        if e < best.0 {
            best = (e, s);
        }
        s += step;
    }
    best.1..best.1 + len
}

/// Learns the noise from the quietest half second of `samples`.
pub fn learn_auto(samples: &[f32], sample_rate: u32) -> NoiseProfile {
    let r = quietest(samples, sample_rate, AUTO_NOISE_SECONDS);
    learn(&samples[r], sample_rate)
}

/// Turns down the learned noise in `samples`. `strength` 0 returns the
/// input unchanged; 1 removes as much as it can (up to 40 dB). The output
/// has the same length.
pub fn denoise(samples: &[f32], sample_rate: u32, noise: &NoiseProfile, strength: f64) -> Vec<f32> {
    let s = strength.clamp(0.0, 1.0) as f32;
    if s <= 0.0 || samples.is_empty() {
        return samples.to_vec();
    }
    let n = noise.frame;
    if n != frame_len(sample_rate) {
        // Learned at another rate: relearn would need the noise audio,
        // so use the closest band of the profile instead.
        let target = frame_len(sample_rate);
        let bins = target / 2 + 1;
        let power = (0..bins)
            .map(|k| {
                let src = (k * (noise.frame / 2) / (target / 2)).min(noise.power.len() - 1);
                noise.power[src] * (target as f32 / noise.frame as f32).powi(2)
            })
            .collect();
        let p = NoiseProfile {
            power,
            frame: target,
        };
        return denoise(samples, sample_rate, &p, strength);
    }
    let hop = n / 4;
    let bins = n / 2 + 1;
    // Over-subtraction grows with strength so loud noise peaks are caught;
    // the floor keeps a little noise so what's left sounds natural.
    let over = 1.0 + 3.0 * s;
    let floor = db_to_amp(-40.0 * s);
    let w = hann(n);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut gains = vec![1.0f32; bins];
    let mut smooth = vec![1.0f32; bins];
    let mut prev = vec![floor; bins];

    // Pad so every input sample is covered by four full frames.
    let pad = n;
    let total = samples.len() + 2 * pad;
    let mut out = vec![0.0f32; total];
    let mut norm = vec![0.0f32; total];
    let mut start = 0usize;
    while start + n <= total {
        for (j, x) in input.iter_mut().enumerate() {
            let i = start + j;
            let v = if i >= pad && i - pad < samples.len() {
                samples[i - pad]
            } else {
                0.0
            };
            *x = v * w[j];
        }
        fft.process(&mut input, &mut spec).expect("fft sizes match");
        for k in 0..bins {
            let p = spec[k].norm_sqr();
            let g = if p > 0.0 {
                (1.0 - over * noise.power[k] / p).max(0.0).sqrt()
            } else {
                0.0
            };
            gains[k] = g;
        }
        // Smooth across three neighbouring bands, then over time: gains
        // rise at once (attacks stay sharp) and fall over ~2 frames.
        for k in 0..bins {
            let a = gains[k.saturating_sub(1)];
            let b = gains[k];
            let c = gains[(k + 1).min(bins - 1)];
            let g = (0.25 * a + 0.5 * b + 0.25 * c).max(floor);
            smooth[k] = if g >= prev[k] {
                g
            } else {
                0.5 * prev[k] + 0.5 * g
            };
            prev[k] = smooth[k];
        }
        for (c, g) in spec.iter_mut().zip(&smooth) {
            *c *= *g;
        }
        // The DC and Nyquist bins must be real for the inverse FFT.
        spec[0] = Complex::new(spec[0].re, 0.0);
        spec[bins - 1] = Complex::new(spec[bins - 1].re, 0.0);
        ifft.process(&mut spec, &mut input)
            .expect("fft sizes match");
        for j in 0..n {
            // realfft's inverse is unnormalised.
            out[start + j] += input[j] * w[j] / n as f32;
            norm[start + j] += w[j] * w[j];
        }
        start += hop;
    }
    (0..samples.len())
        .map(|i| {
            let d = norm[i + pad];
            if d > 1e-6 { out[i + pad] / d } else { 0.0 }
        })
        .collect()
}

/// Learns the noise from the quietest half second and removes it.
pub fn clean(samples: &[f32], sample_rate: u32, strength: f64) -> Vec<f32> {
    let p = learn_auto(samples, sample_rate);
    denoise(samples, sample_rate, &p, strength)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Biquad, amp_to_db, rms};

    const SR: u32 = 48_000;

    /// Seeded white noise at `db` dBFS RMS.
    fn hiss(n: usize, db: f32, seed: u64) -> Vec<f32> {
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let amp = db_to_amp(db) * 3f32.sqrt();
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 33) as f32 / (1u64 << 31) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    fn sine(n: usize, hz: f32, amp: f32, from: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                if i < from {
                    0.0
                } else {
                    amp * (std::f32::consts::TAU * hz * i as f32 / SR as f32).sin()
                }
            })
            .collect()
    }

    /// Level of `hz` in `x` (a narrow band-pass), in dB.
    fn tone_db(x: &[f32], hz: f32) -> f32 {
        let mut bp = Biquad::bandpass(SR as f32, hz, 30.0);
        let y: Vec<f32> = x.iter().map(|&s| bp.process(s)).collect();
        amp_to_db(rms(&y[y.len() / 4..]))
    }

    /// One second of hiss, then a 440 Hz tone over the hiss.
    fn take(noise_db: f32, seed: u64) -> (Vec<f32>, Vec<f32>, usize) {
        let n = 3 * SR as usize;
        let from = SR as usize;
        let noise = hiss(n, noise_db, seed);
        let tone = sine(n, 440.0, 0.3, from);
        let mix = noise.iter().zip(&tone).map(|(a, b)| a + b).collect();
        (mix, tone, from)
    }

    #[test]
    fn strength_point_eight_lowers_hiss_by_twelve_db_and_keeps_the_tone() {
        for (seed, noise_db) in [(1, -40.0), (2, -30.0), (3, -50.0)] {
            let (mix, tone, from) = take(noise_db, seed);
            let out = clean(&mix, SR, 0.8);
            assert_eq!(out.len(), mix.len());
            assert!(out.iter().all(|s| s.is_finite()));
            // Noise floor: the noise-only second (skip the very edges).
            let lo = SR as usize / 10;
            let before = amp_to_db(rms(&mix[lo..from - lo]));
            let after = amp_to_db(rms(&out[lo..from - lo]));
            let drop = before - after;
            // The tone: its level in the second half.
            let t_in = tone_db(&tone[from..], 440.0);
            let t_out = tone_db(&out[from..], 440.0);
            eprintln!(
                "noise {noise_db} dB: floor down {drop:.1} dB, tone changed {:.2} dB",
                t_out - t_in
            );
            assert!(drop >= 12.0, "noise floor only down {drop} dB");
            assert!(
                (t_out - t_in).abs() <= 2.0,
                "tone changed {} dB",
                t_out - t_in
            );
        }
    }

    #[test]
    fn noise_between_the_notes_goes_down_too() {
        // Noise left over while the tone plays: compare the residual
        // (output minus the clean tone) with the noise that was there.
        let (mix, tone, from) = take(-40.0, 4);
        let out = clean(&mix, SR, 0.8);
        let resid: Vec<f32> = out[from..]
            .iter()
            .zip(&tone[from..])
            .map(|(a, b)| a - b)
            .collect();
        let noise: Vec<f32> = mix[from..]
            .iter()
            .zip(&tone[from..])
            .map(|(a, b)| a - b)
            .collect();
        let drop = amp_to_db(rms(&noise)) - amp_to_db(rms(&resid));
        eprintln!("under the tone: noise down {drop:.1} dB");
        assert!(drop > 3.0, "{drop}");
    }

    #[test]
    fn mains_hum_is_removed() {
        let n = 2 * SR as usize;
        let hum: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                0.02 * (std::f32::consts::TAU * 50.0 * t).sin()
                    + 0.01 * (std::f32::consts::TAU * 150.0 * t).sin()
            })
            .collect();
        let out = clean(&hum, SR, 0.8);
        let lo = SR as usize / 10;
        let drop = amp_to_db(rms(&hum[lo..n - lo])) - amp_to_db(rms(&out[lo..n - lo]));
        assert!(drop >= 12.0, "hum only down {drop} dB");
    }

    #[test]
    fn strength_zero_is_untouched_and_more_strength_removes_more() {
        let (mix, _, from) = take(-40.0, 5);
        assert_eq!(clean(&mix, SR, 0.0), mix);
        let lo = SR as usize / 10;
        let level = |s: f64| amp_to_db(rms(&clean(&mix, SR, s)[lo..from - lo]));
        let (a, b, c) = (level(0.2), level(0.5), level(1.0));
        assert!(a > b && b > c, "{a} {b} {c}");
    }

    #[test]
    fn deterministic_and_handles_tiny_inputs() {
        let (mix, _, _) = take(-40.0, 6);
        assert_eq!(clean(&mix, SR, 0.7), clean(&mix, SR, 0.7));
        assert!(clean(&[], SR, 1.0).is_empty());
        let short = clean(&[0.1, -0.1, 0.2], SR, 1.0);
        assert_eq!(short.len(), 3);
        assert!(short.iter().all(|s| s.is_finite()));
        assert!(clean(&vec![0.0; 10_000], SR, 1.0).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn quietest_finds_the_gap() {
        let mut x = sine(SR as usize * 2, 300.0, 0.5, 0);
        for s in &mut x[SR as usize..SR as usize * 3 / 2] {
            *s = 0.0;
        }
        let r = quietest(&x, SR, 0.4);
        assert!(
            r.start >= SR as usize && r.end <= SR as usize * 3 / 2,
            "{r:?}"
        );
    }

    #[test]
    fn profile_level_is_about_right() {
        let p = learn(&hiss(SR as usize, -30.0, 7), SR);
        assert!((p.level_db() + 30.0).abs() < 3.0, "{}", p.level_db());
    }

    #[test]
    fn works_at_other_rates() {
        let sr = 44_100;
        let x = hiss(sr as usize, -40.0, 8);
        let p = learn(&x, SR);
        let y = denoise(&x, sr, &p, 0.8);
        assert_eq!(y.len(), x.len());
        assert!(rms(&y) < rms(&x));
    }
}
