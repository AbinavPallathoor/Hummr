//! Probabilistic YIN (Mauch & Dixon, 2014).
//!
//! Stage 1: for each frame, run YIN with 100 thresholds weighted by a
//! Beta(2, 18) prior. Each threshold votes for the first CMNDF trough below
//! it, which yields a handful of pitch candidates with probabilities.
//!
//! Stage 2: an HMM with a voiced and an unvoiced state per pitch bin, with
//! transitions limited to a plausible slew rate, decoded with Viterbi. This
//! is what removes octave jumps and single-frame glitches.

use super::yin::{CmndfAnalyzer, YinGeometry, parabolic, troughs};
use crate::theory::{hz_to_midi, midi_to_hz};

#[derive(Clone, Debug, PartialEq)]
pub struct PyinParams {
    pub fmin: f32,
    pub fmax: f32,
    pub hop_seconds: f32,
    /// Resolution of the HMM pitch grid.
    pub bins_per_semitone: usize,
    /// Probability of switching between voiced and unvoiced per frame.
    pub switch_prob: f32,
    /// Fastest pitch slew the HMM will follow without paying a penalty.
    pub max_semitones_per_second: f32,
    /// Probability given to the global CMNDF minimum when no trough is below
    /// a threshold.
    pub no_trough_prob: f32,
}

