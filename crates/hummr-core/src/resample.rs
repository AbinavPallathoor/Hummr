//! Band-limited sample-rate conversion with a windowed-sinc kernel.
//!
//! The kernel is a sinc low-pass at just under the lower of the two Nyquist
//! frequencies, shaped by a Blackman window over 16 zero crossings on each
//! side. It is tabulated at 512 sub-sample phases and linearly interpolated
//! between them, which keeps the error far below 16-bit noise while making
//! conversion cheap enough to run on every import.

use std::f64::consts::PI;

/// Zero crossings of the sinc on each side of the centre tap.
const ZERO_CROSSINGS: usize = 16;
/// Kernel table resolution per input sample.
const PHASES: usize = 512;

fn blackman(x: f64) -> f64 {
    // x in [-1, 1]
    if x.abs() >= 1.0 {
        return 0.0;
    }
    let t = (x + 1.0) * 0.5;
    0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos()
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

struct Kernel {
    /// `cutoff` as a fraction of the input Nyquist frequency.
    cutoff: f64,
    /// Half width in input samples.
    half_width: f64,
    table: Vec<f32>,
}

impl Kernel {
    fn new(cutoff: f64) -> Self {
        let half_width = ZERO_CROSSINGS as f64 / cutoff;
        let n = (half_width * PHASES as f64).ceil() as usize + 2;
        let table = (0..n)
            .map(|i| {
                let t = i as f64 / PHASES as f64;
                (cutoff * sinc(cutoff * t) * blackman(t / half_width)) as f32
            })
            .collect();
        Self {
            cutoff,
            half_width,
            table,
        }
    }

    /// Kernel value at distance `t` (input samples) from the centre.
    #[inline]
    fn at(&self, t: f64) -> f32 {
        let pos = t.abs() * PHASES as f64;
        let i = pos as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let frac = (pos - i as f64) as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * frac
    }
}

/// Resamples `input` from `from` Hz to `to` Hz. The output has exactly
/// `len * to / from` samples (rounded down).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let n_out = (input.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    resample_positions(input, n_out, |i| i as f64 * step, (1.0 / step).min(1.0))
}

/// Reads `input` at fractional positions given by `pos(i)` for `i` in
/// `0..n_out`, band-limited to `bandwidth` (fraction of input Nyquist, so 1.0
/// for upsampling and `to / from` for downsampling). Used for resampling and
/// for varispeed playback.
pub fn resample_positions(
    input: &[f32],
    n_out: usize,
    pos: impl Fn(usize) -> f64,
    bandwidth: f64,
) -> Vec<f32> {
    // Leave a little room below Nyquist for the transition band.
    let kernel = Kernel::new((bandwidth * 0.97).clamp(0.01, 0.97));
    let hw = kernel.half_width;
    let len = input.len() as i64;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let x = pos(i);
        let lo = ((x - hw).ceil() as i64).max(0);
        let hi = ((x + hw).floor() as i64).min(len - 1);
        let mut acc = 0.0f32;
        let mut j = lo;
        while j <= hi {
            acc += input[j as usize] * kernel.at(x - j as f64);
            j += 1;
        }
        out.push(acc);
    }
    debug_assert!(kernel.cutoff > 0.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;
    use crate::pitch::yin::yin_pitch;
    use crate::synthetic::sine;

    #[test]
    fn length_is_exact() {
        let x = vec![0.0f32; 48_000];
        assert_eq!(resample(&x, 48_000, 44_100).len(), 44_100);
        assert_eq!(resample(&x, 48_000, 16_000).len(), 16_000);
        assert_eq!(resample(&x[..22_050], 44_100, 48_000).len(), 24_000);
    }

    #[test]
    fn keeps_pitch_and_level_of_an_in_band_tone() {
        for (from, to) in [(44_100, 48_000), (48_000, 44_100), (22_050, 48_000)] {
            let x = sine(1_000.0, from, 0.5, 0.5);
            let y = resample(&x, from, to);
            let hz = yin_pitch(&y[2_000..6_000], to, 60.0, 2_000.0, 0.15).unwrap();
            assert!((hz - 1_000.0).abs() < 1.0, "{from}->{to}: {hz} Hz");
            let level = rms(&y[1_000..y.len() - 1_000]) / rms(&x[1_000..x.len() - 1_000]);
            assert!((level - 1.0).abs() < 0.01, "{from}->{to}: gain {level}");
        }
    }

    #[test]
    fn removes_content_above_the_new_nyquist() {
        // 15 kHz cannot exist at 22.05 kHz; it must not alias down.
        let x = sine(15_000.0, 48_000, 0.5, 0.5);
        let y = resample(&x, 48_000, 22_050);
        let level = rms(&y[500..y.len() - 500]);
        assert!(level < 0.5 * 0.707 * 0.01, "alias level {level}");
    }
}
