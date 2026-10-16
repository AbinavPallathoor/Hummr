//! Undo and redo by song snapshots. A [`Song`](super::Song) holds no audio,
//! so a snapshot is a few kilobytes even for long projects.

use super::Song;

/// Undo/redo stacks of labelled song snapshots.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<(String, Song)>,
    redo: Vec<(String, Song)>,
}

/// How many steps are kept.
const LIMIT: usize = 500;

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers `before` (the song as it was before an edit called
    /// `label`). Clears the redo stack.
    pub fn record(&mut self, label: impl Into<String>, before: Song) {
        self.undo.push((label.into(), before));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Restores the previous state into `song`. Returns the label of the
    /// edit that was undone.
    pub fn undo(&mut self, song: &mut Song) -> Option<String> {
        let (label, before) = self.undo.pop()?;
        let now = std::mem::replace(song, before);
        self.redo.push((label.clone(), now));
        Some(label)
    }

    /// Re-applies the last undone edit.
    pub fn redo(&mut self, song: &mut Song) -> Option<String> {
        let (label, after) = self.redo.pop()?;
        let now = std::mem::replace(song, after);
        self.undo.push((label.clone(), now));
        Some(label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Drops the most recent undo step if it was recorded for a state equal
    /// to `song` (an edit that changed nothing).
    pub fn discard_noop(&mut self, song: &Song) {
        if self.undo.last().is_some_and(|(_, s)| s == song) {
            self.undo.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_round_trip() {
        let mut song = Song::new("a");
        let mut h = History::new();
        h.record("tempo", song.clone());
        song.bpm = 120.0;
        h.record("track", song.clone());
        song.add_track("x");

        assert_eq!(h.undo(&mut song).as_deref(), Some("track"));
        assert!(song.tracks.is_empty());
        assert_eq!(h.undo(&mut song).as_deref(), Some("tempo"));
        assert_eq!(song.bpm, 100.0);
        assert!(h.undo(&mut song).is_none());
        assert_eq!(h.redo(&mut song).as_deref(), Some("tempo"));
        assert_eq!(song.bpm, 120.0);
        h.record("new edit", song.clone());
        assert!(!h.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn noop_edits_are_discarded() {
        let song = Song::new("a");
        let mut h = History::new();
        h.record("nothing", song.clone());
        h.discard_noop(&song);
        assert!(!h.can_undo());
    }
}
