//! Onset (hit) detection by spectral flux.
//!
//! The signal is cut into overlapping Hann-windowed frames (about 21 ms long,
//! a new one every 2.7 ms). For each frame we take the log magnitude
//! spectrum and sum how much each bin *rose* compared with two frames
//! earlier (a slightly smeared previous frame, as in SuperFlux, so vibrato
//! does not look like a new note). That "onset strength" curve peaks
//! wherever something new starts. Peaks are picked against a moving
//! threshold, and each peak's time is then refined to the start of the
//! attack in the waveform itself, which is accurate to about a millisecond.

use realfft::RealFftPlanner;

use crate::dsp::Biquad;

/// Onset-strength curves, one value per analysis frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Odf {
    /// Spectral flux over the whole spectrum, normalised so the max is 1.
    pub flux: Vec<f32>,
    /// Flux weighted towards the body of the sound (high frequencies count
    /// less), used for tempo so hats do not dominate the beat.
    pub body: Vec<f32>,
    /// Flux below 200 Hz (kicks), used to guess where bars start.
    pub low: Vec<f32>,
    /// Rise in signal power (not log-compressed), so loud hits dominate;
    /// used to tell beats from off-beats.
    pub power: Vec<f32>,
    /// Samples between frames.
    pub hop: usize,
    pub sample_rate: u32,
}

impl Odf {
    pub fn frame_rate(&self) -> f64 {
        f64::from(self.sample_rate) / self.hop as f64
    }

    /// Time in seconds of (fractional) frame `i` (frames are centred).
    pub fn time_of(&self, i: f64) -> f64 {
        i * self.hop as f64 / f64::from(self.sample_rate)
    }

    pub fn len(&self) -> usize {
        self.flux.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flux.is_empty()
    }
}

/// A detected hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onset {
    /// Start of the attack, seconds.
    pub time: f64,
    /// Peak onset strength, 0..=1 relative to the strongest hit.
    pub strength: f32,
}

/// Analysis frame and hop for a sample rate (~21 ms and ~2.7 ms).
pub fn frame_and_hop(sample_rate: u32) -> (usize, usize) {
    let frame = ((sample_rate as f64 * 0.021) as usize).next_power_of_two();
    let hop = ((sample_rate as f64 / 375.0).round() as usize).max(1);
    (frame, hop)
}

