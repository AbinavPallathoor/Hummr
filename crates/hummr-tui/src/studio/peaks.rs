//! Per-column waveform levels for clip boxes, cached per clip and zoom.
//!
//! Levels come straight from the source audio (cheap, no rendering): audio
//! clips map each column back to their source region, sliced clips to the
//! slice that plays there, and notes clips draw their notes as blocks. A
//! clip's rendered sound (when the playback engine has one) is used
//! instead, so effects and conversions show up.

use std::collections::HashMap;

use hummr_core::AudioBuffer;
use hummr_core::project::{Clip, ClipKind, SourceBank};

/// Block glyphs from quiet to loud.
pub const GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Levels at or below this (dBFS) draw as the lowest block.
const FLOOR_DB: f32 = -48.0;

/// Glyph for a peak level (linear, 0..1).
pub fn glyph(peak: f32) -> char {
    let db = 20.0 * peak.max(1e-9).log10();
    let frac = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    GLYPHS[((frac * 7.0).round() as usize).min(7)]
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    clip: Clip,
    bpm: f64,
    cols_per_beat: f64,
    rendered: bool,
}

/// Cached levels, keyed by clip id.
#[derive(Default)]
pub struct PeakCache {
    entries: HashMap<u64, (Key, Vec<f32>)>,
    /// Rendered clip audio (mono, at the project rate), by clip id, with
    /// the clip it was rendered from.
    pub rendered: HashMap<u64, (Clip, f64, std::sync::Arc<Vec<f32>>)>,
}

impl PeakCache {
    /// Levels for each column of `clip` (column 0 = the clip's start).
    pub fn levels(
        &mut self,
        clip: &Clip,
        bpm: f64,
        cols_per_beat: f64,
        bank: &SourceBank,
    ) -> &[f32] {
        let mut placed = clip.clone();
        placed.start_beat = 0.0;
        let rendered = self
            .rendered
            .get(&clip.id)
            .filter(|(c, b, _)| {
                let mut c = c.clone();
                c.start_beat = 0.0;
                c == placed && *b == bpm
            })
            .map(|(_, _, a)| a.clone());
        let key = Key {
            clip: placed,
            bpm,
            cols_per_beat,
            rendered: rendered.is_some(),
        };
        let fresh = self.entries.get(&clip.id).is_none_or(|(k, _)| *k != key);
        if fresh {
            let v = match &rendered {
                Some(a) => from_samples(a, clip, bpm, cols_per_beat),
                None => compute(clip, bpm, cols_per_beat, bank),
            };
            self.entries.insert(clip.id, (key, v));
        }
        &self.entries[&clip.id].1
    }

    /// Forgets clips that no longer exist.
    pub fn retain(&mut self, ids: &[u64]) {
        self.entries.retain(|id, _| ids.contains(id));
        self.rendered.retain(|id, _| ids.contains(id));
    }
}

fn columns(clip: &Clip, bpm: f64, cols_per_beat: f64) -> usize {
    (clip.len_beats(bpm) * cols_per_beat).ceil().max(1.0) as usize
}

