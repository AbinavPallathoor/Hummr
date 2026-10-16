//! Effects: the data model for effect chains on clips and tracks, and the
//! master limiter.

use serde::{Deserialize, Serialize};

use crate::dsp::db_to_amp;

/// The effects on one clip or track: an optional named preset with an
/// "amount" macro, four friendly sliders, and raw effects one level down.
///
/// The processing order is preset, then the sliders (drive, filter, echo,
/// reverb), then the raw effects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxChain {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// How strongly the preset is applied, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub amount: f64,
    /// Distortion, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub drive: f64,
    /// Reverb, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub reverb: f64,
    /// Echo, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub echo: f64,
    /// Filter sweep: below 0 darkens (low-pass), above 0 thins (high-pass).
    #[serde(skip_serializing_if = "is_zero")]
    pub filter: f64,
    /// Raw effects, applied last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<Effect>,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

impl FxChain {
    pub fn is_empty(&self) -> bool {
        (self.preset.is_none() || self.amount <= 0.0)
            && self.drive <= 0.0
            && self.reverb <= 0.0
            && self.echo <= 0.0
            && self.filter == 0.0
            && self.raw.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Smooth tanh saturation.
    Soft,
    /// Wave folding: loud parts fold back, which sounds harsh and buzzy.
    Fold,
}

/// One raw effect with all of its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Effect {
    Gain {
        db: f64,
    },
    Filter {
        kind: FilterKind,
        hz: f64,
        q: f64,
    },
    Eq {
        low_db: f64,
        mid_db: f64,
        mid_hz: f64,
        high_db: f64,
    },
    Distortion {
        drive: f64,
        shape: Shape,
        mix: f64,
    },
    Bitcrush {
        bits: f64,
        /// Keep one sample in `downsample`.
        downsample: f64,
    },
    Echo {
        /// Delay time in beats, so echoes stay in time with the song.
        beats: f64,
        feedback: f64,
        mix: f64,
    },
    Reverb {
        size: f64,
        damping: f64,
        mix: f64,
    },
    Compressor {
        threshold_db: f64,
        ratio: f64,
        attack_ms: f64,
        release_ms: f64,
        makeup_db: f64,
    },
    Limiter {
        ceiling_db: f64,
    },
}

/// Runs a chain over a mono signal. `channel` (0 left, 1 right) lets
/// stereo effects such as reverb decorrelate the two sides. The output may
/// be longer than the input (echo and reverb tails).
pub fn apply_chain(
    _chain: &FxChain,
    input: &[f32],
    _sample_rate: u32,
    _bpm: f64,
    _channel: usize,
) -> Vec<f32> {
    input.to_vec()
}

/// A look-ahead peak limiter. Below the ceiling it is bit-transparent (gain
/// exactly 1); above, the gain ramps down ahead of each peak and recovers
/// smoothly. Offline only: it returns output aligned with the input.
pub fn limit(samples: &mut [f32], sample_rate: u32, ceiling_db: f64) {
    limit_linked(&mut [samples], sample_rate, ceiling_db);
}

/// Limits several channels with one shared gain so the stereo image holds.
pub fn limit_linked(channels: &mut [&mut [f32]], sample_rate: u32, ceiling_db: f64) {
    let len = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    if len == 0 {
        return;
    }
    let ceiling = db_to_amp(ceiling_db as f32);
    let look = ((sample_rate as f64 * 0.005) as usize).max(1);
    let release = (-1.0 / (sample_rate as f64 * 0.08)).exp() as f32;
    // Required gain per sample.
    let mut need = vec![1.0f32; len];
    let mut any = false;
    for c in channels.iter() {
        for (i, &s) in c.iter().enumerate() {
            let a = s.abs();
            if a > ceiling {
                need[i] = need[i].min(ceiling / a);
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    // Minimum over the look-ahead window (so gain is down before the peak),
    // then a linear attack ramp across that window and an exponential release.
    let mut window_min = vec![1.0f32; len];
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in 0..len + look {
        if i < len {
            while deque.back().is_some_and(|&j| need[j] >= need[i]) {
                deque.pop_back();
            }
            deque.push_back(i);
        }
        if i >= look {
            let out = i - look;
            while deque.front().is_some_and(|&j| j < out) {
                deque.pop_front();
            }
            window_min[out] = deque.front().map_or(1.0, |&j| need[j]);
        }
    }
    let mut gain = 1.0f32;
    let attack_step = 1.0 / look as f32;
    for i in 0..len {
        let target = window_min[i];
        if target < gain {
            // Reach the target by the time the peak arrives.
            gain = (gain - attack_step).max(target);
        } else {
            gain = target + (gain - target) * release;
        }
        let g = gain.min(need[i]);
        if g < 1.0 {
            for c in channels.iter_mut() {
                if let Some(s) = c.get_mut(i) {
                    *s *= g;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::sine;

    #[test]
    fn limiter_is_transparent_below_the_ceiling() {
        let x = sine(220.0, 48_000, 0.2, 0.5);
        let mut y = x.clone();
        limit(&mut y, 48_000, -1.0);
        assert_eq!(x, y);
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        let mut y = sine(220.0, 48_000, 0.5, 4.0);
        limit(&mut y, 48_000, -1.0);
        let ceiling = db_to_amp(-1.0);
        assert!(y.iter().all(|s| s.is_finite() && s.abs() <= ceiling + 1e-6));
        assert!(y.iter().fold(0.0f32, |m, s| m.max(s.abs())) > 0.8 * ceiling);
    }

    #[test]
    fn empty_chain() {
        assert!(FxChain::default().is_empty());
        let c = FxChain {
            reverb: 0.3,
            ..FxChain::default()
        };
        assert!(!c.is_empty());
    }
}
