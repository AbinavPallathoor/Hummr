//! YIN difference function and cumulative mean normalised difference
//! (de Cheveigné & Kawahara, 2002), computed with an FFT.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Lag range and window sizes for a given sample rate and pitch range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YinGeometry {
    /// Integration window (samples).
    pub window: usize,
    pub tau_min: usize,
    pub tau_max: usize,
}

impl YinGeometry {
    pub fn new(sample_rate: u32, fmin: f32, fmax: f32) -> Self {
        let sr = sample_rate as f32;
        let tau_min = ((sr / fmax).floor() as usize).max(2);
        let tau_max = (sr / fmin).ceil() as usize + 1;
        Self {
            window: tau_max,
            tau_min,
            tau_max,
        }
    }

    /// Samples needed per analysis frame.
    pub fn frame_len(&self) -> usize {
        self.window + self.tau_max + 1
    }
}

/// Reusable FFT state for computing the CMNDF of successive frames.
pub struct CmndfAnalyzer {
    geom: YinGeometry,
    n_fft: usize,
    r2c: Arc<dyn RealToComplex<f32>>,
    c2r: Arc<dyn ComplexToReal<f32>>,
    buf_a: Vec<f32>,
    buf_x: Vec<f32>,
    spec_a: Vec<Complex<f32>>,
    spec_x: Vec<Complex<f32>>,
    corr: Vec<f32>,
    prefix_sq: Vec<f64>,
    scratch_r2c: Vec<Complex<f32>>,
    scratch_c2r: Vec<Complex<f32>>,
}

impl CmndfAnalyzer {
    pub fn new(geom: YinGeometry) -> Self {
        let n_fft = geom.frame_len().next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(n_fft);
        let c2r = planner.plan_fft_inverse(n_fft);
        Self {
            geom,
            n_fft,
            buf_a: r2c.make_input_vec(),
            buf_x: r2c.make_input_vec(),
            spec_a: r2c.make_output_vec(),
            spec_x: r2c.make_output_vec(),
            corr: c2r.make_output_vec(),
            scratch_r2c: r2c.make_scratch_vec(),
            scratch_c2r: c2r.make_scratch_vec(),
            prefix_sq: vec![0.0; geom.frame_len() + 1],
            r2c,
            c2r,
        }
    }

    pub fn geometry(&self) -> YinGeometry {
        self.geom
    }

    /// Fills `out[0..=tau_max]` with the CMNDF of `frame` (which must hold
    /// `frame_len()` samples). Returns the energy of the integration window;
    /// a value near zero means the frame is silent and the CMNDF is meaningless.
    pub fn cmndf(&mut self, frame: &[f32], out: &mut [f32]) -> f64 {
        let g = self.geom;
        let len = g.frame_len();
        assert!(frame.len() >= len && out.len() > g.tau_max);

        self.prefix_sq[0] = 0.0;
        for (i, &s) in frame[..len].iter().enumerate() {
            self.prefix_sq[i + 1] = self.prefix_sq[i] + f64::from(s) * f64::from(s);
        }
        let energy = self.prefix_sq[g.window];

        // r(tau) = sum_{j<W} x[j] * x[j+tau] via conj(FFT(a)) * FFT(x).
        self.buf_a.fill(0.0);
        self.buf_a[..g.window].copy_from_slice(&frame[..g.window]);
        self.buf_x.fill(0.0);
        self.buf_x[..len].copy_from_slice(&frame[..len]);
        self.r2c
            .process_with_scratch(&mut self.buf_a, &mut self.spec_a, &mut self.scratch_r2c)
            .expect("fft sizes are fixed at construction");
        self.r2c
            .process_with_scratch(&mut self.buf_x, &mut self.spec_x, &mut self.scratch_r2c)
            .expect("fft sizes are fixed at construction");
        for (x, a) in self.spec_x.iter_mut().zip(&self.spec_a) {
            *x *= a.conj();
        }
        let last = self.spec_x.len() - 1;
        self.spec_x[0].im = 0.0;
        self.spec_x[last].im = 0.0;
        self.c2r
            .process_with_scratch(&mut self.spec_x, &mut self.corr, &mut self.scratch_c2r)
            .expect("fft sizes are fixed at construction");
        let scale = 1.0 / self.n_fft as f64;

        out[0] = 1.0;
        let mut running = 0.0f64;
        for (tau, o) in out.iter_mut().enumerate().take(g.tau_max + 1).skip(1) {
            let e_tau = self.prefix_sq[tau + g.window] - self.prefix_sq[tau];
            let r = f64::from(self.corr[tau]) * scale;
            let d = (energy + e_tau - 2.0 * r).max(0.0);
            running += d;
            *o = if running > 0.0 {
                (d * tau as f64 / running) as f32
            } else {
                1.0
            };
        }
        energy
    }
}

