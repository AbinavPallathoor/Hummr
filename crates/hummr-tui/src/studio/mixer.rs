//! The mixer: one strip per track (level meter, volume, pan, mute, solo)
//! plus the master meter. `M` shows it; volume and pan change live,
//! without re-rendering anything.

use hummr_core::dsp::amp_to_db;

use super::{Mode, Studio};
use crate::app::Level;

/// Lines the mixer takes at the bottom of the track area.
pub const MIXER_H: u16 = 5;
/// Width of one strip.
pub const STRIP_W: u16 = 14;
/// Quietest volume (dB).
pub const MIN_DB: f64 = -60.0;
pub const MAX_DB: f64 = 12.0;
/// Meter range (dB) and fall rate (dB per second).
pub const METER_FLOOR_DB: f32 = -48.0;
const METER_FALL_DB: f32 = 24.0;

/// A level meter's lit cells out of `cells` for a linear peak.
pub fn meter_cells(peak: f32, cells: usize) -> usize {
    if peak <= 0.0 {
        return 0;
    }
    let db = amp_to_db(peak).max(METER_FLOOR_DB);
    let frac = (db - METER_FLOOR_DB) / -METER_FLOOR_DB;
    ((frac * cells as f32).round() as usize).min(cells)
}

/// "C", "L40", "R20".
pub fn fmt_pan(pan: f64) -> String {
    let p = (pan * 100.0).round() as i64;
    match p {
        0 => "C".into(),
        p if p < 0 => format!("L{}", -p),
        p => format!("R{p}"),
    }
}

/// "+2 dB", "-60 dB".
pub fn fmt_db(db: f64) -> String {
    if db.abs() < 0.05 {
        "0 dB".into()
    } else {
        format!("{db:+.0} dB")
    }
}

impl Studio {
    /// Takes the player's peaks into the meters; meters fall smoothly.
    pub(super) fn tick_meters(&mut self, dt: f64) {
        let (peaks, master) = self.engine.take_peaks();
        let fall = 10f32.powf(-METER_FALL_DB * dt as f32 / 20.0);
        let n = self.song().tracks.len();
        self.meters.resize(n, 0.0);
        for (i, m) in self.meters.iter_mut().enumerate() {
            *m = (*m * fall).max(peaks.get(i).copied().unwrap_or(0.0));
        }
        self.master_meter = (self.master_meter * fall).max(master);
    }

    /// `M`: shows the mixer and puts the keys on it, or hides it.
    pub fn toggle_mixer(&mut self) {
        if self.show_mixer {
            self.show_mixer = false;
            self.mode = Mode::Normal;
        } else {
            self.show_mixer = true;
            self.mode = Mode::Mixer;
            self.say(
                "Mixer: h/l track · j/k volume · [ ] pan · m mute · S solo · 0 reset · Esc back to the clips",
                Level::Info,
            );
        }
    }

    /// Selects the next (`+1`) or previous (`-1`) track.
    pub fn select_track(&mut self, dir: i64) {
        let n = self.song().tracks.len() as i64;
        let Some(t) = self.current_track() else {
            return;
        };
        let to = (t as i64 + dir).clamp(0, (n - 1).max(0)) as usize;
        if let Some(r) = self.rows().iter().position(|r| r.track == to) {
            self.row = r;
        }
    }

    /// Volume of the selected track by `db` (repeats are one undo step).
    pub fn nudge_volume(&mut self, db: f64) {
        let Some(t) = self.current_track() else {
            return;
        };
        self.edit_merge("volume", |s| {
            let v = &mut s.tracks[t].volume_db;
            *v = (v.round() + db).clamp(MIN_DB, MAX_DB);
        });
    }

    /// Pan of the selected track by `step` (−1 left … +1 right).
    pub fn nudge_pan(&mut self, step: f64) {
        let Some(t) = self.current_track() else {
            return;
        };
        self.edit_merge("pan", |s| {
            let p = &mut s.tracks[t].pan;
            *p = (((*p + step) * 10.0).round() / 10.0).clamp(-1.0, 1.0);
        });
    }

    /// Volume to 0 dB and pan to the centre.
    pub fn reset_strip(&mut self) {
        let Some(t) = self.current_track() else {
            return;
        };
        self.edit("reset volume and pan", |s| {
            s.tracks[t].volume_db = 0.0;
            s.tracks[t].pan = 0.0;
        });
    }
}
