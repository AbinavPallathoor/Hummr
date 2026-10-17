//! The timeline screen ("studio"): tracks are rows, clips are boxes on a
//! beat grid. This is the default screen; the old piano-roll editor is the
//! notes editor, opened for notes clips.

pub mod command;
pub mod draw;
pub mod edit;
pub mod engine;
#[doc(hidden)]
pub mod fixture;
pub mod import;
pub mod inspector;
pub mod keys;
pub mod peaks;
pub mod record;

use std::time::Duration;

use hummr_core::instruments;
use hummr_core::project::{Clip, ClipKind, History, Project, Song, Voice};

use crate::app::Level;

/// Which screen is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Timeline,
    Notes,
}

/// What the keyboard is doing on the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Normal,
    /// Typing a `:` command.
    Command(String),
    Help,
    /// Recording (see `Studio::recording`).
    Recording,
    /// Picking a sample to import.
    Import(import::Picker),
    /// The clip inspector (see `Studio::inspector`).
    Inspector,
}

/// One visible row: a track's lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub track: usize,
    pub lane: usize,
}

/// Horizontal view: which beat is at the left edge and how wide a beat is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub start_beat: f64,
    pub cols_per_beat: f64,
}

/// Zoom steps (columns per beat).
pub const ZOOMS: [f64; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// Lines per lane: box top, waveform, box bottom.
pub const LANE_H: u16 = 3;
/// Width of the track-name column.
pub const HEADER_W: u16 = 10;

impl Default for View {
    fn default() -> Self {
        Self {
            start_beat: 0.0,
            cols_per_beat: 4.0,
        }
    }
}

impl View {
    pub fn beat_to_col(&self, beat: f64) -> f64 {
        (beat - self.start_beat) * self.cols_per_beat
    }

    pub fn col_to_beat(&self, col: f64) -> f64 {
        self.start_beat + col / self.cols_per_beat
    }
}

/// Everything the timeline needs.
pub struct Studio {
    pub project: Project,
    pub history: History,
    pub screen: Screen,
    pub mode: Mode,
    /// The playhead (beats). When stopped, this is where playback and
    /// recording start.
    pub playhead: f64,
    /// Selected visible row.
    pub row: usize,
    /// First visible row.
    pub row_scroll: usize,
    pub view: View,
    /// Width of the clip area at the last draw (for scrolling and fitting).
    pub lane_cols: u16,
    /// Visible rows at the last draw.
    pub visible_rows: usize,
    pub playing: bool,
    pub message: Option<(String, Level)>,
    pub animations: bool,
    pub peaks: peaks::PeakCache,
    pub clipboard: Option<Clip>,
    pub recording: Option<record::Recording>,
    pub inspector: Option<inspector::Inspector>,
    /// Label of the last edit made with `edit_merge`, so repeated slider
    /// steps become one undo step.
    pub(crate) last_merge: Option<String>,
    /// A clip to highlight briefly (id, clock when added).
    pub flash: Option<(u64, f64)>,
    pub engine: engine::Engine,
    /// Bumped on every change to the song, so the engine knows to sync.
    pub version: u64,
    synced: Option<u64>,
    /// The song as last saved (to warn about unsaved changes).
    saved: Song,
    /// Whether `q` was already pressed once with unsaved changes.
    quit_armed: bool,
    pub quit: bool,
    /// Seconds since start, for blinking and toasts.
    pub clock: f64,
}

/// Options for opening the timeline.
#[derive(Clone, Debug, Default)]
pub struct StudioOptions {
    pub animations: bool,
    pub mouse: bool,
    /// Open the sound device (off in tests).
    pub audio: bool,
}

impl Studio {
    pub fn new(project: Project, opts: &StudioOptions) -> Self {
        let mut s = Self {
            project,
            history: History::new(),
            screen: Screen::Timeline,
            mode: Mode::Normal,
            playhead: 0.0,
            row: 0,
            row_scroll: 0,
            view: View::default(),
            lane_cols: 64,
            visible_rows: 4,
            playing: false,
            message: None,
            animations: opts.animations,
            peaks: peaks::PeakCache::default(),
            clipboard: None,
            recording: None,
            inspector: None,
            last_merge: None,
            flash: None,
            engine: if opts.audio {
                engine::Engine::open()
            } else {
                engine::Engine::headless()
            },
            version: 0,
            synced: None,
            saved: Song::new(""),
            quit_armed: false,
            quit: false,
            clock: 0.0,
        };
        s.saved = s.project.song.clone();
        if let Some(p) = s.engine.problem.clone() {
            s.say(p, Level::Warn);
        }
        let warnings = std::mem::take(&mut s.project.warnings);
        if let Some(w) = warnings.first() {
            s.say(w.clone(), Level::Warn);
        }
        s
    }

    pub fn song(&self) -> &Song {
        &self.project.song
    }

    pub fn say(&mut self, text: impl Into<String>, level: Level) {
        self.message = Some((text.into(), level));
    }

    /// Per-frame work: sync and poll the engine, follow the playhead.
    pub fn tick(&mut self, dt: Duration) {
        self.clock += dt.as_secs_f64();
        if self.synced != Some(self.version) {
            self.engine.sync(&self.project.song, &self.project.sources);
            self.synced = Some(self.version);
            let ids: Vec<u64> = self
                .project
                .song
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter().map(|c| c.id))
                .collect();
            self.peaks.retain(&ids);
        }
        for (clip, bpm, audio) in self.engine.poll(&self.project.song) {
            self.peaks.rendered.insert(clip.id, (clip, bpm, audio));
        }
        self.tick_recording();
        self.engine.advance(dt.as_secs_f64());
        if self.playing {
            if self.engine.is_playing() {
                self.playhead = self.engine.position_beats(&self.project.song);
            } else {
                self.playing = false;
            }
        }
    }

    /// Whether the screen needs redrawing every frame.
    pub fn is_live(&self) -> bool {
        self.playing || self.recording.is_some() || !self.engine.is_ready()
    }

    /// Changes the song as one undoable step called `label`.
    pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Song) -> R) -> R {
        let before = self.project.song.clone();
        let r = f(&mut self.project.song);
        if self.project.song != before {
            self.history.record(label, before);
            self.touch();
        }
        self.last_merge = None;
        r
    }

    /// Like [`Studio::edit`], but repeated edits with the same label in a
    /// row (slider steps) are undone together.
    pub fn edit_merge(&mut self, label: &str, f: impl FnOnce(&mut Song)) {
        if self.last_merge.as_deref() == Some(label) {
            let before = self.project.song.clone();
            f(&mut self.project.song);
            if self.project.song != before {
                self.touch();
            }
        } else {
            self.edit(label, f);
            self.last_merge = Some(label.to_string());
        }
    }

    /// Marks the song changed (for the engine and the unsaved warning).
    pub fn touch(&mut self) {
        self.version += 1;
        self.quit_armed = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.project.song != self.saved || !self.project.sources_saved()
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.project.song.clone();
    }

    /// Quits, or warns once if there are unsaved changes.
    pub fn request_quit(&mut self, force: bool) {
        if force || !self.is_dirty() || self.quit_armed {
            self.engine.stop();
            self.quit = true;
        } else {
            self.quit_armed = true;
            self.say(
                "You have unsaved changes. Press q again to quit without saving, or :w to save.",
                Level::Warn,
            );
        }
    }

    // ---- moving around ------------------------------------------------------

    /// Moves the playhead to `beat` (not before 0), seeking if playing.
    pub fn seek(&mut self, beat: f64) {
        self.playhead = beat.max(0.0);
        if self.playing {
            let song = self.project.song.clone();
            self.engine.play(&song, self.playhead);
        }
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.engine.stop();
            self.playing = false;
        } else if self.song().tracks.iter().all(|t| t.clips.is_empty()) {
            self.say(
                "Nothing to play yet. Press r to record, or i to import a sample.",
                Level::Info,
            );
        } else {
            let song = self.project.song.clone();
            if self.playhead >= song.end_beats() - 1e-9 {
                self.playhead = 0.0;
            }
            self.engine.play(&song, self.playhead);
            self.playing = true;
            if !self.engine.is_ready() {
                self.say("Preparing the sound…", Level::Info);
            }
        }
    }

    pub fn move_row(&mut self, delta: i64) {
        let n = self.rows().len() as i64;
        if n == 0 {
            return;
        }
        self.row = (self.row as i64 + delta).clamp(0, n - 1) as usize;
    }

    /// Jumps to the previous (`-1`) or next (`+1`) bar line.
    pub fn bar_jump(&mut self, dir: i64) {
        let bar = self.song().bar_beats();
        let at = self.playhead / bar;
        let target = if dir < 0 {
            let f = at.floor();
            if (at - f).abs() < 1e-9 { f - 1.0 } else { f }
        } else {
            at.floor() + 1.0
        };
        self.seek(target * bar);
    }

    /// Zooms in (`+1`) or out (`-1`) around the playhead.
    pub fn zoom(&mut self, dir: i64) {
        let i = ZOOMS
            .iter()
            .position(|&z| z >= self.view.cols_per_beat - 1e-9)
            .unwrap_or(ZOOMS.len() - 1) as i64;
        let j = (i + dir).clamp(0, ZOOMS.len() as i64 - 1) as usize;
        let col = self.view.beat_to_col(self.playhead);
        self.view.cols_per_beat = ZOOMS[j];
        self.view.start_beat = (self.playhead - col / self.view.cols_per_beat).max(0.0);
    }

    /// Zooms so the whole song fits.
    pub fn fit(&mut self) {
        let end = self.song().end_beats().max(self.song().bar_beats());
        let cols = f64::from(self.lane_cols.max(8)) - 1.0;
        self.view.cols_per_beat = ZOOMS
            .iter()
            .rev()
            .copied()
            .find(|z| end * z <= cols)
            .unwrap_or(ZOOMS[0]);
        self.view.start_beat = 0.0;
    }

    /// All visible rows, top to bottom: every lane of every track.
    pub fn rows(&self) -> Vec<Row> {
        let song = self.song();
        (0..song.tracks.len())
            .flat_map(|t| (0..song.lane_count(t)).map(move |lane| Row { track: t, lane }))
            .collect()
    }

    /// The selected row, if there are any tracks.
    pub fn current_row(&self) -> Option<Row> {
        let rows = self.rows();
        rows.get(self.row.min(rows.len().saturating_sub(1)))
            .copied()
    }

    /// The selected track index.
    pub fn current_track(&self) -> Option<usize> {
        self.current_row().map(|r| r.track)
    }

    /// The clip under the playhead on the selected row.
    pub fn selected_clip(&self) -> Option<&Clip> {
        let row = self.current_row()?;
        let song = self.song();
        let lanes = song.lanes(row.track);
        song.tracks[row.track]
            .clips
            .iter()
            .zip(lanes)
            .find(|(c, lane)| {
                *lane == row.lane
                    && c.start_beat <= self.playhead + 1e-9
                    && self.playhead < c.end_beat(song.bpm) - 1e-9
            })
            .map(|(c, _)| c)
    }

    pub fn selected_clip_id(&self) -> Option<u64> {
        self.selected_clip().map(|c| c.id)
    }

    /// Keeps the selected row valid and visible, and the playhead in view.
    pub fn clamp_view(&mut self) {
        let n = self.rows().len();
        self.row = self.row.min(n.saturating_sub(1));
        let vis = self.visible_rows.max(1);
        if self.row < self.row_scroll {
            self.row_scroll = self.row;
        } else if self.row >= self.row_scroll + vis {
            self.row_scroll = self.row + 1 - vis;
        }
        self.row_scroll = self.row_scroll.min(n.saturating_sub(vis));
        let cols = f64::from(self.lane_cols.max(1));
        let col = self.view.beat_to_col(self.playhead);
        if col < 0.0 || col >= cols - 1.0 {
            // Page so the playhead sits a quarter of the way in.
            let start = self.playhead - cols * 0.25 / self.view.cols_per_beat;
            let bar = self.song().bar_beats();
            self.view.start_beat = ((start / bar).floor() * bar).max(0.0);
        }
    }
}

/// The label in a clip's box: its name, plus what it was turned into.
pub fn clip_label(clip: &Clip) -> String {
    match &clip.kind {
        ClipKind::Notes { voice, .. } => {
            let to = match voice {
                Voice::Instrument { program } => short_instrument(*program),
                Voice::Sampler { .. } => "own sound".into(),
                Voice::Drums => "drums".into(),
            };
            format!("{} → {to}", clip.name)
        }
        _ => clip.name.clone(),
    }
}

/// A short lower-case instrument name for clip boxes ("piano", "violin").
pub fn short_instrument(program: u8) -> String {
    let name = instruments::name(program).to_ascii_lowercase();
    let drop = [
        "acoustic ",
        "electric ",
        " (nylon)",
        " (steel)",
        " (jazz)",
        " (clean)",
        " (muted)",
        " (finger)",
        " (pick)",
        "bright ",
        "grand ",
        " 1",
        " 2",
    ];
    let mut s = name;
    for d in drop {
        s = s.replace(d, "");
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests;