/// Plain O(W·tau) difference function, used to check the FFT version.
pub fn difference_naive(frame: &[f32], window: usize, tau_max: usize) -> Vec<f32> {
    (0..=tau_max)
        .map(|tau| {
            (0..window)
                .map(|j| {
                    let d = f64::from(frame[j]) - f64::from(frame[j + tau]);
                    d * d
                })
                .sum::<f64>() as f32
        })
        .collect()
}

/// Local minima of the CMNDF within `[tau_min, tau_max)`.
pub fn troughs(cmndf: &[f32], tau_min: usize, tau_max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for tau in tau_min..tau_max {
        let left = if tau > tau_min {
            cmndf[tau - 1]
        } else {
            f32::INFINITY
        };
        if cmndf[tau] < left && cmndf[tau] <= cmndf[tau + 1] {
            out.push(tau);
        }
    }
    out
}

/// Sub-sample lag refinement by fitting a parabola through the trough.
pub fn parabolic(cmndf: &[f32], tau: usize) -> f32 {
    if tau == 0 || tau + 1 >= cmndf.len() {
        return tau as f32;
    }
    let (a, b, c) = (cmndf[tau - 1], cmndf[tau], cmndf[tau + 1]);
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-12 {
        return tau as f32;
    }
    tau as f32 + (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
}

/// Classic single-threshold YIN on one frame. Useful for live meters where
/// latency matters more than smoothing.
pub fn yin_pitch(
    frame: &[f32],
    sample_rate: u32,
    fmin: f32,
    fmax: f32,
    threshold: f32,
) -> Option<f32> {
    let geom = YinGeometry::new(sample_rate, fmin, fmax);
    if frame.len() < geom.frame_len() {
        return None;
    }
    let mut an = CmndfAnalyzer::new(geom);
    let mut d = vec![0.0; geom.tau_max + 1];
    if an.cmndf(frame, &mut d) < 1e-8 {
        return None;
    }
    let ts = troughs(&d, geom.tau_min, geom.tau_max);
    let tau = ts.iter().copied().find(|&t| d[t] < threshold)?;
    Some(sample_rate as f32 / parabolic(&d, tau))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::sine;

    #[test]
    fn fft_difference_matches_naive() {
        let geom = YinGeometry::new(16_000, 80.0, 800.0);
        let x: Vec<f32> = sine(173.0, 16_000, 0.2, 0.7)
            .iter()
            .enumerate()
            .map(|(i, s)| s + 0.1 * ((i as f32) * 0.37).sin())
            .collect();
        let frame = &x[..geom.frame_len()];
        let naive = difference_naive(frame, geom.window, geom.tau_max);
        // Rebuild CMNDF from the naive difference and compare.
        let mut expected = vec![1.0f32; geom.tau_max + 1];
        let mut running = 0.0f64;
        for tau in 1..=geom.tau_max {
            running += f64::from(naive[tau]);
            expected[tau] = (f64::from(naive[tau]) * tau as f64 / running) as f32;
        }
        let mut an = CmndfAnalyzer::new(geom);
        let mut got = vec![0.0; geom.tau_max + 1];
        an.cmndf(frame, &mut got);
        for tau in 1..=geom.tau_max {
            assert!(
                (got[tau] - expected[tau]).abs() < 1e-3,
                "tau {tau}: fft {} vs naive {}",
                got[tau],
                expected[tau]
            );
        }
    }

    #[test]
    fn yin_finds_sine_pitch() {
        for &f in &[82.4f32, 110.0, 220.0, 261.63, 440.0, 880.0] {
            let x = sine(f, 44_100, 0.2, 0.5);
            let est = yin_pitch(&x, 44_100, 60.0, 1200.0, 0.15).unwrap();
            assert!((est / f - 1.0).abs() < 0.005, "{f} Hz estimated as {est}");
        }
    }

    #[test]
    fn yin_silence_is_unvoiced() {
        let x = vec![0.0f32; 8_192];
        assert_eq!(yin_pitch(&x, 44_100, 60.0, 1200.0, 0.15), None);
    }
}
