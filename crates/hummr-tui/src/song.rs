//! Tracks, recorded voice clips, and the four ways a new take is placed
//! into the song: append, insert, overwrite and overlay.

use std::sync::Arc;

use hummr_core::AudioBuffer;
use hummr_core::instruments;
use hummr_core::segment::DetectedNote;
use hummr_core::sequence::SeqNote;

/// A note in the editor, remembering what was actually sung (if anything).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditNote {
    pub note: SeqNote,
    pub sung: Option<DetectedNote>,
}

impl EditNote {
    pub fn start(&self) -> f64 {
        self.note.start_beats
    }

    pub fn end(&self) -> f64 {
        self.note.start_beats + self.note.duration_beats
    }
}

/// One instrument part.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Custom name; by default a track is called by its instrument.
    pub name: Option<String>,
    pub program: u8,
    pub notes: Vec<EditNote>,
}

impl Track {
    pub fn new(program: u8) -> Self {
        Self {
            name: None,
            program,
            notes: Vec::new(),
        }
    }

    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| instruments::name(self.program).to_string())
    }

    pub fn end_beats(&self) -> f64 {
        self.notes.iter().map(EditNote::end).fold(0.0, f64::max)
    }

    pub fn sort(&mut self) {
        self.notes.sort_by(|a, b| a.start().total_cmp(&b.start()));
    }
}

/// A recorded take of the singer's voice, positioned on the song timeline:
/// `origin_seconds` into the recording plays at beat `at_beat`.
#[derive(Clone, Debug)]
pub struct Clip {
    pub audio: Arc<AudioBuffer>,
    pub at_beat: f64,
    pub origin_seconds: f64,
}

/// Where a new take goes. Both are recorded in time with the song.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A new track (new instrument) layered on top of the song. Key `r`.
    Overlay,
    /// Re-record the current track from the playhead, replacing what was
    /// there for as long as you sang. Key `R`.
    Overwrite,
}

/// What a placement did, for the status message.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placed {
    pub added: usize,
    pub removed: usize,
}

/// Places `take` (notes measured from beat 0 of the take) into `notes` at
/// beat `at`. Overwrite first clears `at..until` (the stretch that was
/// recorded over) and shortens any note still ringing into it.
pub fn place(
    notes: &mut Vec<EditNote>,
    take: &[EditNote],
    placement: Placement,
    at: f64,
    until: f64,
) -> Placed {
    let eps = 1e-9;
    let mut placed = Placed {
        added: take.len(),
        removed: 0,
    };
    if placement == Placement::Overwrite {
        let take_end = at + take.iter().map(EditNote::end).fold(0.0, f64::max);
        let until = until.max(take_end);
        let before = notes.len();
        notes.retain(|n| !(n.start() >= at - eps && n.start() < until - eps));
        placed.removed = before - notes.len();
        for n in notes.iter_mut() {
            if n.start() < at && n.end() > at + eps {
                n.note.duration_beats = at - n.start();
            }
        }
    }
    notes.extend(take.iter().map(|n| {
        let mut n = *n;
        n.note.start_beats += at;
        n
    }));
    notes.sort_by(|a, b| a.start().total_cmp(&b.start()));
    placed
}

/// A pleasant instrument for a new layer that the song does not use yet.
pub fn next_instrument(used: &[u8]) -> u8 {
    const ROTATION: [u8; 10] = [0, 42, 73, 33, 48, 56, 71, 24, 11, 40];
    ROTATION
        .into_iter()
        .find(|p| !used.contains(p))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(pitch: u8, start: f64, dur: f64) -> EditNote {
        EditNote {
            note: SeqNote {
                pitch,
                start_beats: start,
                duration_beats: dur,
                velocity: 100,
            },
            sung: None,
        }
    }

    fn summary(notes: &[EditNote]) -> Vec<(u8, f64, f64)> {
        notes
            .iter()
            .map(|x| (x.note.pitch, x.note.start_beats, x.note.duration_beats))
            .collect()
    }

    fn song() -> Vec<EditNote> {
        vec![
            n(60, 0.0, 1.0),
            n(62, 1.0, 1.0),
            n(64, 2.0, 2.0),
            n(65, 4.0, 1.0),
        ]
    }

    fn take() -> Vec<EditNote> {
        vec![n(72, 0.0, 1.0), n(71, 1.0, 1.0)]
    }

    #[test]
    fn overwrite_replaces_what_was_recorded_over() {
        // Recorded from beat 1 to beat 3: D4 and E4 go, C4 and F4 stay.
        let mut notes = song();
        let r = place(&mut notes, &take(), Placement::Overwrite, 1.0, 3.0);
        assert_eq!(r.removed, 2);
        assert_eq!(
            summary(&notes),
            vec![
                (60, 0.0, 1.0),
                (72, 1.0, 1.0),
                (71, 2.0, 1.0),
                (65, 4.0, 1.0)
            ]
        );
    }

    #[test]
    fn overwrite_covers_a_longer_recording_than_the_notes() {
        // Kept recording (silently) until beat 6: F4 at beat 4 goes too.
        let mut notes = song();
        place(&mut notes, &take(), Placement::Overwrite, 1.0, 6.0);
        assert_eq!(
            summary(&notes),
            vec![(60, 0.0, 1.0), (72, 1.0, 1.0), (71, 2.0, 1.0)]
        );
    }

    #[test]
    fn overwrite_trims_a_note_that_rings_into_the_take() {
        let mut notes = vec![n(60, 0.0, 3.0)];
        place(&mut notes, &take(), Placement::Overwrite, 2.0, 4.0);
        assert_eq!(
            summary(&notes),
            vec![(60, 0.0, 2.0), (72, 2.0, 1.0), (71, 3.0, 1.0)]
        );
    }

    #[test]
    fn overlay_just_adds() {
        let mut notes = Vec::new();
        place(&mut notes, &take(), Placement::Overlay, 4.0, 8.0);
        assert_eq!(summary(&notes), vec![(72, 4.0, 1.0), (71, 5.0, 1.0)]);
    }

    #[test]
    fn new_layers_get_unused_instruments() {
        assert_eq!(next_instrument(&[40]), 0);
        assert_eq!(next_instrument(&[40, 0]), 42);
    }
}
