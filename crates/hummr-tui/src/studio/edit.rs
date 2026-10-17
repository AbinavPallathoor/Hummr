//! Clip and track editing on the timeline. Every change is one undo step.

use hummr_core::AudioBuffer;
use hummr_core::project::Clip;

use super::{Row, Studio};
use crate::app::Level;

const NO_CLIP: &str = "No clip under the playhead on this row. Move with h/l (time) or j/k (rows).";

impl Studio {
    /// Selects the row holding clip `id` and keeps the playhead on it.
    pub fn follow_clip(&mut self, id: u64) {
        let song = self.song();
        let Some((t, i)) = song.find_clip(id) else {
            return;
        };
        let lane = song.lanes(t)[i];
        let clip = &song.tracks[t].clips[i];
        let (start, end) = (clip.start_beat, clip.end_beat(song.bpm));
        if let Some(r) = self
            .rows()
            .iter()
            .position(|r| *r == Row { track: t, lane })
        {
            self.row = r;
        }
        if self.playhead < start - 1e-9 || self.playhead >= end - 1e-9 {
            self.playhead = start;
        }
    }

    /// The selected clip's id, or a message saying why there is none.
    fn need_clip(&mut self) -> Option<u64> {
        let id = self.selected_clip_id();
        if id.is_none() {
            self.say(NO_CLIP, Level::Warn);
        }
        id
    }

    /// `H`/`L`: moves the selected clip one snap step, landing on the grid.
    pub fn nudge_clip(&mut self, dir: f64) {
        let Some(id) = self.need_clip() else { return };
        let step = self.song().snap_beats();
        let clip = self.song().clip(id).cloned().expect("selected clip exists");
        // The next grid line in that direction (one step when on the grid).
        let at = clip.start_beat / step;
        let target = if dir > 0.0 {
            ((at + 1e-6).floor() + 1.0) * step
        } else {
            ((at - 1e-6).ceil() - 1.0) * step
        }
        .max(0.0);
        let delta = target - clip.start_beat;
        if delta.abs() < 1e-9 {
            return;
        }
        let (t, _) = self.song().find_clip(id).expect("clip");
        self.edit("move clip", |s| s.move_clip(id, t, target));
        self.playhead = (self.playhead + delta).max(0.0);
        self.follow_clip(id);
    }

    /// `J`/`K`: moves the selected clip to the next/previous track.
    pub fn shift_clip_track(&mut self, dir: i64) {
        let Some(id) = self.need_clip() else { return };
        let (t, i) = self.song().find_clip(id).expect("clip");
        let n = self.song().tracks.len() as i64;
        let to = t as i64 + dir;
        if to < 0 || to >= n {
            self.say(
                if dir > 0 {
                    "This is the last track. Press n to add one."
                } else {
                    "This is the first track."
                },
                Level::Info,
            );
            return;
        }
        let start = self.song().tracks[t].clips[i].start_beat;
        self.edit("move clip to another track", |s| {
            s.move_clip(id, to as usize, start)
        });
        self.follow_clip(id);
    }

    /// `s`: splits the selected clip at the playhead.
    pub fn split(&mut self) {
        let Some(id) = self.need_clip() else { return };
        let at = self.playhead;
        match self.edit("split clip", |s| s.split_clip(id, at)) {
            Some((_, right)) => {
                self.follow_clip(right);
                self.say("Split in two (u to undo).", Level::Info);
            }
            None => self.say(
                "Can't split at the very edge of a clip. Move the playhead inside it.",
                Level::Warn,
            ),
        }
    }

