//! A monophonic or polyphonic note sequence measured in beats (quarter notes).

use crate::theory::Key;

/// MIDI ticks per quarter note used for everything we write.
pub const PPQ: u16 = 480;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeqNote {
    pub pitch: u8,
    pub start_beats: f64,
    pub duration_beats: f64,
    pub velocity: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub name: String,
    pub tempo_bpm: f64,
    pub key: Option<Key>,
    /// General MIDI program (0-based), if an instrument is chosen.
    pub program: Option<u8>,
    /// MIDI channel 0..=15 (9 is drums).
    pub channel: u8,
    pub notes: Vec<SeqNote>,
}

impl Sequence {
    pub fn new(name: impl Into<String>, tempo_bpm: f64) -> Self {
        Self {
            name: name.into(),
            tempo_bpm,
            key: None,
            program: None,
            channel: 0,
            notes: Vec::new(),
        }
    }

    pub fn end_beats(&self) -> f64 {
        self.notes
            .iter()
            .map(|n| n.start_beats + n.duration_beats)
            .fold(0.0, f64::max)
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        beats * 60.0 / self.tempo_bpm
    }
}
