//! Tempo, beat phase and confidence for recordings with a beat.
//!
//! 1. Onset-strength curve (see [`crate::onset`]), weighted towards the body
//!    of the sound so hi-hats do not dominate.
//! 2. Its autocorrelation scores every tempo from 40 to 240 BPM; a mild
//!    preference for tempos people usually count in (around 115 BPM) breaks
//!    the half/double ambiguity.
//! 3. The best tempo and the beat phase are refined together by sliding a
//!    comb of beat positions over the curve, then by a least-squares line
//!    through the detected hits that sit on beats.
//! 4. Confidence is how much of the hit energy lands on the 8th-note grid
//!    compared with what random timing would give.
//! 5. Beat 1 is the beat position with the most low-frequency (kick) energy.

use crate::onset::{Odf, Onset, onset_strength, pick_onsets};

/// What [`estimate`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatEstimate {
    pub bpm: f64,
    /// Time in seconds of the first beat (0 ≤ phase < one beat).
    pub phase: f64,
    /// Time in seconds of the first likely "beat 1" of a bar.
    pub downbeat: f64,
    /// 0 (no steady beat) ..= 1 (rock solid).
    pub confidence: f64,
    /// Half and double tempo, most likely first.
    pub alternatives: Vec<f64>,
    /// The hits the estimate is based on.
    pub onsets: Vec<Onset>,
}

/// Confidence at or above which the estimate is reported as "sure".
pub const SURE: f64 = 0.5;
/// Below this there is no usable beat.
pub const NO_BEAT: f64 = 0.15;

impl BeatEstimate {
    pub fn period(&self) -> f64 {
        60.0 / self.bpm
    }

    pub fn is_sure(&self) -> bool {
        self.confidence >= SURE
    }

    pub fn has_beat(&self) -> bool {
        self.confidence >= NO_BEAT
    }

    /// Plain-words summary: "118 BPM, sure", "118 BPM or 59? not sure" or
    /// "no steady beat found".
    pub fn describe(&self) -> String {
        if !self.has_beat() {
            return "no steady beat found".into();
        }
        let b = fmt_bpm(self.bpm);
        if self.is_sure() {
            format!("{b} BPM, sure")
        } else if let Some(alt) = self.alternatives.first() {
            format!("{b} BPM or {}? not sure", fmt_bpm(*alt))
        } else {
            format!("{b} BPM? not sure")
        }
    }

    /// Time (seconds) of beat `k` (beat 0 is at `phase`).
    pub fn beat_time(&self, k: f64) -> f64 {
        self.phase + k * self.period()
    }
}

/// BPM as people write it: whole numbers when close, else one decimal.
pub fn fmt_bpm(bpm: f64) -> String {
    if (bpm - bpm.round()).abs() < 0.05 {
        format!("{}", bpm.round() as i64)
    } else {
        format!("{bpm:.1}")
    }
}

/// Preference for tempos people tend to count in (log-normal around 115).
fn prior(bpm: f64) -> f64 {
    let oct = (bpm / 115.0).log2();
    (-0.5 * (oct / 0.9).powi(2)).exp()
}

/// Autocorrelation of a curve after removing its moving average, for lags
/// `0..=max_lag`, normalised so lag 0 is 1.
fn autocorr(x: &[f32], max_lag: usize) -> Vec<f64> {
    let n = x.len();
    let mut out = vec![0.0; max_lag + 1];
    if n == 0 {
        return out;
    }
    for (lag, o) in out.iter_mut().enumerate() {
        if lag >= n {
            break;
        }
        let mut s = 0.0f64;
        for i in 0..n - lag {
            s += f64::from(x[i]) * f64::from(x[i + lag]);
        }
        *o = s / (n - lag) as f64;
    }
    let z = out[0];
    if z > 0.0 {
        for o in &mut out {
            *o /= z;
        }
    }
    out
}

fn interp(x: &[f64], pos: f64) -> f64 {
    if pos < 0.0 {
        return 0.0;
    }
    let i = pos.floor() as usize;
    if i + 1 >= x.len() {
        return x.last().copied().unwrap_or(0.0) * f64::from(u8::from(i < x.len()));
    }
    let f = pos - i as f64;
    x[i] * (1.0 - f) + x[i + 1] * f
}

