//! Note segmentation: pitch track → discrete notes.
//!
//! 1. Frames are "loud" if their RMS is within `silence_db_below_peak` of
//!    the recording's typical peak level.
//! 2. A global tuning offset is estimated (people often sing consistently
//!    sharp or flat) and removed.
//! 3. A note-level HMM (one state per semitone plus silence, Viterbi decoded)
//!    splits on pitch changes. Its Gaussian emission is wide and switching
//!    notes is expensive, so vibrato and scoops stay inside one note while a
//!    real step to a new pitch is picked up within ~40 ms.
//! 4. Each note is split again at RMS energy dips, which is how repeated
//!    notes at the same pitch ("da da da") are separated.
//! 5. Very short fragments are merged into a neighbour or dropped.

use crate::dsp::{amp_to_db, median, quantile};
use crate::pitch::PitchTrack;
use crate::theory::hz_to_midi;

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentParams {
    pub min_note_seconds: f64,
    pub silence_db_below_peak: f32,
    pub absolute_floor_db: f32,
    /// Energy dip (dB below both neighbouring peaks) that splits a note.
    pub split_dip_db: f32,
    /// The dip must fall and recover within this time on each side.
    pub split_dip_seconds: f64,
    /// Width of the pitch emission in semitones.
    pub pitch_sigma: f32,
    /// Per-frame probability of moving directly to another note.
    pub change_prob: f64,
    pub correct_tuning: bool,
    pub lowest_note: u8,
    pub highest_note: u8,
}

