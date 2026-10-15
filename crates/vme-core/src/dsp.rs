//! Small DSP building blocks shared by analysis and the mixer.

use std::f32::consts::PI;

/// Converts a linear amplitude to decibels, flooring silence at -200 dB.
pub fn amp_to_db(amp: f32) -> f32 {
    20.0 * amp.max(1e-10).log10()
}

pub fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// The `q`-quantile (0..=1) of `values`, by sorting a copy. Returns 0 for an
/// empty slice.
pub fn quantile(values: &[f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let idx = ((v.len() - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
    v[idx]
}

pub fn median(values: &[f32]) -> f32 {
    quantile(values, 0.5)
}

/// RBJ-cookbook biquad filter (direct form I).
#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn omega(sample_rate: f32, freq: f32) -> (f32, f32) {
        let w = 2.0 * PI * (freq / sample_rate).clamp(1e-5, 0.499);
        (w.cos(), w.sin())
    }

    pub fn highpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, sin) = Self::omega(sample_rate, freq);
        let alpha = sin / (2.0 * q);
        Self::from_coeffs(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    pub fn lowpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos, sin) = Self::omega(sample_rate, freq);
        let alpha = sin / (2.0 * q);
        Self::from_coeffs(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    pub fn peaking(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (cos, sin) = Self::omega(sample_rate, freq);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / (2.0 * q);
        Self::from_coeffs(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        )
    }

    pub fn low_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let (cos, sin) = Self::omega(sample_rate, freq);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_coeffs(
            a * ((a + 1.0) - (a - 1.0) * cos + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sa),
            (a + 1.0) + (a - 1.0) * cos + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sa,
        )
    }

    pub fn high_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let (cos, sin) = Self::omega(sample_rate, freq);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = sin / 2.0 * 2f32.sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Self::from_coeffs(
            a * ((a + 1.0) + (a - 1.0) * cos + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sa),
            (a + 1.0) - (a - 1.0) * cos + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sa,
        )
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn process_buffer(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s = self.process(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::sine;

    fn gain_at(mut f: Biquad, freq: f32) -> f32 {
        let mut x = sine(freq, 44_100, 0.5, 1.0);
        f.process_buffer(&mut x);
        rms(&x[11_025..]) / rms(&sine(freq, 44_100, 0.5, 1.0)[11_025..])
    }

    #[test]
    fn highpass_blocks_low_and_passes_high() {
        assert!(gain_at(Biquad::highpass(44_100.0, 200.0, 0.707), 30.0) < 0.05);
        assert!((gain_at(Biquad::highpass(44_100.0, 200.0, 0.707), 2_000.0) - 1.0).abs() < 0.05);
    }

    #[test]
    fn peaking_boosts_at_centre() {
        let g = gain_at(Biquad::peaking(44_100.0, 1_000.0, 1.0, 6.0), 1_000.0);
        assert!((amp_to_db(g) - 6.0).abs() < 0.3, "gain {g}");
    }

    #[test]
    fn quantiles() {
        let v = [5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(median(&v), 3.0);
        assert_eq!(quantile(&v, 0.0), 1.0);
        assert_eq!(quantile(&v, 1.0), 5.0);
    }
}