fn interp32(x: &[f32], pos: f64) -> f64 {
    if pos < 0.0 {
        return 0.0;
    }
    let i = pos.floor() as usize;
    if i + 1 >= x.len() {
        return 0.0;
    }
    let f = pos - i as f64;
    f64::from(x[i]) * (1.0 - f) + f64::from(x[i + 1]) * f
}

/// Moving-average removal (window `w` frames) and half-wave rectification.
fn detrend(x: &[f32], w: usize) -> Vec<f32> {
    let n = x.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + f64::from(x[i]);
    }
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(w);
            let hi = (i + w + 1).min(n);
            let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (f64::from(x[i]) - mean).max(0.0) as f32
        })
        .collect()
}

/// Gaussian smoothing with standard deviation `sigma` frames.
fn smooth(x: &[f32], sigma: f64) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i64;
    let k: Vec<f64> = (-r..=r)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let norm: f64 = k.iter().sum();
    (0..x.len() as i64)
        .map(|i| {
            let mut s = 0.0;
            for (j, w) in k.iter().enumerate() {
                let idx = i + j as i64 - r;
                if idx >= 0 && (idx as usize) < x.len() {
                    s += f64::from(x[idx as usize]) * w;
                }
            }
            (s / norm) as f32
        })
        .collect()
}

/// Tempo salience (without the prior) at `bpm` from the autocorrelation.
fn salience(acf: &[f64], frame_rate: f64, bpm: f64) -> f64 {
    let lag = 60.0 * frame_rate / bpm;
    // The beat itself plus a little of the bar-level repetition.
    interp(acf, lag) + 0.25 * interp(acf, 2.0 * lag)
}

/// Estimates tempo, beat phase and confidence from audio.
pub fn estimate(samples: &[f32], sample_rate: u32) -> BeatEstimate {
    // Long files: the first minute is plenty and keeps this fast.
    let max = (sample_rate as usize) * 60;
    let samples = &samples[..samples.len().min(max)];
    let odf = onset_strength(samples, sample_rate);
    let onsets = pick_onsets(samples, &odf, 0.5);
    estimate_from(&odf, onsets, 4)
}

/// The beat phase (seconds, `0 ≤ phase < 60/bpm`) of a recording whose
/// tempo is already known, e.g. from the count-in or the command line.
pub fn phase_for_tempo(samples: &[f32], sample_rate: u32, bpm: f64) -> f64 {
    let odf = onset_strength(samples, sample_rate);
    let onsets = pick_onsets(samples, &odf, 0.5);
    let fr = odf.frame_rate();
    let period = 60.0 / bpm;
    if odf.is_empty() {
        return 0.0;
    }
    let curve = smooth(&odf.body, 0.008 * fr);
    let frames = period * fr;
    let mut best = (f64::NEG_INFINITY, 0.0);
    let mut phi = 0.0;
    while phi < frames {
        let mut s = 0.0;
        let mut k = 0.0;
        while phi + k * frames < curve.len() as f64 - 1.0 {
            s += interp32(&curve, phi + k * frames);
            k += 1.0;
        }
        if s > best.0 {
            best = (s, phi);
        }
        phi += 0.5;
    }
    let mut phase = odf.time_of(best.1);
    // Average the offsets of the hits that sit near beats.
    let (mut sw, mut sd) = (0.0, 0.0);
    for o in &onsets {
        let k = ((o.time - phase) / period).round();
        let d = o.time - (phase + k * period);
        if d.abs() < 0.25 * period.min(0.25) {
            let w = f64::from(o.strength).powi(2);
            sw += w;
            sd += w * d;
        }
    }
    if sw > 0.0 {
        phase += sd / sw;
    }
    phase.rem_euclid(period)
}

