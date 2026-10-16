//! The timeline screen ("studio"): tracks are rows, clips are boxes on a
//! beat grid. This is the default screen; the old piano-roll editor is the
//! notes editor, opened for notes clips.

pub mod draw;
#[doc(hidden)]
pub mod fixture;
pub mod peaks;

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
    pub quit: bool,
    /// Seconds since start, for blinking and toasts.
    pub clock: f64,
}

/// Options for opening the timeline.
#[derive(Clone, Debug, Default)]
pub struct StudioOptions {
    pub animations: bool,
    pub mouse: bool,
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
            quit: false,
            clock: 0.0,
        };
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

    pub fn tick(&mut self, dt: Duration) {
        self.clock += dt.as_secs_f64();
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
