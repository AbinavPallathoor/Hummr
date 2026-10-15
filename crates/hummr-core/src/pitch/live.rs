//! Low-latency single-frame pitch estimate for live meters.

use super::yin::{CmndfAnalyzer, YinGeometry, parabolic, troughs};
use crate::dsp::{amp_to_db, rms};

/// Reusable YIN analyser for the most recent audio of a live input.
pub struct LivePitch {
    sample_rate: u32,
    analyzer: CmndfAnalyzer,
    cmndf: Vec<f32>,
    /// Frames quieter than this (dBFS) are reported as unpitched.
    pub gate_db: f32,
}

impl LivePitch {
    pub fn new(sample_rate: u32) -> Self {
        let geom = YinGeometry::new(sample_rate, 70.0, 1200.0);
        Self {
            sample_rate,
            analyzer: CmndfAnalyzer::new(geom),
            cmndf: vec![0.0; geom.tau_max + 1],
            gate_db: -45.0,
        }
    }

    /// Samples of history needed for one estimate.
    pub fn window_len(&self) -> usize {
        self.analyzer.geometry().frame_len()
    }

    /// Estimates the pitch (Hz) of the last `window_len()` samples of `tail`.
    pub fn estimate(&mut self, tail: &[f32]) -> Option<f32> {
        let n = self.window_len();
        if tail.len() < n {
            return None;
        }
        let frame = &tail[tail.len() - n..];
        if amp_to_db(rms(frame)) < self.gate_db {
            return None;
        }
        let g = self.analyzer.geometry();
        self.analyzer.cmndf(frame, &mut self.cmndf);
        let ts = troughs(&self.cmndf, g.tau_min, g.tau_max);
        let tau = ts.into_iter().find(|&t| self.cmndf[t] < 0.2)?;
        Some(self.sample_rate as f32 / parabolic(&self.cmndf, tau))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::sine;

    #[test]
    fn tracks_and_gates() {
        let mut lp = LivePitch::new(48_000);
        let x = sine(330.0, 48_000, 0.2, 0.3);
        let hz = lp.estimate(&x).unwrap();
        assert!((hz - 330.0).abs() < 2.0);
        let quiet = sine(330.0, 48_000, 0.2, 0.001);
        assert_eq!(lp.estimate(&quiet), None);
        assert_eq!(lp.estimate(&x[..100]), None);
    }
}