/// Estimates tempo from an onset-strength curve and its picked onsets.
/// `beats_per_bar` is used to guess where beat 1 falls.
pub fn estimate_from(odf: &Odf, onsets: Vec<Onset>, beats_per_bar: u32) -> BeatEstimate {
    let nothing = BeatEstimate {
        bpm: 120.0,
        phase: 0.0,
        downbeat: 0.0,
        confidence: 0.0,
        alternatives: Vec::new(),
        onsets: onsets.clone(),
    };
    let fr = odf.frame_rate();
    if onsets.len() < 4 || odf.len() < (fr * 1.0) as usize {
        return nothing;
    }

    // 2. Tempo by autocorrelation with a prior.
    let curve = detrend(&smooth(&odf.body, 0.006 * fr), (0.4 * fr) as usize);
    let max_lag = ((60.0 / 30.0) * fr) as usize;
    let acf = autocorr(&curve, max_lag.min(curve.len().saturating_sub(1)));
    let mut best = (f64::NEG_INFINITY, 120.0);
    let mut bpm = 40.0;
    while bpm <= 240.0 {
        let s = salience(&acf, fr, bpm) * prior(bpm);
        if s > best.0 {
            best = (s, bpm);
        }
        bpm += 0.25;
    }
    let coarse = best.1;

    // 3a. Joint refinement of tempo and phase with a comb over the curve.
    let comb_curve = smooth(&odf.body, 0.008 * fr);
    let (mut bpm, mut phase_frames) = comb_fit(&comb_curve, fr, coarse);
    // Octave check on the power curve (loud hits dominate): if every other
    // pulse is weak we are counting 8th notes, so count the strong ones;
    // if the grid misses many hits and the doubled pulse is even, double.
    let power = smooth(&odf.power, 0.008 * fr);
    for _ in 0..2 {
        let period = 60.0 * fr / bpm;
        let (even, odd) = pulse_means(&power, period, phase_frames);
        let ratio = even.min(odd) / even.max(odd).max(1e-9);
        let (de, dodd) = pulse_means(&power, period / 2.0, phase_frames);
        let ratio2 = de.min(dodd) / de.max(dodd).max(1e-9);
        let conf = grid_confidence(&onsets, period / fr, odf.time_of(phase_frames));
        let conf2 = grid_confidence(&onsets, period / fr / 2.0, odf.time_of(phase_frames));
        if ratio < HALVE_BELOW && bpm / 2.0 >= 40.0 {
            let strong_phase = if even >= odd {
                phase_frames
            } else {
                phase_frames + period
            };
            let (b, _) = comb_fit(&comb_curve, fr, bpm / 2.0);
            bpm = b;
            phase_frames = strong_phase.rem_euclid(60.0 * fr / bpm);
        } else if conf2 > conf + 0.25 && ratio2 >= DOUBLE_ABOVE && bpm * 2.0 <= 240.0 {
            let (b, p) = comb_fit(&comb_curve, fr, bpm * 2.0);
            bpm = b;
            phase_frames = p;
        } else {
            break;
        }
    }
    // 3b. Least-squares line through hits that sit on beats.
    let (bpm, phase) = regress_beats(&onsets, bpm, odf.time_of(phase_frames));

    let period = 60.0 / bpm;
    let confidence = confidence(&onsets, period, phase);

    // Alternatives: half and double, ordered by raw salience.
    let mut alternatives: Vec<(f64, f64)> = [bpm / 2.0, bpm * 2.0]
        .into_iter()
        .filter(|b| (25.0..=300.0).contains(b))
        .map(|b| (b, salience(&acf, fr, b)))
        .collect();
    alternatives.sort_by(|a, b| b.1.total_cmp(&a.1));

    // 5. Beat 1: the beat position with the most kick energy.
    let low = smooth(&odf.low, 0.008 * fr);
    let bpb = beats_per_bar.max(1) as usize;
    let mut energy = vec![0.0f64; bpb];
    let n_beats = ((odf.time_of(odf.len() as f64) - phase) / period)
        .floor()
        .max(0.0) as usize;
    for k in 0..=n_beats {
        let t = phase + k as f64 * period;
        energy[k % bpb] += interp32(&low, t * fr);
    }
    let first = energy
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i);
    let downbeat = phase + first as f64 * period;

    BeatEstimate {
        bpm: round_bpm(bpm),
        phase,
        downbeat,
        confidence,
        alternatives: alternatives.into_iter().map(|a| round_bpm(a.0)).collect(),
        onsets,
    }
}

