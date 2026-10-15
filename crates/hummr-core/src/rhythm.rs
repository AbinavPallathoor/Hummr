//! Quantisation grids and tempo detection.

use std::f64::consts::TAU;
use std::fmt;
use std::str::FromStr;

use crate::{Error, Result};

/// A quantisation grid such as `1/8` or `1/16t` (triplet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub numerator: u32,
    pub denominator: u32,
    pub triplet: bool,
}

impl Grid {
    pub const EIGHTH: Grid = Grid {
        numerator: 1,
        denominator: 8,
        triplet: false,
    };

    /// Grid step in beats (quarter notes).
    pub fn beats(&self) -> f64 {
        let b = 4.0 * f64::from(self.numerator) / f64::from(self.denominator);
        if self.triplet { b * 2.0 / 3.0 } else { b }
    }

    /// Parses `off`/`none` as `None`, otherwise a grid.
    pub fn parse_optional(s: &str) -> Result<Option<Grid>> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "no" | "0" => Ok(None),
            _ => s.parse().map(Some),
        }
    }
}

impl FromStr for Grid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let hint = "expected a note length like 1/4, 1/8, 1/16 or 1/8t (triplet), or 'off'";
        let t = s.trim().to_ascii_lowercase();
        let (body, triplet) = match t.strip_suffix('t') {
            Some(b) => (b, true),
            None => (t.as_str(), false),
        };
        let (num, den) = match body.split_once('/') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (body, "1"),
        };
        let numerator: u32 = num.parse().map_err(|_| Error::parse("grid", s, hint))?;
        let denominator: u32 = den.parse().map_err(|_| Error::parse("grid", s, hint))?;
        if numerator == 0 || denominator == 0 || denominator > 128 || !denominator.is_power_of_two()
        {
            return Err(Error::parse("grid", s, hint));
        }
        Ok(Grid {
            numerator,
            denominator,
            triplet,
        })
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)?;
        if self.triplet {
            write!(f, "t")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoEstimate {
    pub bpm: f64,
    /// Time in seconds of one grid line; the grid is `phase + k * step`.
    pub phase: f64,
    /// How tightly onsets sit on the grid, 0 (random) ..= 1 (perfect).
    pub fit: f64,
}

impl TempoEstimate {
    pub fn grid_step_seconds(&self, grid_beats: f64) -> f64 {
        grid_beats * 60.0 / self.bpm
    }
}

/// Weighted circular mean of onset phases on a grid with period `step`:
/// returns (resultant length 0..=1, phase in seconds).
pub fn grid_alignment(onsets: &[f64], weights: &[f64], step: f64) -> (f64, f64) {
    let (mut c, mut s, mut wsum) = (0.0, 0.0, 0.0);
    for (i, &t) in onsets.iter().enumerate() {
        let w = weights.get(i).copied().unwrap_or(1.0);
        let theta = TAU * (t / step);
        c += w * theta.cos();
        s += w * theta.sin();
        wsum += w;
    }
    if wsum == 0.0 {
        return (0.0, 0.0);
    }
    let r = (c * c + s * s).sqrt() / wsum;
    let phase = (s.atan2(c) / TAU * step).rem_euclid(step);
    (r, phase)
}

/// Mild preference for moderate tempos so that the octave ambiguity
/// (100 BPM eighths vs 50 BPM sixteenths) resolves the way people count.
fn tempo_prior(bpm: f64) -> f64 {
    let octaves = (bpm / 105.0).log2();
    (-0.5 * (octaves / 0.75).powi(2)).exp()
}

/// Finds the tempo whose `grid_beats` grid best fits the note onsets.
///
/// Uses the circular resultant length of onset phases, which naturally
/// penalises grids that are too fine (timing jitter becomes a larger fraction
/// of the step) and grids that are too coarse (onsets fall between lines).
pub fn detect_tempo(
    onsets: &[f64],
    weights: &[f64],
    grid_beats: f64,
    min_bpm: f64,
    max_bpm: f64,
) -> Option<TempoEstimate> {
    if onsets.len() < 4 || grid_beats <= 0.0 || min_bpm >= max_bpm {
        return None;
    }
    let score_at = |bpm: f64| {
        let (r, phase) = grid_alignment(onsets, weights, grid_beats * 60.0 / bpm);
        (r * tempo_prior(bpm), r, phase)
    };
    let mut best = (f64::NEG_INFINITY, 0.0, 0.0, 0.0);
    let mut bpm = min_bpm;
    while bpm <= max_bpm {
        let (score, r, phase) = score_at(bpm);
        if score > best.0 {
            best = (score, bpm, r, phase);
        }
        bpm += 0.25;
    }
    // Refine around the coarse optimum.
    let centre = best.1;
    let mut b = (centre - 0.25).max(min_bpm);
    while b <= (centre + 0.25).min(max_bpm) {
        let (score, r, phase) = score_at(b);
        if score > best.0 {
            best = (score, b, r, phase);
        }
        b += 0.01;
    }
    // Whole-number tempos are friendlier for everything downstream; take one
    // if it fits nearly as well.
    let whole = best.1.round();
    if (min_bpm..=max_bpm).contains(&whole) {
        let (score, r, phase) = score_at(whole);
        if score >= 0.97 * best.0 {
            best = (score, whole, r, phase);
        }
    }
    Some(TempoEstimate {
        bpm: (best.1 * 100.0).round() / 100.0,
        phase: best.3,
        fit: best.2,
    })
}

/// Grid alignment for a fixed tempo.
pub fn fit_fixed_tempo(
    onsets: &[f64],
    weights: &[f64],
    grid_beats: f64,
    bpm: f64,
) -> TempoEstimate {
    let (fit, phase) = grid_alignment(onsets, weights, grid_beats * 60.0 / bpm);
    TempoEstimate { bpm, phase, fit }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::Rng;

    #[test]
    fn parse_grids() {
        assert_eq!("1/8".parse::<Grid>().unwrap().beats(), 0.5);
        assert_eq!("1/4".parse::<Grid>().unwrap().beats(), 1.0);
        assert!(("1/8t".parse::<Grid>().unwrap().beats() - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!("1".parse::<Grid>().unwrap().beats(), 4.0);
        assert_eq!(Grid::parse_optional("off").unwrap(), None);
        assert!("1/7".parse::<Grid>().is_err());
        assert!("eighth".parse::<Grid>().is_err());
        assert_eq!("1/16t".parse::<Grid>().unwrap().to_string(), "1/16t");
    }

    fn onsets_from_beats(beats: &[f64], bpm: f64, offset: f64, jitter: f64, seed: u64) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        beats
            .iter()
            .map(|b| offset + b * 60.0 / bpm + f64::from(rng.bipolar()) * jitter)
            .collect()
    }

    #[test]
    fn detects_tempo_of_jittered_eighths_and_quarters() {
        // Mixed rhythm: quarters and eighths.
        let mut beats = Vec::new();
        let mut b = 0.0;
        for len in [
            1.0, 0.5, 0.5, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5, 2.0, 1.0, 1.0, 0.5, 0.5, 1.0,
        ] {
            beats.push(b);
            b += len;
        }
        for &bpm in &[84.0, 96.0, 110.0, 128.0] {
            let onsets = onsets_from_beats(&beats, bpm, 0.37, 0.015, 5);
            let w = vec![1.0; onsets.len()];
            let est = detect_tempo(&onsets, &w, 0.5, 70.0, 160.0).unwrap();
            assert!(
                (est.bpm - bpm).abs() < 1.5,
                "expected {bpm}, got {}",
                est.bpm
            );
            assert!(est.fit > 0.8);
        }
    }

    #[test]
    fn phase_matches_first_onset() {
        let onsets = onsets_from_beats(&[0.0, 1.0, 1.5, 2.0, 3.0, 4.0], 100.0, 0.42, 0.0, 1);
        let est = fit_fixed_tempo(&onsets, &[], 0.5, 100.0);
        let step = 0.3;
        let d = ((0.42 - est.phase) / step).fract().abs();
        assert!(d < 1e-6 || (1.0 - d) < 1e-6, "phase {}", est.phase);
        assert!(est.fit > 0.999);
    }

    #[test]
    fn too_few_onsets() {
        assert!(detect_tempo(&[0.0, 0.5], &[], 0.5, 70.0, 160.0).is_none());
    }
}
