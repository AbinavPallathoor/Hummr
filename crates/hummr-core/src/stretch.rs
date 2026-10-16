//! Time-stretching and pitch-shifting.

use crate::resample::resample_positions;

/// Makes `input` `ratio` times longer and shifts it by `semitones`.
///
/// Placeholder until WSOLA lands: varispeed (pitch follows speed).
pub fn stretch_and_shift(input: &[f32], _sample_rate: u32, ratio: f64, semitones: f64) -> Vec<f32> {
    if (ratio - 1.0).abs() < 1e-9 && semitones.abs() < 1e-9 {
        return input.to_vec();
    }
    let n = (input.len() as f64 * ratio).round() as usize;
    resample_positions(input, n, |i| i as f64 / ratio, ratio.min(1.0))
}