/// Computes the onset-strength curves of a mono signal.
pub fn onset_strength(samples: &[f32], sample_rate: u32) -> Odf {
    let (n, hop) = frame_and_hop(sample_rate);
    let frames = samples.len() / hop + 1;
    if samples.is_empty() {
        return Odf {
            hop,
            sample_rate,
            ..Odf::default()
        };
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect();
    let bins = spec.len();
    let bin_hz = sample_rate as f32 / n as f32;
    let low_bins = ((200.0 / bin_hz) as usize).max(2);
    // Body weighting: full weight to 3 kHz, then falling to 0.25.
    let weight: Vec<f32> = (0..bins)
        .map(|k| {
            let f = k as f32 * bin_hz;
            if f <= 3_000.0 {
                1.0
            } else {
                (3_000.0 / f).max(0.25)
            }
        })
        .collect();

    let lag = 2;
    let mut history: Vec<Vec<f32>> = Vec::with_capacity(lag + 1);
    let mut flux = vec![0.0f32; frames];
    let mut body = vec![0.0f32; frames];
    let mut low = vec![0.0f32; frames];
    let mut power = vec![0.0f32; frames];
    let mut power_hist: Vec<f32> = Vec::with_capacity(lag + 1);
    let half = n / 2;
    for f in 0..frames {
        let centre = f * hop;
        for (i, (x, w)) in input.iter_mut().zip(&window).enumerate() {
            let j = (centre + i) as i64 - half as i64;
            let s = if j >= 0 {
                samples.get(j as usize).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *x = s * w;
        }
        fft.process_with_scratch(&mut input, &mut spec, &mut scratch)
            .expect("fft sizes are fixed");
        let mag: Vec<f32> = spec.iter().map(|c| (1.0 + 100.0 * c.norm()).ln()).collect();
        let p: f32 = spec.iter().map(|c| c.norm_sqr()).sum();
        if power_hist.len() == lag {
            power[f] = (p - power_hist[0]).max(0.0).sqrt();
            power_hist.remove(0);
        }
        power_hist.push(p);
        if history.len() == lag {
            let prev = &history[0];
            let (mut a, mut b, mut c) = (0.0f32, 0.0f32, 0.0f32);
            for k in 0..bins {
                // Max over neighbouring bins of the earlier frame.
                let p = prev[k]
                    .max(prev[k.saturating_sub(1)])
                    .max(prev[(k + 1).min(bins - 1)]);
                let d = (mag[k] - p).max(0.0);
                a += d;
                b += d * weight[k];
                if k < low_bins {
                    c += d;
                }
            }
            flux[f] = a;
            body[f] = b;
            low[f] = c;
            history.remove(0);
        }
        history.push(mag);
    }
    for v in [&mut flux, &mut body, &mut low, &mut power] {
        let max = v.iter().fold(0.0f32, |m, x| m.max(*x));
        if max > 0.0 {
            for x in v.iter_mut() {
                *x /= max;
            }
        }
    }
    Odf {
        flux,
        body,
        low,
        power,
        hop,
        sample_rate,
    }
}

/// Picks onsets from the flux curve and refines their times on the
/// waveform. `sensitivity` 0..=1 (0.5 is a good default) lowers the
/// threshold as it rises.
pub fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<Onset> {
    let odf = onset_strength(samples, sample_rate);
    pick_onsets(samples, &odf, 0.5)
}

/// Peak picking on an onset-strength curve.
pub fn pick_onsets(samples: &[f32], odf: &Odf, sensitivity: f32) -> Vec<Onset> {
    let fr = odf.frame_rate();
    let w_max = ((0.03 * fr) as usize).max(1); // hits closer than 30 ms merge
    let w_mean = ((0.1 * fr) as usize).max(1);
    let delta = 0.12 - 0.1 * sensitivity.clamp(0.0, 1.0);
    let o = &odf.flux;
    // Ignore frames where the signal is essentially silent.
    let gate = 10f32.powf(-55.0 / 20.0);
    let mut out = Vec::new();
    for i in 0..o.len() {
        let lo = i.saturating_sub(w_max);
        let hi = (i + w_max).min(o.len() - 1);
        let v = o[i];
        if v <= 0.0 || o[lo..=hi].iter().any(|&x| x > v) {
            continue;
        }
        // Plateaus: keep only the first frame.
        if i > 0 && o[i - 1] == v {
            continue;
        }
        let ml = i.saturating_sub(w_mean);
        let mh = (i + w_mean).min(o.len() - 1);
        let mean = o[ml..=mh].iter().sum::<f32>() / (mh - ml + 1) as f32;
        // Real hits stand far above their surroundings; the random ripple
        // of steady noise does not.
        if v < mean + delta || v < 1.6 * mean {
            continue;
        }
        let t = odf.time_of(i as f64);
        let s = (t * f64::from(odf.sample_rate)) as usize;
        let near = &samples[s.saturating_sub(odf.hop * 4).min(samples.len())
            ..(s + odf.hop * 8).min(samples.len())];
        if near.iter().all(|x| x.abs() < gate) {
            continue;
        }
        out.push(Onset {
            time: refine_attack(samples, odf.sample_rate, t),
            strength: v,
        });
    }
    // Refinement can pull two neighbours onto the same attack.
    out.dedup_by(|b, a| (b.time - a.time).abs() < 0.015);
    out
}

/// Moves a rough onset time `t` to the start of the attack: the point where
/// the waveform envelope rises through 30% of the way from the quiet before
/// the hit to its peak. The envelope is taken above 500 Hz when there is
/// enough there, so the slow wobble of a ringing bass note (a kick tail
/// under a hat) is not mistaken for an attack.
pub fn refine_attack(samples: &[f32], sample_rate: u32, t: f64) -> f64 {
    let sr = f64::from(sample_rate);
    let from = ((t - 0.025) * sr).max(0.0) as usize;
    let to = (((t + 0.02) * sr) as usize).min(samples.len());
    if to <= from + 64 {
        return t;
    }
    // Run the filter over some pre-roll so it has settled by `from`.
    let pre = from.saturating_sub((sr * 0.01) as usize);
    let mut hp = Biquad::highpass(sample_rate as f32, 500.0, 0.707);
    let filtered: Vec<f32> = samples[pre..to].iter().map(|&x| hp.process(x)).collect();
    let high = &filtered[from - pre..];
    let raw = &samples[from..to];
    let peak_of = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let sig = if peak_of(high) > 0.1 * peak_of(raw) {
        high
    } else {
        raw
    };
    match attack_index(sig, (sr * 0.0005) as usize) {
        Some(k) => (from + k) as f64 / sr,
        None => t,
    }
}

/// Index of the attack in `x` using a max-abs envelope of `w`-sample blocks.
fn attack_index(x: &[f32], w: usize) -> Option<usize> {
    let w = w.max(1);
    let env: Vec<f32> = x
        .chunks(w)
        .map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs())))
        .collect();
    let (peak_i, &peak) = env.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let floor = env[..=peak_i].iter().fold(f32::INFINITY, |m, &v| m.min(v));
    if peak <= floor * 1.5 {
        return None;
    }
    let thr = floor + 0.3 * (peak - floor);
    let mut k = peak_i;
    while k > 0 && env[k - 1] >= thr {
        k -= 1;
    }
    Some(k * w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{drum_loop, white_noise};

    #[test]
    fn finds_every_hit_of_a_loop_within_a_few_ms() {
        for (bpm, which) in [(90.0, 0), (120.0, 1), (150.0, 2)] {
            let (audio, hits) = drum_loop(bpm, 2, which, 0.3, 0.0, 4, 48_000);
            let onsets = detect_onsets(&audio, 48_000);
            // Simultaneous hits (kick + hat) are one onset.
            let mut times: Vec<f64> = hits.iter().map(|h| h.time).collect();
            times.dedup_by(|b, a| (*b - *a).abs() < 0.01);
            let mut found = 0;
            for t in &times {
                if onsets.iter().any(|o| (o.time - t).abs() < 0.004) {
                    found += 1;
                }
            }
            let hits = times;
            assert!(
                found as f64 >= hits.len() as f64 * 0.95,
                "{bpm} BPM: found {found} of {} hits",
                hits.len()
            );
            assert!(
                onsets.len() <= hits.len() + 2,
                "{bpm} BPM: {} onsets for {} hits",
                onsets.len(),
                hits.len()
            );
            assert!(onsets.iter().any(|o| o.strength > 0.5));
        }
    }

    #[test]
    fn silence_has_no_onsets() {
        assert!(detect_onsets(&vec![0.0; 48_000], 48_000).is_empty());
        assert!(detect_onsets(&[], 48_000).is_empty());
    }

    #[test]
    fn steady_noise_has_few_onsets() {
        for seed in [9, 10, 11] {
            let n = white_noise(96_000, 0.3, seed);
            let o = detect_onsets(&n, 48_000);
            assert!(o.len() <= 3, "{} onsets in 2 s of noise", o.len());
        }
    }
}