fn peak(samples: &[f32], from: f64, to: f64, sr: f64) -> f32 {
    let a = (from * sr).max(0.0) as usize;
    let b = ((to * sr).max(0.0) as usize).min(samples.len());
    if a >= b {
        return 0.0;
    }
    // At most ~2000 reads per column.
    let step = ((b - a) / 2_000).max(1);
    samples[a..b]
        .iter()
        .step_by(step)
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

fn from_samples(samples: &[f32], clip: &Clip, bpm: f64, cols_per_beat: f64) -> Vec<f32> {
    let spb = 60.0 / bpm;
    let sr = f64::from(hummr_core::project::PROJECT_RATE);
    (0..columns(clip, bpm, cols_per_beat))
        .map(|c| {
            let b0 = c as f64 / cols_per_beat;
            peak(samples, b0 * spb, (b0 + 1.0 / cols_per_beat) * spb, sr)
        })
        .collect()
}

fn compute(clip: &Clip, bpm: f64, cols_per_beat: f64, bank: &SourceBank) -> Vec<f32> {
    let spb = 60.0 / bpm;
    let n = columns(clip, bpm, cols_per_beat);
    let gain = 10f32.powf(clip.gain_db as f32 / 20.0);
    let empty = AudioBuffer::new(Vec::new(), hummr_core::project::PROJECT_RATE);
    let audio = bank.get(&clip.source).map_or(&empty, |a| a.as_ref());
    let sr = f64::from(audio.sample_rate);
    let unit = clip.unit_beats(bpm).max(1e-6);
    let col_beats = 1.0 / cols_per_beat;
    (0..n)
        .map(|c| {
            let b0 = c as f64 * col_beats;
            let level = match &clip.kind {
                ClipKind::Audio => {
                    let u0 = b0 % unit;
                    let u1 = (u0 + col_beats).min(unit);
                    let (mut t0, mut t1) =
                        (u0 / unit * clip.source_len, u1 / unit * clip.source_len);
                    if clip.reverse {
                        (t0, t1) = (clip.source_len - t1, clip.source_len - t0);
                    }
                    peak(
                        &audio.samples,
                        clip.source_start + t0,
                        clip.source_start + t1,
                        sr,
                    )
                }
                ClipKind::Sliced { slices, .. } => {
                    let u0 = b0 % unit;
                    let u1 = u0 + col_beats;
                    slices
                        .iter()
                        .filter_map(|s| {
                            let end = s.beat + s.len / spb;
                            let lo = u0.max(s.beat);
                            let hi = u1.min(end);
                            (lo < hi).then(|| {
                                peak(
                                    &audio.samples,
                                    s.src + (lo - s.beat) * spb,
                                    s.src + (hi - s.beat) * spb,
                                    sr,
                                )
                            })
                        })
                        .fold(0.0, f32::max)
                }
                ClipKind::Notes { notes, .. } => {
                    let u0 = b0 % unit;
                    let u1 = u0 + col_beats;
                    notes
                        .iter()
                        .filter(|n| n.beat < u1 && n.beat + n.len.max(0.05) > u0)
                        .map(|n| {
                            // Louder at the start of a note, like a hit.
                            let fresh = n.beat >= u0 - 1e-9;
                            let v = f32::from(n.vel) / 127.0;
                            if fresh { v * 0.7 } else { v * 0.35 }
                        })
                        .fold(0.0, f32::max)
                }
            };
            level * gain
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hummr_core::project::PROJECT_RATE;
    use std::sync::Arc;

    #[test]
    fn glyph_scale() {
        assert_eq!(glyph(0.0), '▁');
        assert_eq!(glyph(1.0), '█');
        assert_eq!(glyph(0.004), '▁');
        assert!(GLYPHS.contains(&glyph(0.1)));
    }

    #[test]
    fn audio_levels_follow_the_source() {
        // One beat of silence, then one loud beat (120 BPM → 0.5 s each).
        let mut x = vec![0.0f32; 24_000];
        x.extend(vec![0.8f32; 24_000]);
        let mut bank = SourceBank::default();
        bank.insert("s", Arc::new(AudioBuffer::new(x, PROJECT_RATE)));
        let clip = Clip::audio(1, "c", "s", 0.0, 1.0, 3.0);
        let mut cache = PeakCache::default();
        let v = cache.levels(&clip, 120.0, 4.0, &bank).to_vec();
        assert_eq!(v.len(), 8);
        assert!(v[..4].iter().all(|&l| l == 0.0));
        assert!(v[4..].iter().all(|&l| (l - 0.8).abs() < 1e-6));
        // Reversed: loud first.
        let mut r = clip.clone();
        r.reverse = true;
        let v = cache.levels(&r, 120.0, 4.0, &bank).to_vec();
        assert!(v[0] > 0.5 && v[7] == 0.0);
    }
}