/// Rounds to 0.01 BPM, and to a whole number when within 0.15.
fn round_bpm(bpm: f64) -> f64 {
    if (bpm - bpm.round()).abs() < 0.15 {
        bpm.round()
    } else {
        (bpm * 100.0).round() / 100.0
    }
}

/// Off-beats weaker than this fraction of the beats (in power) mean the
/// "beats" are really 8th notes. Measured on synthetic loops: 8th-note
/// trains alternate at 0.13–0.25 (0.5–0.6 with a syncopated kick), real
/// beat trains at 0.42 or more (four on the floor with snares on 2 and 4).
const HALVE_BELOW: f64 = 0.3;
/// Doubling needs the doubled pulse to be at least this even.
const DOUBLE_ABOVE: f64 = 0.45;

/// Mean curve value on even and odd pulses of a pulse train.
fn pulse_means(curve: &[f32], period: f64, phase: f64) -> (f64, f64) {
    let (mut sums, mut counts) = ([0.0f64; 2], [0usize; 2]);
    let mut k = 0usize;
    loop {
        let pos = phase + k as f64 * period;
        if pos >= curve.len() as f64 - 1.0 {
            break;
        }
        sums[k % 2] += interp32(curve, pos);
        counts[k % 2] += 1;
        k += 1;
    }
    let mean = |i: usize| {
        if counts[i] == 0 {
            0.0
        } else {
            sums[i] / counts[i] as f64
        }
    };
    (mean(0), mean(1))
}

/// Slides a comb of beat positions over `curve` for tempos within ±3% of
/// `bpm`; returns the best (bpm, phase in frames).
fn comb_fit(curve: &[f32], fr: f64, bpm: f64) -> (f64, f64) {
    let mut best = (f64::NEG_INFINITY, bpm, 0.0);
    let n = curve.len() as f64;
    let mut b = bpm * 0.97;
    while b <= bpm * 1.03 {
        let period = 60.0 * fr / b;
        let beats = ((n - 1.0) / period).floor() as usize;
        if beats >= 2 {
            let mut phi = 0.0;
            while phi < period {
                let mut s = 0.0;
                for k in 0..beats {
                    s += interp32(curve, phi + k as f64 * period);
                }
                let s = s / beats as f64;
                if s > best.0 {
                    best = (s, b, phi);
                }
                phi += 0.5;
            }
        }
        b += 0.05;
    }
    (best.1, best.2)
}

/// Fits `t = phase + k * period` through onsets within a quarter beat of a
/// predicted beat (weighted by strength). Returns (bpm, phase) with phase
/// folded into [0, period).
fn regress_beats(onsets: &[Onset], bpm: f64, phase: f64) -> (f64, f64) {
    let mut period = 60.0 / bpm;
    let mut phase = phase;
    for _ in 0..3 {
        let (mut sw, mut sk, mut st, mut skk, mut skt) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for o in onsets {
            let k = ((o.time - phase) / period).round();
            let err = o.time - (phase + k * period);
            if err.abs() > 0.25 * period.min(0.25) {
                continue;
            }
            let w = f64::from(o.strength).powi(2);
            sw += w;
            sk += w * k;
            st += w * o.time;
            skk += w * k * k;
            skt += w * k * o.time;
        }
        let den = sw * skk - sk * sk;
        if sw <= 0.0 || den.abs() < 1e-9 {
            break;
        }
        let p = (sw * skt - sk * st) / den;
        if !(p.is_finite() && p > 0.0 && (p / period - 1.0).abs() < 0.05) {
            break;
        }
        period = p;
        phase = (st - period * sk) / sw;
    }
    (60.0 / period, phase.rem_euclid(period))
}

