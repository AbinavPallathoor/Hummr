//! Hummed audio → notes, with an explanation for every change made.

use std::fmt;
use std::str::FromStr;

use crate::dsp::Biquad;
use crate::notation;
use crate::pitch::{PitchTrack, PyinParams, pyin};
use crate::rhythm::{Grid, TempoEstimate, detect_tempo, fit_fixed_tempo};
use crate::segment::{DetectedNote, Discarded, SegmentParams, Segmentation, segment};
use crate::sequence::{SeqNote, Sequence};
use crate::theory::{Key, KeyEstimate, Scale, detect_key, format_cents, note_name};
use crate::{AudioBuffer, Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyChoice {
    Auto,
    Fixed(Key),
    /// No key: notes keep the nearest chromatic pitch.
    Off,
}

impl FromStr for KeyChoice {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "detect" => Ok(KeyChoice::Auto),
            "none" | "off" | "chromatic" => Ok(KeyChoice::Off),
            _ => s.parse().map(KeyChoice::Fixed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoChoice {
    Auto,
    Fixed(f64),
}

impl FromStr for TempoChoice {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim().to_ascii_lowercase();
        if t == "auto" || t == "detect" {
            return Ok(TempoChoice::Auto);
        }
        let bpm: f64 = t
            .trim_end_matches("bpm")
            .trim()
            .parse()
            .map_err(|_| Error::parse("tempo", s, "expected 'auto' or BPM like 96"))?;
        if !(20.0..=400.0).contains(&bpm) {
            return Err(Error::parse("tempo", s, "BPM must be between 20 and 400"));
        }
        Ok(TempoChoice::Fixed(bpm))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscribeOptions {
    pub key: KeyChoice,
    pub quantize: Option<Grid>,
    pub tempo: TempoChoice,
    pub min_bpm: f64,
    pub max_bpm: f64,
    pub pyin: PyinParams,
    pub segment: SegmentParams,
    /// Time in the recording (seconds) that is beat 0, e.g. the end of a
    /// count-in. When set, the grid is anchored there instead of being
    /// fitted to the onsets, and notes sung before it are dropped.
    pub grid_origin: Option<f64>,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            key: KeyChoice::Auto,
            quantize: Some(Grid::EIGHTH),
            tempo: TempoChoice::Auto,
            min_bpm: 70.0,
            max_bpm: 160.0,
            pyin: PyinParams::default(),
            segment: SegmentParams::default(),
            grid_origin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranscribedNote {
    pub pitch: u8,
    pub start_beats: f64,
    pub duration_beats: f64,
    pub velocity: u8,
    /// What was actually sung.
    pub detected: DetectedNote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Pitch,
    Timing,
    Length,
    Merged,
    Dropped,
}

/// One explained adjustment.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// Index into [`Transcription::notes`], if the change concerns a kept note.
    pub note: Option<usize>,
    /// Time in the recording (seconds).
    pub time: f64,
    pub kind: ChangeKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeySource {
    Detected(KeyEstimate),
    Given,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoSource {
    Detected(TempoEstimate),
    Given(TempoEstimate),
    /// Too few notes to detect a tempo.
    Default,
}

#[derive(Clone, Debug)]
pub struct Transcription {
    pub notes: Vec<TranscribedNote>,
    pub key: Option<Key>,
    pub key_source: KeySource,
    pub tempo_bpm: f64,
    pub tempo_source: TempoSource,
    pub grid: Option<Grid>,
    /// Time in the recording (seconds) that corresponds to beat 0.
    pub origin_seconds: f64,
    /// Global tuning offset that was removed, in semitones.
    pub tuning_offset: f32,
    pub changes: Vec<Change>,
    pub duration_seconds: f64,
    pub pitch_track: PitchTrack,
    pub segmentation: Segmentation,
}

impl Transcription {
    pub fn to_sequence(&self, name: &str) -> Sequence {
        let mut seq = Sequence::new(name, self.tempo_bpm);
        seq.key = self.key;
        seq.notes = self
            .notes
            .iter()
            .map(|n| SeqNote {
                pitch: n.pitch,
                start_beats: n.start_beats,
                duration_beats: n.duration_beats,
                velocity: n.velocity,
            })
            .collect();
        seq
    }

    /// The notes in project-file notation, e.g. `C4 1/8 E4 1/4`.
    pub fn notation(&self) -> String {
        let notes: Vec<(u8, f64, f64)> = self
            .notes
            .iter()
            .map(|n| (n.pitch, n.start_beats, n.duration_beats))
            .collect();
        notation::format(&notes, self.key.as_ref())
    }

    pub fn changes_of(&self, kind: ChangeKind) -> usize {
        self.changes.iter().filter(|c| c.kind == kind).count()
    }
}

fn ms(seconds: f64) -> String {
    format!("{:.0} ms", seconds * 1000.0)
}

/// Musical position of a beat offset, 1-based, assuming 4/4.
pub struct Position(pub f64);

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bar = (self.0 / 4.0).floor() as i64 + 1;
        let beat = self.0.rem_euclid(4.0) + 1.0;
        if (beat - beat.round()).abs() < 1e-6 {
            write!(f, "bar {bar} beat {}", beat.round())
        } else {
            write!(f, "bar {bar} beat {}", (beat * 1000.0).round() / 1000.0)
        }
    }
}

/// Runs the full pipeline: pitch tracking, segmentation, key snapping and
/// quantisation.
pub fn transcribe(audio: &AudioBuffer, opts: &TranscribeOptions) -> Result<Transcription> {
    if audio.samples.is_empty() {
        return Err(Error::Audio("the recording is empty".into()));
    }
    if audio.sample_rate < 8_000 {
        return Err(Error::Audio(format!(
            "sample rate {} Hz is too low; record at 16 kHz or more",
            audio.sample_rate
        )));
    }
    let sr = audio.sample_rate;

    // Remove rumble and DC so they cannot dominate the level estimates.
    let mut x = audio.samples.clone();
    Biquad::highpass(sr as f32, 40.0, 0.707).process_buffer(&mut x);
    let track = pyin(&x, sr, &opts.pyin);
    let seg = segment(&track, &opts.segment);
    let detected = seg.notes.clone();

    let mut changes: Vec<Change> = Vec::new();
    for d in &seg.discarded {
        let (time, message) = match d {
            Discarded::Merged { fragment, .. } => (
                fragment.start,
                format!(
                    "merged a {} blip ({}) into the neighbouring note",
                    ms(fragment.duration()),
                    note_name(fragment.nearest_pitch())
                ),
            ),
            Discarded::TooShort(n) => (
                n.start,
                format!(
                    "dropped a {} blip ({}): shorter than {}",
                    ms(n.duration()),
                    note_name(n.nearest_pitch()),
                    ms(opts.segment.min_note_seconds)
                ),
            ),
            Discarded::MostlyUnvoiced(n) => (
                n.start,
                format!(
                    "dropped {} of unpitched sound (breath or consonant)",
                    ms(n.duration())
                ),
            ),
        };
        changes.push(Change {
            note: None,
            time,
            kind: ChangeKind::Dropped,
            message,
        });
    }

    // Key.
    let (key, key_source) = match opts.key {
        KeyChoice::Off => (None, KeySource::Off),
        KeyChoice::Fixed(k) => (Some(k), KeySource::Given),
        KeyChoice::Auto => {
            let last = detected.len().saturating_sub(1);
            // Melodies usually end on the tonic, so the last note counts double.
            let weighted: Vec<(u8, f64)> = detected
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    (
                        n.nearest_pitch(),
                        n.duration() * if i == last { 2.0 } else { 1.0 },
                    )
                })
                .collect();
            match detect_key(&weighted) {
                Some(est) => (Some(est.key), KeySource::Detected(est)),
                None => (None, KeySource::Off),
            }
        }
    };

    // Pitch snapping; changes are attached to final note indices later.
    let mut pitch_notes: Vec<Option<String>> = vec![None; detected.len()];
    let pitches: Vec<u8> = detected
        .iter()
        .enumerate()
        .map(|(i, n)| match key.filter(|k| k.scale != Scale::Chromatic) {
            Some(k) => {
                let s = k.snap(n.sung_pitch);
                if s.moved() {
                    let from = k.note_name(s.nearest_chromatic);
                    let to = k.note_name(s.pitch);
                    let off = n.sung_pitch - f32::from(s.nearest_chromatic);
                    let why = match s.other_option {
                        Some(o) => format!(
                            "sung {from} {}, nearer {to} than {}",
                            format_cents(off),
                            k.note_name(o)
                        ),
                        None => format!("sung {from} {}", format_cents(off)),
                    };
                    pitch_notes[i] = Some(format!("moved {from} to {to}: outside {k} ({why})"));
                }
                s.pitch
            }
            None => n.nearest_pitch(),
        })
        .collect();

    // Tempo.
    let onsets: Vec<f64> = detected.iter().map(|n| n.start).collect();
    let weights: Vec<f64> = vec![1.0; onsets.len()];
    let grid_beats = opts.quantize.map_or(Grid::EIGHTH.beats(), |g| g.beats());
    let fallback_phase = onsets.first().copied().unwrap_or(0.0);
    let (tempo, tempo_source) = match opts.tempo {
        TempoChoice::Fixed(bpm) => {
            let t = if onsets.is_empty() {
                TempoEstimate {
                    bpm,
                    phase: 0.0,
                    fit: 0.0,
                }
            } else {
                fit_fixed_tempo(&onsets, &weights, grid_beats, bpm)
            };
            (t, TempoSource::Given(t))
        }
        TempoChoice::Auto => {
            match detect_tempo(&onsets, &weights, grid_beats, opts.min_bpm, opts.max_bpm) {
                Some(t) => (t, TempoSource::Detected(t)),
                None => (
                    TempoEstimate {
                        bpm: 120.0,
                        phase: fallback_phase,
                        fit: 0.0,
                    },
                    TempoSource::Default,
                ),
            }
        }
    };

    let tempo = match opts.grid_origin {
        Some(origin) => TempoEstimate {
            phase: origin,
            ..tempo
        },
        None => tempo,
    };
    let anchored = opts.grid_origin.is_some();
    let count_in_drop = |d: &DetectedNote| Change {
        note: None,
        time: d.start,
        kind: ChangeKind::Dropped,
        message: format!(
            "dropped {} at {:.3}s: sung before the count-in finished",
            note_name(d.nearest_pitch()),
            d.start
        ),
    };

    // Quantise (or just convert to beats).
    struct Slot {
        start: i64,
        end: i64,
        src: usize,
    }
    let mut notes: Vec<TranscribedNote> = Vec::new();
    let mut src_of_note: Vec<usize> = Vec::new();
    let mut timing_notes: Vec<Vec<(ChangeKind, String)>> = Vec::new();
    let origin_seconds;

    if let Some(grid) = opts.quantize {
        let step = tempo.grid_step_seconds(grid_beats);
        let idx = |t: f64| ((t - tempo.phase) / step).round() as i64;
        // People sing notes shorter than written, so ends round up a little.
        let end_idx = |t: f64| ((t - tempo.phase) / step + 0.25).round() as i64;
        let mut slots: Vec<Slot> = detected
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let s = idx(n.start);
                Slot {
                    start: s,
                    end: end_idx(n.end).max(s + 1),
                    src: i,
                }
            })
            .collect();
        slots.sort_by_key(|s| (s.start, s.src));

        let mut kept: Vec<Slot> = Vec::new();
        let mut collision_notes: Vec<Option<String>> = vec![None; detected.len()];
        for mut slot in slots {
            if let Some(prev) = kept.last_mut() {
                if slot.start <= prev.start {
                    if prev.start + 1 < slot.end {
                        slot.start = prev.start + 1;
                        collision_notes[slot.src] = Some(format!(
                            "pushed to the next {grid} slot (the previous note already starts on its nearest one)"
                        ));
                    } else {
                        let d = &detected[slot.src];
                        prev.end = prev.end.max(slot.end);
                        changes.push(Change {
                            note: None,
                            time: d.start,
                            kind: ChangeKind::Merged,
                            message: format!(
                                "dropped {} at {:.3}s: it landed on the same {grid} slot as the previous note",
                                note_name(pitches[slot.src]),
                                d.start
                            ),
                        });
                        continue;
                    }
                }
                if prev.end > slot.start {
                    prev.end = slot.start;
                }
            }
            kept.push(slot);
        }

        // Singers breathe between notes. A gap shorter than one grid step
        // before the next note is articulation, not a rest: hold the note.
        let mut legato = vec![false; detected.len()];
        for i in 0..kept.len().saturating_sub(1) {
            let (next_start, next_src) = (kept[i + 1].start, kept[i + 1].src);
            let slot = &mut kept[i];
            let real_gap = detected[next_src].start - detected[slot.src].end;
            if real_gap < step && next_start - slot.end <= 1 {
                slot.end = next_start;
                legato[slot.src] = true;
            }
        }
        let held = legato.iter().filter(|&&l| l).count();
        if held > 0 {
            changes.push(Change {
                note: None,
                time: detected.first().map_or(0.0, |d| d.start),
                kind: ChangeKind::Length,
                message: format!(
                    "held {held} notes through the short breaths before the next note \
                     (gaps under one {grid} step are articulation, not rests)"
                ),
            });
        }

        if anchored {
            kept.retain(|slot| {
                let early = slot.start < 0;
                if early {
                    changes.push(count_in_drop(&detected[slot.src]));
                }
                !early
            });
        }
        let k0 = if anchored {
            0
        } else {
            kept.first().map_or(0, |s| s.start)
        };
        origin_seconds = tempo.phase + k0 as f64 * step;
        for slot in &kept {
            let d = &detected[slot.src];
            let start_beats = (slot.start - k0) as f64 * grid_beats;
            let duration_beats = (slot.end - slot.start) as f64 * grid_beats;
            let mut why = Vec::new();
            if let Some(c) = &collision_notes[slot.src] {
                why.push((ChangeKind::Timing, c.clone()));
            }
            let q_start = tempo.phase + slot.start as f64 * step;
            let shift = q_start - d.start;
            if shift.abs() >= 0.010 {
                why.push((
                    ChangeKind::Timing,
                    format!(
                        "start {:.3}s → {} ({}{})",
                        d.start,
                        Position(start_beats),
                        if shift >= 0.0 { "+" } else { "" },
                        ms(shift),
                    ),
                ));
            }
            let q_len = (slot.end - slot.start) as f64 * step;
            if !legato[slot.src] && (q_len - d.duration()).abs() >= 0.025 {
                why.push((
                    ChangeKind::Length,
                    format!(
                        "length {} → {} ({})",
                        ms(d.duration()),
                        notation::format_length(duration_beats),
                        ms(q_len)
                    ),
                ));
            }
            notes.push(TranscribedNote {
                pitch: pitches[slot.src],
                start_beats,
                duration_beats,
                velocity: d.velocity,
                detected: *d,
            });
            src_of_note.push(slot.src);
            timing_notes.push(why);
        }
    } else {
        let t0 = opts.grid_origin.unwrap_or(fallback_phase);
        origin_seconds = t0;
        let beats_per_sec = tempo.bpm / 60.0;
        for (i, d) in detected.iter().enumerate() {
            if d.start < t0 - 0.05 {
                changes.push(count_in_drop(d));
                continue;
            }
            notes.push(TranscribedNote {
                pitch: pitches[i],
                start_beats: ((d.start - t0) * beats_per_sec).max(0.0),
                duration_beats: d.duration() * beats_per_sec,
                velocity: d.velocity,
                detected: *d,
            });
            src_of_note.push(i);
            timing_notes.push(Vec::new());
        }
    }

    for (ni, (&src, timing)) in src_of_note.iter().zip(timing_notes).enumerate() {
        let time = detected[src].start;
        if let Some(msg) = pitch_notes[src].take() {
            changes.push(Change {
                note: Some(ni),
                time,
                kind: ChangeKind::Pitch,
                message: msg,
            });
        }
        for (kind, message) in timing {
            changes.push(Change {
                note: Some(ni),
                time,
                kind,
                message,
            });
        }
    }
    changes.sort_by(|a, b| a.time.total_cmp(&b.time));

    Ok(Transcription {
        notes,
        key,
        key_source,
        tempo_bpm: tempo.bpm,
        tempo_source,
        grid: opts.quantize,
        origin_seconds,
        tuning_offset: seg.tuning_offset,
        changes,
        duration_seconds: audio.duration_seconds(),
        pitch_track: track,
        segmentation: seg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_choices() {
        assert_eq!("auto".parse::<KeyChoice>().unwrap(), KeyChoice::Auto);
        assert_eq!("none".parse::<KeyChoice>().unwrap(), KeyChoice::Off);
        assert_eq!(
            "D minor".parse::<KeyChoice>().unwrap(),
            KeyChoice::Fixed(Key::new(2, Scale::Minor))
        );
        assert_eq!(
            "96".parse::<TempoChoice>().unwrap(),
            TempoChoice::Fixed(96.0)
        );
        assert_eq!(
            "120bpm".parse::<TempoChoice>().unwrap(),
            TempoChoice::Fixed(120.0)
        );
        assert!("fast".parse::<TempoChoice>().is_err());
        assert!("1000".parse::<TempoChoice>().is_err());
    }

    #[test]
    fn positions() {
        assert_eq!(Position(0.0).to_string(), "bar 1 beat 1");
        assert_eq!(Position(5.5).to_string(), "bar 2 beat 2.5");
    }

    #[test]
    fn empty_audio_is_an_error() {
        let e = transcribe(
            &AudioBuffer::new(vec![], 44_100),
            &TranscribeOptions::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("empty"));
    }

    #[test]
    fn silence_gives_no_notes() {
        let t = transcribe(
            &AudioBuffer::new(vec![0.0; 44_100], 44_100),
            &TranscribeOptions::default(),
        )
        .unwrap();
        assert!(t.notes.is_empty());
        assert!(matches!(t.tempo_source, TempoSource::Default));
    }
}
