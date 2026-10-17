//! `e` on a notes clip: the old piano-roll editor, opened on just that
//! clip's notes. Esc (or :q) comes back and the edited notes replace the
//! clip's, as one undo step.

use std::time::Duration;

use hummr_core::project::{ClipKind, Note, Voice};
use hummr_core::sequence::SeqNote;
use ratatui::crossterm::event::KeyEvent;

use super::{Screen, Studio};
use crate::app::{App, Level, Options};
use crate::song::{EditNote, Track};

impl Studio {
    /// Opens the notes editor on the selected clip.
    pub fn open_notes(&mut self) {
        let Some(clip) = self.selected_clip().cloned() else {
            self.say(
                "No clip under the playhead on this row. Move onto a notes clip, then press e.",
                Level::Warn,
            );
            return;
        };
        let ClipKind::Notes { voice, notes, .. } = &clip.kind else {
            self.say(
                "This clip is a recording. To edit notes, turn it into notes first: Enter → Turn into → Notes.",
                Level::Info,
            );
            return;
        };
        if self.playing {
            self.toggle_play();
        }
        let program = match voice {
            Voice::Instrument { program } => *program,
            _ => 0,
        };
        let mut app = App::new(&Options {
            file: None,
            animations: self.animations,
            mouse: false,
        });
        app.tempo_bpm = self.song().bpm;
        app.key = self.song().key;
        app.quantize = self.song().snap;
        app.tracks = vec![Track {
            name: Some(clip.name.clone()),
            program,
            notes: notes
                .iter()
                .map(|n| EditNote {
                    note: SeqNote {
                        pitch: n.pitch,
                        start_beats: n.beat,
                        duration_beats: n.len,
                        velocity: n.vel,
                    },
                    sung: None,
                })
                .collect(),
        }];
        app.sync_strips();
        app.view_fit = true;
        app.embedded = Some(clip.name.clone());
        let preview = if matches!(voice, Voice::Instrument { .. }) {
            ""
        } else {
            " (it plays with a stand-in instrument here)"
        };
        app.info(format!(
            "Editing the notes of \"{}\"{preview}. Esc goes back to the timeline.",
            clip.name
        ));
        self.notes_editor = Some((clip.id, Box::new(app)));
        self.screen = Screen::Notes;
    }

    /// A key in the notes editor; returns to the timeline when it quits.
    pub fn notes_key(&mut self, key: KeyEvent) {
        if let Some((_, app)) = &mut self.notes_editor {
            crate::input::on_key(app, key);
            if app.quit {
                self.close_notes();
            }
        }
    }

    pub(super) fn tick_notes(&mut self, dt: Duration) {
        if let Some((_, app)) = &mut self.notes_editor {
            app.tick(dt);
        }
    }

    /// Back to the timeline, writing the notes into the clip.
    pub fn close_notes(&mut self) {
        let Some((id, mut app)) = self.notes_editor.take() else {
            return;
        };
        app.stop();
        self.screen = Screen::Timeline;
        let Some(track) = app.tracks.first() else {
            return;
        };
        let new: Vec<Note> = track
            .notes
            .iter()
            .map(|n| Note {
                pitch: n.note.pitch,
                beat: n.note.start_beats.max(0.0),
                len: n.note.duration_beats,
                vel: n.note.velocity,
            })
            .collect();
        let program = track.program;
        let end = new.iter().map(|n| n.beat + n.len).fold(0.0, f64::max);
        let count = new.len();
        self.edit("edit notes", |s| {
            if let Some(c) = s.clip_mut(id) {
                if let ClipKind::Notes {
                    notes,
                    length_beats,
                    voice,
                } = &mut c.kind
                {
                    *notes = new;
                    *length_beats = length_beats.max(end.ceil());
                    if let Voice::Instrument { program: p } = voice {
                        *p = program;
                    }
                }
            }
        });
        let extra = if app.tracks.len() > 1 {
            " Only the first part came back; extra parts added there are dropped."
        } else {
            ""
        };
        self.say(
            format!(
                "Back on the timeline: {count} note{} (u to undo).{extra}",
                if count == 1 { "" } else { "s" }
            ),
            if extra.is_empty() {
                Level::Info
            } else {
                Level::Warn
            },
        );
    }
}