/// How much of the hit strength lands near the 8th-note grid, rescaled so
/// random timing gives about 0 and perfect timing 1.
fn grid_confidence(onsets: &[Onset], period: f64, phase: f64) -> f64 {
    if onsets.len() < 4 {
        return 0.0;
    }
    let step = period / 2.0;
    let tol = 0.025f64.min(step * 0.2);
    let chance = (2.0 * tol / step).min(1.0);
    let (mut on, mut all) = (0.0, 0.0);
    for o in onsets {
        let w = f64::from(o.strength);
        let d = ((o.time - phase) / step).fract().abs();
        let d = d.min(1.0 - d) * step;
        if d <= tol {
            on += w;
        }
        all += w;
    }
    if all <= 0.0 {
        return 0.0;
    }
    let frac = on / all;
    // Few hits can line up by luck: scale down below 8 hits.
    let count = (onsets.len() as f64 / 8.0).min(1.0);
    (((frac - chance) / (1.0 - chance)).clamp(0.0, 1.0) * count).clamp(0.0, 1.0)
}

/// Fraction of the beats between the first and last hit that have a hit
/// within 35 ms. A steady groove fills (nearly) every beat; hits at random
/// times leave most beats empty even when a tempo is fitted to them.
fn coverage(onsets: &[Onset], period: f64, phase: f64) -> f64 {
    let (Some(first), Some(last)) = (onsets.first(), onsets.last()) else {
        return 0.0;
    };
    let tol = 0.035f64.min(period * 0.15);
    let k0 = ((first.time - phase) / period).ceil() as i64;
    let k1 = ((last.time - phase) / period).floor() as i64;
    if k1 < k0 {
        return 0.0;
    }
    let mut hit = 0;
    for k in k0..=k1 {
        let t = phase + k as f64 * period;
        if onsets.iter().any(|o| (o.time - t).abs() <= tol) {
            hit += 1;
        }
    }
    f64::from(hit) / (k1 - k0 + 1) as f64
}