impl Default for PyinParams {
    fn default() -> Self {
        Self {
            fmin: 55.0,
            fmax: 1400.0,
            hop_seconds: 0.005,
            bins_per_semitone: 5,
            switch_prob: 0.01,
            // librosa's default of 35.92 octaves/s.
            max_semitones_per_second: 431.0,
            no_trough_prob: 0.01,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchFrame {
    /// Centre of the frame in seconds.
    pub time: f64,
    pub f0: Option<f32>,
    /// Total probability mass of voiced candidates in this frame.
    pub voiced_prob: f32,
    /// RMS level of the frame's integration window.
    pub rms: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PitchTrack {
    pub hop_seconds: f64,
    pub sample_rate: u32,
    pub frames: Vec<PitchFrame>,
}

impl PitchTrack {
    /// Writes a CSV dump for debugging: time, f0, MIDI pitch, voicing, level.
    pub fn to_csv(&self) -> String {
        let mut s = String::from("time,f0_hz,midi,voiced_prob,rms_db\n");
        for f in &self.frames {
            let (hz, midi) = match f.f0 {
                Some(h) => (format!("{h:.2}"), format!("{:.2}", hz_to_midi(h))),
                None => (String::new(), String::new()),
            };
            s.push_str(&format!(
                "{:.4},{hz},{midi},{:.3},{:.1}\n",
                f.time,
                f.voiced_prob,
                crate::dsp::amp_to_db(f.rms)
            ));
        }
        s
    }
}

/// Probability mass of each of the 100 thresholds 0.01..=1.00 under a
/// Beta(2, 18) prior (mean 0.1). For integer parameters the regularised
/// incomplete beta function is a binomial tail sum.
fn beta_threshold_probs() -> Vec<f64> {
    const A: u32 = 2;
    const B: u32 = 18;
    let n = A + B - 1;
    let binom =
        |k: u32| -> f64 { (0..k).fold(1.0, |acc, i| acc * f64::from(n - i) / f64::from(i + 1)) };
    let cdf = |x: f64| -> f64 {
        (A..=n)
            .map(|j| binom(j) * x.powi(j as i32) * (1.0 - x).powi((n - j) as i32))
            .sum()
    };
    (1..=100)
        .map(|i| cdf(f64::from(i) / 100.0) - cdf(f64::from(i - 1) / 100.0))
        .collect()
}

struct Candidate {
    bin: usize,
    prob: f32,
    hz: f32,
}

struct FrameObs {
    candidates: Vec<Candidate>,
    voiced_prob: f32,
}

/// Runs pYIN over a mono signal.
pub fn pyin(samples: &[f32], sample_rate: u32, params: &PyinParams) -> PitchTrack {
    let geom = YinGeometry::new(sample_rate, params.fmin, params.fmax);
    let hop = ((params.hop_seconds * sample_rate as f32).round() as usize).max(1);
    let frame_len = geom.frame_len();
    let n_frames = samples.len().div_ceil(hop);

    let midi_min = hz_to_midi(params.fmin);
    let midi_max = hz_to_midi(params.fmax);
    let bps = params.bins_per_semitone as f32;
    let n_bins = ((midi_max - midi_min) * bps).ceil() as usize + 1;
    let bin_of = |hz: f32| -> Option<usize> {
        let b = ((hz_to_midi(hz) - midi_min) * bps).round();
        (b >= 0.0 && (b as usize) < n_bins).then_some(b as usize)
    };

    let beta = beta_threshold_probs();
    let thresholds: Vec<f32> = (1..=100).map(|i| i as f32 / 100.0).collect();

    let mut analyzer = CmndfAnalyzer::new(geom);
    let mut cmndf = vec![0.0f32; geom.tau_max + 1];
    let mut frame = vec![0.0f32; frame_len];
    let mut obs = Vec::with_capacity(n_frames);
    let mut frames = Vec::with_capacity(n_frames);

    for t in 0..n_frames {
        // Frames are centred on t * hop, zero-padded at the edges.
        let centre = (t * hop) as isize;
        let start = centre - (frame_len / 2) as isize;
        for (i, f) in frame.iter_mut().enumerate() {
            let idx = start + i as isize;
            *f = if idx >= 0 && (idx as usize) < samples.len() {
                samples[idx as usize]
            } else {
                0.0
            };
        }
        let energy = analyzer.cmndf(&frame, &mut cmndf);
        let rms = (energy / geom.window as f64).sqrt() as f32;
        let time = (t * hop) as f64 / f64::from(sample_rate);

        let mut candidates: Vec<Candidate> = Vec::new();
        if energy > 1e-9 * geom.window as f64 {
            let ts = troughs(&cmndf, geom.tau_min, geom.tau_max);
            let global_min = (geom.tau_min..geom.tau_max)
                .min_by(|&a, &b| cmndf[a].total_cmp(&cmndf[b]))
                .unwrap_or(geom.tau_min);
            let add = |tau: usize, p: f64, cands: &mut Vec<Candidate>| {
                let hz = sample_rate as f32 / parabolic(&cmndf, tau);
                if let Some(bin) = bin_of(hz) {
                    match cands.iter_mut().find(|c| c.bin == bin) {
                        Some(c) => c.prob += p as f32,
                        None => cands.push(Candidate {
                            bin,
                            prob: p as f32,
                            hz,
                        }),
                    }
                }
            };
            for (s, &p) in thresholds.iter().zip(&beta) {
                match ts.iter().copied().find(|&tau| cmndf[tau] < *s) {
                    Some(tau) => add(tau, p, &mut candidates),
                    None => add(
                        global_min,
                        p * f64::from(params.no_trough_prob),
                        &mut candidates,
                    ),
                }
            }
        }
        let voiced_prob = candidates
            .iter()
            .map(|c| c.prob)
            .sum::<f32>()
            .clamp(0.0, 1.0);
        obs.push(FrameObs {
            candidates,
            voiced_prob,
        });
        frames.push(PitchFrame {
            time,
            f0: None,
            voiced_prob,
            rms,
        });
    }

    let max_step =
        ((params.max_semitones_per_second * params.hop_seconds * bps).ceil() as usize).max(1);
    let path = viterbi(&obs, n_bins, max_step, params.switch_prob);

    for (t, state) in path.into_iter().enumerate() {
        if state < n_bins {
            // Prefer the exact candidate frequency near the chosen bin over the
            // bin centre, so we keep sub-bin accuracy for tuning estimates.
            let near = obs[t]
                .candidates
                .iter()
                .filter(|c| c.bin.abs_diff(state) <= 2)
                .max_by(|a, b| a.prob.total_cmp(&b.prob));
            let hz = near.map_or_else(|| midi_to_hz(midi_min + state as f32 / bps), |c| c.hz);
            frames[t].f0 = Some(hz);
        }
    }

    PitchTrack {
        hop_seconds: hop as f64 / f64::from(sample_rate),
        sample_rate,
        frames,
    }
}

/// Viterbi over `2 * n_bins` states: `0..n` voiced, `n..2n` unvoiced. Pitch
/// transitions use a triangular kernel of half-width `max_step` bins.
fn viterbi(obs: &[FrameObs], n: usize, max_step: usize, switch_prob: f32) -> Vec<usize> {
    if obs.is_empty() {
        return Vec::new();
    }
    let n_states = 2 * n;
    let ln = |x: f32| x.max(1e-30).ln();

    let ln_tri: Vec<f32> = (0..=max_step)
        .map(|k| ((max_step + 1 - k) as f32).ln())
        .collect();
    let ln_norm: Vec<f32> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(max_step);
            let hi = (i + max_step).min(n - 1);
            ((lo..=hi)
                .map(|j| (max_step + 1 - i.abs_diff(j)) as f32)
                .sum::<f32>())
            .ln()
        })
        .collect();
    let ln_stay = ln(1.0 - switch_prob);
    let ln_switch = ln(switch_prob);

    // Voiced bins emit the candidate mass at that bin. Unvoiced states all
    // emit (1 - voiced_prob): unlike librosa we do not divide this by n,
    // which would make even pure noise look voiced.
    let emissions = |o: &FrameObs, out: &mut [f32]| {
        out[..n].fill(ln(0.0));
        for c in &o.candidates {
            out[c.bin] = ln(c.prob);
        }
        out[n..].fill(ln(1.0 - o.voiced_prob));
    };

    let mut emis = vec![0.0f32; n_states];
    let mut delta = vec![0.0f32; n_states];
    let mut next = vec![0.0f32; n_states];
    let mut adjusted = vec![0.0f32; n_states];
    let mut back = vec![0u16; obs.len() * n_states];

    emissions(&obs[0], &mut emis);
    let ln_init = -(n_states as f32).ln();
    for (d, e) in delta.iter_mut().zip(&emis) {
        *d = e + ln_init;
    }

    for t in 1..obs.len() {
        emissions(&obs[t], &mut emis);
        for g in 0..2 {
            for i in 0..n {
                adjusted[g * n + i] = delta[g * n + i] - ln_norm[i];
            }
        }
        let row = &mut back[t * n_states..(t + 1) * n_states];
        for g in 0..2 {
            for j in 0..n {
                let lo = j.saturating_sub(max_step);
                let hi = (j + max_step).min(n - 1);
                let mut best = f32::NEG_INFINITY;
                let mut arg = 0usize;
                for gs in 0..2 {
                    let lt = if gs == g { ln_stay } else { ln_switch };
                    let base = gs * n;
                    for i in lo..=hi {
                        let v = adjusted[base + i] + lt + ln_tri[i.abs_diff(j)];
                        if v > best {
                            best = v;
                            arg = base + i;
                        }
                    }
                }
                next[g * n + j] = best + emis[g * n + j];
                row[g * n + j] = arg as u16;
            }
        }
        std::mem::swap(&mut delta, &mut next);
    }

    let mut state = delta
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let mut path = vec![0usize; obs.len()];
    for t in (0..obs.len()).rev() {
        path[t] = state;
        if t > 0 {
            state = usize::from(back[t * n_states + state]);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{self, HumParams, Tone, sine};

    fn voiced_midis(track: &PitchTrack, from: f64, to: f64) -> Vec<f32> {
        track
            .frames
            .iter()
            .filter(|f| f.time >= from && f.time <= to)
            .filter_map(|f| f.f0.map(hz_to_midi))
            .collect()
    }

    #[test]
    fn beta_prior_sums_to_one() {
        let p = beta_threshold_probs();
        assert_eq!(p.len(), 100);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        // Mode of Beta(2,18) is 1/18 ≈ 0.056, so thresholds 0.05/0.06 dominate.
        let max_i = (0..100).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap();
        assert!((4..=6).contains(&max_i), "max at {max_i}");
    }

    #[test]
    fn steady_sines_are_tracked_within_ten_cents() {
        for &f in &[98.0f32, 196.0, 261.63, 392.0, 659.25] {
            let x = sine(f, 44_100, 0.6, 0.5);
            let track = pyin(&x, 44_100, &PyinParams::default());
            let m = voiced_midis(&track, 0.1, 0.5);
            assert!(m.len() > 70, "{f} Hz: only {} voiced frames", m.len());
            let target = hz_to_midi(f);
            for v in m {
                assert!((v - target).abs() < 0.1, "{f} Hz: frame at {v} vs {target}");
            }
        }
    }

    #[test]
    fn vibrato_is_followed_without_octave_jumps() {
        // 5.5 Hz vibrato of ±60 cents around A3.
        let sr = 44_100;
        let x = synthetic::hum(
            &[Tone::new(57.0, 0.0, 1.5)],
            &HumParams {
                vibrato_cents: 60.0,
                vibrato_hz: 5.5,
                noise_db: -60.0,
                ..HumParams::new(sr)
            },
            1,
        );
        let track = pyin(&x, sr, &PyinParams::default());
        let m = voiced_midis(&track, 0.1, 1.4);
        assert!(m.len() > 200);
        let lo = m.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = m.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(lo > 57.0 - 0.75 && hi < 57.0 + 0.75, "range {lo}..{hi}");
        // The vibrato must actually be tracked, not smoothed flat.
        assert!(hi - lo > 0.8, "vibrato depth only {}", hi - lo);
    }

    #[test]
    fn weak_fundamental_does_not_cause_octave_errors() {
        // Harmonic-rich tone whose 2nd harmonic is stronger than the 1st,
        // a classic octave trap.
        let sr = 44_100;
        let f = 130.81f32;
        let x: Vec<f32> = (0..(sr as usize))
            .map(|i| {
                let t = i as f32 / sr as f32;
                let w = 2.0 * std::f32::consts::PI * f * t;
                0.2 * w.sin()
                    + 0.5 * (2.0 * w).sin()
                    + 0.3 * (3.0 * w).sin()
                    + 0.2 * (4.0 * w).sin()
            })
            .collect();
        let track = pyin(&x, sr, &PyinParams::default());
        let m = voiced_midis(&track, 0.1, 0.9);
        let wrong = m.iter().filter(|v| (**v - 48.0).abs() > 0.5).count();
        assert!(m.len() > 100);
        assert_eq!(wrong, 0, "{wrong} of {} frames off C3", m.len());
    }

    #[test]
    fn silence_and_noise_are_unvoiced() {
        let sr = 22_050;
        let silent = vec![0.0f32; sr as usize];
        let track = pyin(&silent, sr, &PyinParams::default());
        assert!(track.frames.iter().all(|f| f.f0.is_none()));

        let noise = synthetic::white_noise(sr as usize, 0.3, 7);
        let track = pyin(&noise, sr, &PyinParams::default());
        let voiced = track.frames.iter().filter(|f| f.f0.is_some()).count();
        assert!(
            voiced * 10 < track.frames.len(),
            "{voiced} of {} noise frames voiced",
            track.frames.len()
        );
    }

    #[test]
    fn note_change_is_tracked_promptly() {
        let sr = 44_100;
        let x = synthetic::hum(
            &[Tone::new(60.0, 0.0, 0.5), Tone::new(67.0, 0.5, 0.5)],
            &HumParams {
                vibrato_cents: 0.0,
                glide_seconds: 0.0,
                ..HumParams::new(sr)
            },
            3,
        );
        let track = pyin(&x, sr, &PyinParams::default());
        let before = voiced_midis(&track, 0.1, 0.45);
        let after = voiced_midis(&track, 0.56, 0.9);
        assert!(before.iter().all(|m| (m - 60.0).abs() < 0.3));
        assert!(after.iter().all(|m| (m - 67.0).abs() < 0.3));
    }
}
