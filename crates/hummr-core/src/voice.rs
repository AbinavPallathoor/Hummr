//! Voices that play the notes of a notes clip.

use crate::AudioBuffer;
use crate::preview;
use crate::project::{Clip, Note, Song, Voice};
use crate::sequence::{SeqNote, Sequence};

/// Renders `notes` (beats from the clip start) with `voice`, mono at the
/// song's sample rate.
pub fn render_notes(
    song: &Song,
    _clip: &Clip,
    _source: &AudioBuffer,
    notes: &[Note],
    voice: &Voice,
) -> Vec<f32> {
    let program = match voice {
        Voice::Instrument { program } => *program,
        Voice::Sampler { .. } | Voice::Drums => 0,
    };
    let mut seq = Sequence::new("clip", song.bpm);
    seq.program = Some(program);
    seq.notes = notes
        .iter()
        .map(|n| SeqNote {
            pitch: n.pitch,
            start_beats: n.beat,
            duration_beats: n.len,
            velocity: n.vel,
        })
        .collect();
    preview::render(&seq, song.sample_rate, 0.0)
}