    /// `x`: deletes the selected clip.
    pub fn delete_clip(&mut self) {
        let Some(id) = self.need_clip() else { return };
        let name = self
            .song()
            .clip(id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        self.edit("delete clip", |s| s.remove_clip(id));
        self.clamp_view();
        self.say(format!("Deleted \"{name}\" (u to undo)."), Level::Info);
    }

    /// `y`: copies the selected clip.
    pub fn copy_clip(&mut self) {
        let Some(id) = self.need_clip() else { return };
        self.clipboard = self.song().clip(id).cloned();
        let name = self
            .clipboard
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_default();
        self.say(
            format!("Copied \"{name}\". Press p to paste it at the playhead."),
            Level::Info,
        );
    }

    /// `p`: pastes the copied clip at the playhead on the selected track.
    pub fn paste(&mut self) {
        let Some(mut clip) = self.clipboard.clone() else {
            self.say(
                "Nothing copied yet. Select a clip and press y.",
                Level::Info,
            );
            return;
        };
        let Some(t) = self.current_track() else {
            self.say("Add a track first (n).", Level::Info);
            return;
        };
        clip.start_beat = self.playhead;
        if let Some(id) = self.edit("paste clip", |s| s.insert_clip(t, clip)) {
            self.follow_clip(id);
        }
    }

    /// `d`: duplicates the selected clip right after itself.
    pub fn duplicate(&mut self) {
        let Some(id) = self.need_clip() else { return };
        let bpm = self.song().bpm;
        let (t, i) = self.song().find_clip(id).expect("clip");
        let mut copy = self.song().tracks[t].clips[i].clone();
        copy.start_beat = copy.end_beat(bpm);
        if let Some(new) = self.edit("duplicate clip", |s| s.insert_clip(t, copy)) {
            let start = self.song().clip(new).map_or(0.0, |c| c.start_beat);
            self.playhead = start;
            self.follow_clip(new);
        }
    }

    pub fn undo(&mut self) {
        self.last_merge = None;
        match self.history.undo(&mut self.project.song) {
            Some(label) => {
                self.touch();
                self.clamp_view();
                self.say(format!("Undid {label}."), Level::Info);
            }
            None => self.say("Nothing to undo.", Level::Info),
        }
    }

    pub fn redo(&mut self) {
        self.last_merge = None;
        match self.history.redo(&mut self.project.song) {
            Some(label) => {
                self.touch();
                self.clamp_view();
                self.say(format!("Redid {label}."), Level::Info);
            }
            None => self.say("Nothing to redo.", Level::Info),
        }
    }

    /// `m`: mutes or unmutes the selected track.
    pub fn toggle_mute(&mut self) {
        let Some(t) = self.current_track() else {
            return;
        };
        let on = self.edit("mute", |s| {
            s.tracks[t].mute = !s.tracks[t].mute;
            s.tracks[t].mute
        });
        let name = self.song().tracks[t].name.clone();
        self.say(
            format!("{name} {}.", if on { "muted" } else { "unmuted" }),
            Level::Info,
        );
    }

    /// `S`: solos or unsolos the selected track.
    pub fn toggle_solo(&mut self) {
        let Some(t) = self.current_track() else {
            return;
        };
        let on = self.edit("solo", |s| {
            s.tracks[t].solo = !s.tracks[t].solo;
            s.tracks[t].solo
        });
        let name = self.song().tracks[t].name.clone();
        self.say(
            format!(
                "{name} {}.",
                if on {
                    "solo: only soloed tracks play"
                } else {
                    "no longer solo"
                }
            ),
            Level::Info,
        );
    }

    /// `n`: adds a track at the bottom and selects it.
    pub fn new_track(&mut self, name: Option<String>) -> usize {
        let n = self.song().tracks.len() + 1;
        let name = name.unwrap_or_else(|| format!("Track {n}"));
        let t = self.edit("new track", |s| s.add_track(name));
        if let Some(r) = self.rows().iter().position(|r| r.track == t) {
            self.row = r;
        }
        t
    }

    /// Renames the selected track.
    pub fn rename_track(&mut self, name: &str) {
        let Some(t) = self.current_track() else {
            return;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            self.say("Give the track a name, e.g. :track name Bass", Level::Error);
            return;
        }
        self.edit("rename track", |s| s.tracks[t].name = name);
    }

    /// Deletes the selected track and its clips.
    pub fn delete_track(&mut self) {
        let Some(t) = self.current_track() else {
            return;
        };
        let name = self.song().tracks[t].name.clone();
        self.edit("delete track", |s| s.tracks.remove(t));
        self.clamp_view();
        self.say(format!("Deleted track {name} (u to undo)."), Level::Info);
    }

    /// Adds a new source and a clip of all of it at `at_beat` on `track`
    /// (creating a track if there is none). Returns the clip id.
    pub fn add_clip_from_audio(
        &mut self,
        name: &str,
        audio: AudioBuffer,
        recorded_bpm: Option<f64>,
        track: Option<usize>,
        at_beat: f64,
    ) -> u64 {
        let id = self.project.add_audio(name, audio, recorded_bpm);
        self.add_clip_of_source(&id, name, track, at_beat)
    }

    /// Places all of source `src` at `at_beat` on `track`.
    pub fn add_clip_of_source(
        &mut self,
        src: &str,
        name: &str,
        track: Option<usize>,
        at_beat: f64,
    ) -> u64 {
        let len = self
            .project
            .sources
            .get(src)
            .map_or(0.0, |a| a.duration_seconds());
        let clip = Clip::audio(0, name, src, 0.0, len, at_beat.max(0.0));
        let track = match track.filter(|&t| t < self.song().tracks.len()) {
            Some(t) => t,
            None => self.new_track(None),
        };
        let id = self
            .edit("add clip", |s| s.insert_clip(track, clip))
            .expect("track exists");
        self.follow_clip(id);
        self.flash_clip(id);
        id
    }

    /// Asks for a short highlight on a new clip (when animations are on).
    pub fn flash_clip(&mut self, id: u64) {
        if self.animations {
            self.flash = Some((id, self.clock));
        }
    }
}
