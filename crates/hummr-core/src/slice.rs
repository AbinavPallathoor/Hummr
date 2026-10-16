//! Slicing a clip at its hits and placing the slices on a beat grid.

use crate::AudioBuffer;
use crate::project::Slice;
use crate::project::render::{fade_in, fade_out, region};

/// Crossfade length at each cut (seconds).
pub const XFADE: f64 = 0.005;

/// Renders slices (positions in beats from the clip start) into one mono
/// buffer. Neighbouring slices overlap by [`XFADE`] with complementary
/// linear fades centred on the cut, so slices left at their original
/// positions sum back to the original audio exactly. A slice that would run
/// into the next one (after a tempo increase or snapping) is cut short there.
pub fn render_slices(
    audio: &AudioBuffer,
    slices: &[Slice],
    seconds_per_beat: f64,
    semitones: f64,
    reverse: bool,
) -> Vec<f32> {
    let sr = f64::from(audio.sample_rate);
    let h = (XFADE * sr / 2.0).round() as i64; // half the crossfade, samples
    let mut order: Vec<&Slice> = slices.iter().collect();
    order.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let mut out: Vec<f32> = Vec::new();
    for (i, s) in order.iter().enumerate() {
        let first = i == 0;
        let last = i + 1 == order.len();
        let at = (s.beat * seconds_per_beat * sr).round() as i64;
        let mut len = (s.len * sr).round() as i64;
        let mut cut_short = false;
        if let Some(next) = order.get(i + 1) {
            let next_at = (next.beat * seconds_per_beat * sr).round() as i64;
            if next_at - at < len {
                len = (next_at - at).max(0);
                cut_short = true;
            }
        }
        if len == 0 {
            continue;
        }
        let pre = if first { 0 } else { h };
        let post = if last && !cut_short { 0 } else { h };
        let src0 = (s.src * sr).round() as i64 - pre;
        let n = (len + pre + post) as usize;
        let mut seg = region(audio, src0 as f64 / sr, n as f64 / sr);
        seg.resize(n, 0.0);
        if reverse {
            seg[pre as usize..(pre + len) as usize].reverse();
        }
        if semitones != 0.0 {
            seg = crate::stretch::stretch_and_shift(&seg, audio.sample_rate, 1.0, semitones);
            seg.resize(n, 0.0);
        }
        if pre > 0 {
            fade_in(&mut seg, (2 * pre) as usize);
        }
        if post > 0 {
            fade_out(&mut seg, (2 * post) as usize);
        }
        let start = at - pre;
        let end = start + n as i64;
        if end > out.len() as i64 {
            out.resize(end.max(0) as usize, 0.0);
        }
        for (k, &x) in seg.iter().enumerate() {
            let j = start + k as i64;
            if j >= 0 {
                out[j as usize] += x;
            }
        }
    }
    out
}