/// Overall confidence: hits sit on the grid *and* the beats are filled.
fn confidence(onsets: &[Onset], period: f64, phase: f64) -> f64 {
    let grid = grid_confidence(onsets, period, phase);
    let cover = ((coverage(onsets, period, phase) - 0.3) / 0.5).clamp(0.0, 1.0);
    grid * cover
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{Rng, drum_loop, white_noise};

    /// The M2 acceptance test: synthetic loops from 60 to 180 BPM with
    /// ±15 ms jitter.
    #[test]
    fn tempo_and_phase_on_jittered_loops() {
        let mut in_range = 0;
        let mut in_range_ok = 0;
        let mut worst_phase: f64 = 0.0;
        let mut worst_err: f64 = 0.0;
        let mut min_conf: f64 = 1.0;
        let mut failures = Vec::new();
        let mut case = 0u64;
        let mut bpm = 60.0;
        while bpm <= 180.0 {
            for which in 0..3 {
                case += 1;
                let offset = 0.05 + 0.3 * ((case * 37) % 10) as f64 / 10.0;
                let (audio, hits) = drum_loop(bpm, 4, which, offset, 0.015, case, 48_000);
                let est = estimate(&audio, 48_000);
                let close = |b: f64| (b / bpm - 1.0).abs() <= 0.01;
                let ok = close(est.bpm);
                if (80.0..=160.0).contains(&bpm) {
                    in_range += 1;
                    in_range_ok += usize::from(ok);
                }
                if !ok && !est.alternatives.first().is_some_and(|&a| close(a)) {
                    failures.push(format!(
                        "{bpm} BPM pattern {which}: got {} alts {:?}",
                        est.bpm, est.alternatives
                    ));
                }
                if ok {
                    worst_err = worst_err.max((est.bpm / bpm - 1.0).abs());
                    min_conf = min_conf.min(est.confidence);
                    // Phase error against the true (unjittered) beat grid.
                    let true_phase = hits[0].time - hits[0].beat * 60.0 / bpm;
                    let p = 60.0 / bpm;
                    let true_phase = true_phase.rem_euclid(p);
                    let d = (est.phase - true_phase).rem_euclid(p);
                    let err = d.min(p - d);
                    worst_phase = worst_phase.max(err);
                    assert!(
                        est.is_sure(),
                        "{bpm} BPM pattern {which}: confidence {}",
                        est.confidence
                    );
                }
            }
            bpm += 5.0;
        }
        let rate = in_range_ok as f64 / in_range as f64;
        eprintln!(
            "tempo: {in_range_ok}/{in_range} within 1% in 80..160; worst tempo error {:.2}%; \
             worst phase error {:.1} ms; lowest confidence {min_conf:.2}",
            worst_err * 100.0,
            worst_phase * 1000.0
        );
        assert!(rate >= 0.9, "only {in_range_ok}/{in_range} in range");
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(worst_phase < 0.030, "phase error {worst_phase}");
    }

    #[test]
    fn noise_and_pulseless_loops_are_not_sure() {
        let noise = white_noise(48_000 * 6, 0.3, 3);
        let e = estimate(&noise, 48_000);
        eprintln!("noise: confidence {:.2} ({})", e.confidence, e.describe());
        assert!(!e.is_sure(), "noise: {}", e.confidence);

        // Hits at random times: no pulse.
        for seed in 17..33 {
            let mut rng = Rng::new(seed);
            let mut audio = vec![0.0f32; 48_000 * 8];
            let mut t = 0.1;
            while t < 7.5 {
                let hit = crate::kit::drum_hit(crate::kit::Sound::Snare, 0.8, 48_000, 1);
                let at = (t * 48_000.0) as usize;
                for (k, v) in hit.iter().enumerate() {
                    if let Some(o) = audio.get_mut(at + k) {
                        *o += v;
                    }
                }
                t += 0.08 + 0.45 * f64::from(rng.bipolar().abs());
            }
            let e = estimate(&audio, 48_000);
            eprintln!(
                "random hits {seed}: confidence {:.2} ({})",
                e.confidence,
                e.describe()
            );
            assert!(
                !e.is_sure(),
                "random hits: {} ({})",
                e.confidence,
                e.describe()
            );
        }

        let silence = estimate(&vec![0.0; 48_000 * 3], 48_000);
        assert_eq!(silence.confidence, 0.0);
        assert_eq!(silence.describe(), "no steady beat found");
    }

    #[test]
    fn phase_for_a_known_tempo() {
        let (audio, hits) = drum_loop(96.0, 4, 1, 0.37, 0.01, 8, 48_000);
        let p = phase_for_tempo(&audio, 48_000, 96.0);
        let period = 60.0 / 96.0;
        let truth = (hits[0].time - hits[0].beat * period).rem_euclid(period);
        let d = (p - truth).rem_euclid(period);
        assert!(d.min(period - d) < 0.01, "phase {p} vs {truth}");
    }

    #[test]
    fn plain_words() {
        let mut e = BeatEstimate {
            bpm: 118.0,
            phase: 0.0,
            downbeat: 0.0,
            confidence: 0.9,
            alternatives: vec![59.0, 236.0],
            onsets: Vec::new(),
        };
        assert_eq!(e.describe(), "118 BPM, sure");
        e.confidence = 0.3;
        assert_eq!(e.describe(), "118 BPM or 59? not sure");
        assert_eq!(fmt_bpm(96.5), "96.5");
    }

    #[test]
    fn downbeat_falls_on_the_kick() {
        let (audio, hits) = drum_loop(100.0, 4, 0, 0.4, 0.0, 2, 48_000);
        let e = estimate(&audio, 48_000);
        assert!((e.bpm - 100.0).abs() < 1.0);
        let bar = 4.0 * 60.0 / 100.0;
        let d = (e.downbeat - hits[0].time).rem_euclid(bar);
        // Pattern 0 has kicks on 1 and 3, so either is a fair "beat 1".
        let half = bar / 2.0;
        let d = d.rem_euclid(half);
        assert!(
            d.min(half - d) < 0.03,
            "downbeat {} vs {}",
            e.downbeat,
            hits[0].time
        );
    }
}
