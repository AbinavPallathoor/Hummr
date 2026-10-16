//! Choosing the voice for General MIDI instruments.

use crate::voice::{InstrumentVoice, PreviewSynth};

/// The voice that plays General MIDI `program`.
pub fn instrument(program: u8) -> Box<dyn InstrumentVoice> {
    Box::new(PreviewSynth { program })
}