impl Default for SegmentParams {
    fn default() -> Self {
        Self {
            min_note_seconds: 0.07,
            silence_db_below_peak: 30.0,
            absolute_floor_db: -60.0,
            split_dip_db: 6.0,
            split_dip_seconds: 0.08,
            pitch_sigma: 0.6,
            change_prob: 1e-4,
            correct_tuning: true,
            lowest_note: 33,
            highest_note: 96,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectedNote {
    pub start: f64,
    pub end: f64,
    /// Median sung pitch after tuning correction (fractional MIDI).
    pub sung_pitch: f32,
    /// Median sung pitch as recorded (fractional MIDI).
    pub raw_pitch: f32,
    pub peak_db: f32,
    pub velocity: u8,
    pub voiced_fraction: f32,
}

impl DetectedNote {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    pub fn nearest_pitch(&self) -> u8 {
        self.sung_pitch.round().clamp(0.0, 127.0) as u8
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Discarded {
    /// Too short, absorbed into the neighbouring note at this index.
    Merged {
        fragment: DetectedNote,
        into: usize,
    },
    TooShort(DetectedNote),
    MostlyUnvoiced(DetectedNote),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segmentation {
    pub notes: Vec<DetectedNote>,
    /// Estimated global tuning offset in semitones (+ means sharp).
    pub tuning_offset: f32,
    pub reference_db: f32,
    pub threshold_db: f32,
    pub discarded: Vec<Discarded>,
}

/// Circular mean of the fractional part of MIDI pitches: returns the offset
/// in semitones (-0.5..0.5) and its consistency (0..1).
pub fn estimate_tuning(midis: &[f32]) -> (f32, f32) {
    let weighted: Vec<(f32, f64)> = midis.iter().map(|&m| (m, 1.0)).collect();
    estimate_tuning_weighted(&weighted)
}

/// Like [`estimate_tuning`] with a weight per pitch (e.g. note duration).
pub fn estimate_tuning_weighted(pitches: &[(f32, f64)]) -> (f32, f32) {
    let n: f64 = pitches.iter().map(|p| p.1).sum();
    if n <= 0.0 {
        return (0.0, 0.0);
    }
    let (mut c, mut s) = (0.0f64, 0.0f64);
    for &(m, w) in pitches {
        let theta = std::f64::consts::TAU * f64::from(m - m.round());
        c += w * theta.cos();
        s += w * theta.sin();
    }
    let r = ((c * c + s * s).sqrt() / n) as f32;
    let offset = (s.atan2(c) / std::f64::consts::TAU) as f32;
    (offset, r)
}

fn gauss(x: f32, sigma: f32) -> f32 {
    (-0.5 * (x / sigma).powi(2)).exp() / (sigma * (2.0 * std::f32::consts::PI).sqrt())
}

#[derive(Clone, Copy, PartialEq)]
enum Obs {
    Voiced(f32),
    LoudUnvoiced,
    Quiet,
}

/// Viterbi over silence (state 0) and notes `lo..=hi` (states 1..).
fn note_viterbi(obs: &[Obs], lo: u8, hi: u8, p: &SegmentParams) -> Vec<Option<u8>> {
    // Back-pointers are stored as u8.
    let hi = hi.clamp(lo, lo.saturating_add(250));
    let n_notes = usize::from(hi - lo) + 1;
    let n = n_notes + 1;
    let range = n_notes as f32;
    let ln = |x: f64| x.max(1e-300).ln();

    // Emission probabilities by observation type.
    const NOTE_VOICED: f32 = 0.90;
    const NOTE_LOUD_UNVOICED: f32 = 0.06;
    const NOTE_QUIET: f32 = 0.04;
    const SIL_VOICED: f32 = 0.05;
    const SIL_LOUD_UNVOICED: f32 = 0.25;
    const SIL_QUIET: f32 = 0.70;

    let emit = |o: Obs, out: &mut [f64]| match o {
        Obs::Voiced(m) => {
            out[0] = f64::from(SIL_VOICED / range).ln();
            for (k, e) in out[1..].iter_mut().enumerate() {
                let pitch = f32::from(lo) + k as f32;
                // Mixture with a uniform outlier term so a few glitchy frames
                // (octave errors, breath) cannot break a note.
                let d = 0.95 * gauss(m - pitch, p.pitch_sigma) + 0.05 / range;
                *e = f64::from(NOTE_VOICED * d).ln();
            }
        }
        Obs::LoudUnvoiced => {
            out[0] = f64::from(SIL_LOUD_UNVOICED).ln();
            out[1..].fill(f64::from(NOTE_LOUD_UNVOICED).ln());
        }
        Obs::Quiet => {
            out[0] = f64::from(SIL_QUIET).ln();
            out[1..].fill(f64::from(NOTE_QUIET).ln());
        }
    };

    let start_prob = 0.02f64;
    let end_prob = 0.01f64;
    let ln_sil_stay = ln(1.0 - start_prob);
    let ln_sil_to_note = ln(start_prob / n_notes as f64);
    let ln_note_stay = ln(1.0 - end_prob - p.change_prob);
    let ln_note_end = ln(end_prob);
    // Interval preference for note changes: small steps are more likely.
    let interval_w: Vec<f64> = (0..n_notes).map(|k| (-(k as f64) / 6.0).exp()).collect();
    let ln_change: Vec<Vec<f64>> = (0..n_notes)
        .map(|i| {
            let z: f64 = (0..n_notes)
                .filter(|&j| j != i)
                .map(|j| interval_w[i.abs_diff(j)])
                .sum();
            (0..n_notes)
                .map(|j| ln(p.change_prob * interval_w[i.abs_diff(j)] / z))
                .collect()
        })
        .collect();

    let mut delta = vec![0.0f64; n];
    let mut next = vec![0.0f64; n];
    let mut emis = vec![0.0f64; n];
    let mut back = vec![0u8; obs.len() * n];

    emit(obs[0], &mut emis);
    delta[0] = emis[0];
    for s in 1..n {
        delta[s] = ln_sil_to_note + emis[s];
    }

    for t in 1..obs.len() {
        emit(obs[t], &mut emis);
        let row = &mut back[t * n..(t + 1) * n];
        // To silence.
        let mut best = delta[0] + ln_sil_stay;
        let mut arg = 0usize;
        for (s, d) in delta.iter().enumerate().skip(1) {
            let v = d + ln_note_end;
            if v > best {
                best = v;
                arg = s;
            }
        }
        next[0] = best + emis[0];
        row[0] = arg as u8;
        // To each note.
        for j in 1..n {
            let mut best = delta[0] + ln_sil_to_note;
            let mut arg = 0usize;
            for i in 1..n {
                let tr = if i == j {
                    ln_note_stay
                } else {
                    ln_change[i - 1][j - 1]
                };
                let v = delta[i] + tr;
                if v > best {
                    best = v;
                    arg = i;
                }
            }
            next[j] = best + emis[j];
            row[j] = arg as u8;
        }
        std::mem::swap(&mut delta, &mut next);
    }

    let mut state = (0..n)
        .max_by(|&a, &b| delta[a].total_cmp(&delta[b]))
        .unwrap_or(0);
    let mut out = vec![None; obs.len()];
    for t in (0..obs.len()).rev() {
        out[t] = (state > 0).then(|| lo + (state - 1) as u8);
        if t > 0 {
            state = usize::from(back[t * n + state]);
        }
    }
    out
}

/// Recursively splits `[a, b]` (inclusive frame indices) at energy dips.
///
/// Only sharp dips count: the level must fall by `dip_db` and come back
/// within `reach` frames on each side, which is what a re-articulated note
/// ("da da") looks like. The slow swells of a held note are not split.
fn split_at_dips(
    db: &[f32],
    a: usize,
    b: usize,
    min_frames: usize,
    reach: usize,
    dip_db: f32,
    out: &mut Vec<(usize, usize)>,
) {
    if b + 1 - a >= 2 * min_frames {
        let mut best: Option<(usize, f32)> = None;
        let upper = (b + 1 - min_frames).min(db.len().saturating_sub(2));
        for k in (a + min_frames)..=upper {
            if db[k] > db[k - 1] || db[k] > db[k + 1] {
                continue;
            }
            let left = db[a.max(k.saturating_sub(reach))..k]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            let right = db[k..=b.min(k + reach)]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            let depth = left.min(right) - db[k];
            if depth >= dip_db && best.is_none_or(|(_, d)| depth > d) {
                best = Some((k, depth));
            }
        }
        if let Some((k, _)) = best {
            split_at_dips(db, a, k - 1, min_frames, reach, dip_db, out);
            split_at_dips(db, k, b, min_frames, reach, dip_db, out);
            return;
        }
    }
    out.push((a, b));
}

fn smooth(values: &[f32], radius: usize) -> Vec<f32> {
    (0..values.len())
        .map(|i| {
            let lo = i.saturating_sub(radius);
            let hi = (i + radius).min(values.len() - 1);
            values[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
        })
        .collect()
}

/// Segments the pitch track into notes.
///
/// Tuning is estimated in two passes: segment without correction, take the
/// median pitch of each note (the median sits at the centre of a vibrato),
/// estimate the offset from those, then segment again with it removed.
/// Estimating from raw frames instead fails for wide vibrato, whose swing
/// wraps around the ±50 cent boundary.
pub fn segment(track: &PitchTrack, p: &SegmentParams) -> Segmentation {
    let first = segment_with_offset(track, p, 0.0);
    if !p.correct_tuning || first.notes.is_empty() {
        return first;
    }
    let weighted: Vec<(f32, f64)> = first
        .notes
        .iter()
        .map(|n| (n.raw_pitch, n.duration()))
        .collect();
    let (offset, consistency) = estimate_tuning_weighted(&weighted);
    if consistency < 0.3 || offset.abs() < 0.03 {
        return first;
    }
    segment_with_offset(track, p, offset)
}

/// Segments with a known tuning offset (semitones, + means sung sharp).
pub fn segment_with_offset(
    track: &PitchTrack,
    p: &SegmentParams,
    tuning_offset: f32,
) -> Segmentation {
    let frames = &track.frames;
    let hop = track.hop_seconds;
    let empty = |reference_db, threshold_db| Segmentation {
        notes: Vec::new(),
        tuning_offset,
        reference_db,
        threshold_db,
        discarded: Vec::new(),
    };
    if frames.is_empty() {
        return empty(-200.0, -200.0);
    }

    let db: Vec<f32> = frames.iter().map(|f| amp_to_db(f.rms)).collect();
    let reference_db = quantile(&db, 0.95);
    let threshold_db = (reference_db - p.silence_db_below_peak).max(p.absolute_floor_db);
    if reference_db < p.absolute_floor_db {
        return empty(reference_db, threshold_db);
    }
    let loud: Vec<bool> = db.iter().map(|&d| d >= threshold_db).collect();
    let midi: Vec<Option<f32>> = frames.iter().map(|f| f.f0.map(hz_to_midi)).collect();

    let obs: Vec<Obs> = midi
        .iter()
        .zip(&loud)
        .map(|(m, &l)| match (m, l) {
            (Some(m), true) => Obs::Voiced(m - tuning_offset),
            (None, true) => Obs::LoudUnvoiced,
            (_, false) => Obs::Quiet,
        })
        .collect();
    let states = note_viterbi(&obs, p.lowest_note, p.highest_note, p);

    // Runs of the same note state.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut t = 0;
    while t < states.len() {
        if let Some(note) = states[t] {
            let start = t;
            while t + 1 < states.len() && states[t + 1] == Some(note) {
                t += 1;
            }
            runs.push((start, t));
        }
        t += 1;
    }

    let min_frames = ((p.min_note_seconds / hop).round() as usize).max(1);
    let reach = ((p.split_dip_seconds / hop).round() as usize).max(2);
    let smoothed = smooth(&db, 2);
    let mut pieces = Vec::new();
    for (a, b) in runs {
        split_at_dips(
            &smoothed,
            a,
            b,
            min_frames,
            reach,
            p.split_dip_db,
            &mut pieces,
        );
    }

    let mut discarded = Vec::new();
    let mut notes: Vec<DetectedNote> = Vec::new();
    for (a, b) in pieces {
        let voiced: Vec<f32> = (a..=b).filter_map(|i| midi[i]).collect();
        let len = b + 1 - a;
        let peak_db = db[a..=b].iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let raw = if voiced.is_empty() {
            0.0
        } else {
            median(&voiced)
        };
        let note = DetectedNote {
            start: frames[a].time,
            end: frames[b].time + hop,
            sung_pitch: raw - tuning_offset,
            raw_pitch: raw,
            peak_db,
            velocity: (100.0 + 2.0 * (peak_db - reference_db)).clamp(30.0, 127.0) as u8,
            voiced_fraction: voiced.len() as f32 / len as f32,
        };
        if note.voiced_fraction < 0.4 {
            discarded.push(Discarded::MostlyUnvoiced(note));
        } else {
            notes.push(note);
        }
    }

    // Absorb fragments shorter than the minimum into a touching neighbour
    // (typically the scoop at the start of a note), else drop them.
    let touch = 2.5 * hop;
    let mut i = 0;
    while i < notes.len() {
        if notes[i].duration() >= p.min_note_seconds {
            i += 1;
            continue;
        }
        let frag = notes[i];
        let prev = i
            .checked_sub(1)
            .filter(|&j| frag.start - notes[j].end <= touch);
        let next = (i + 1 < notes.len() && notes[i + 1].start - frag.end <= touch).then_some(i + 1);
        let target = match (prev, next) {
            (Some(a), Some(b)) => {
                if (notes[a].sung_pitch - frag.sung_pitch).abs()
                    <= (notes[b].sung_pitch - frag.sung_pitch).abs()
                {
                    Some(a)
                } else {
                    Some(b)
                }
            }
            (a, b) => a.or(b),
        };
        notes.remove(i);
        match target {
            Some(j) => {
                let j = if j > i { j - 1 } else { j };
                notes[j].start = notes[j].start.min(frag.start);
                notes[j].end = notes[j].end.max(frag.end);
                discarded.push(Discarded::Merged {
                    fragment: frag,
                    into: j,
                });
            }
            None => discarded.push(Discarded::TooShort(frag)),
        }
    }

    Segmentation {
        notes,
        tuning_offset,
        reference_db,
        threshold_db,
        discarded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{PyinParams, pyin};
    use crate::synthetic::{self, HumParams, Tone};

    fn run(tones: &[Tone], params: HumParams) -> Segmentation {
        let sr = params.sample_rate;
        let x = synthetic::hum(tones, &params, 11);
        let track = pyin(&x, sr, &PyinParams::default());
        segment(&track, &SegmentParams::default())
    }

    fn pitches(s: &Segmentation) -> Vec<u8> {
        s.notes.iter().map(DetectedNote::nearest_pitch).collect()
    }

    #[test]
    fn tuning_estimate() {
        let (off, r) = estimate_tuning(&[60.2, 62.21, 64.19, 65.2]);
        assert!((off - 0.2).abs() < 0.01 && r > 0.99);
        // Offsets near ±0.5 wrap correctly.
        let (off, _) = estimate_tuning(&[60.49, 61.51]);
        assert!((off.abs() - 0.5).abs() < 0.02, "{off}");
    }

    #[test]
    fn separated_notes_with_vibrato_and_detune() {
        let tones = synthetic::melody(
            &[(60, 1.0), (64, 1.0), (67, 1.0), (72, 2.0)],
            100.0,
            0.2,
            0.0,
            1,
        );
        let s = run(
            &tones,
            HumParams {
                vibrato_cents: 45.0,
                detune_cents: -35.0,
                ..HumParams::new(44_100)
            },
        );
        assert_eq!(pitches(&s), vec![60, 64, 67, 72]);
        assert!(
            (s.tuning_offset + 0.35).abs() < 0.1,
            "tuning {}",
            s.tuning_offset
        );
        for (n, t) in s.notes.iter().zip(&tones) {
            assert!(
                (n.start - t.start).abs() < 0.04,
                "start {} vs {}",
                n.start,
                t.start
            );
        }
    }

    #[test]
    fn legato_steps_split_on_pitch_change() {
        // No gaps at all, with portamento between notes.
        let tones = [
            Tone::new(62.0, 0.1, 0.4),
            Tone::new(64.0, 0.5, 0.4),
            Tone::new(65.0, 0.9, 0.4),
            Tone::new(62.0, 1.3, 0.6),
        ];
        let s = run(
            &tones,
            HumParams {
                attack_seconds: 0.005,
                release_seconds: 0.005,
                ..HumParams::new(44_100)
            },
        );
        assert_eq!(pitches(&s), vec![62, 64, 65, 62]);
    }

    #[test]
    fn repeated_notes_split_on_energy_dips() {
        let tones = [
            Tone::new(67.0, 0.1, 0.25),
            Tone::new(67.0, 0.36, 0.25),
            Tone::new(67.0, 0.62, 0.25),
        ];
        let s = run(
            &tones,
            HumParams {
                attack_seconds: 0.02,
                release_seconds: 0.06,
                ..HumParams::new(44_100)
            },
        );
        assert_eq!(pitches(&s), vec![67, 67, 67]);
    }

    #[test]
    fn silence_has_no_notes() {
        let track = pyin(&vec![0.0; 22_050], 22_050, &PyinParams::default());
        assert!(segment(&track, &SegmentParams::default()).notes.is_empty());
    }
}
