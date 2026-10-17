//! Editing the song header: tempo (typed, nudged or tapped), time
//! signature, key and snap. Every change is one undo step.

use hummr_core::beat::fmt_bpm;
use hummr_core::project::TimeSig;
use hummr_core::rhythm::Grid;
use hummr_core::theory::Key;

use super::Studio;
use crate::app::Level;

/// Taps further apart than this start a new count.
const TAP_GAP: f64 = 2.0;
/// Taps remembered (the tempo is the median gap between them).
const TAP_KEEP: usize = 8;

impl Studio {
    /// Seconds on a steady clock (a fixed time in tests).
    pub fn now(&self) -> f64 {
        self.fake_now
            .unwrap_or_else(|| self.started.elapsed().as_secs_f64())
    }

    /// Sets the tempo. Sliced and notes clips keep their beats; audio clips
    /// keep their sound unless they follow the tempo. Playback carries on
    /// from the same beat.
    pub fn set_bpm(&mut self, bpm: f64) {
        self.last_merge = None;
        self.change_bpm(bpm);
        self.last_merge = None;
    }

    /// Like [`Studio::set_bpm`], but merges with the previous tempo step.
    fn change_bpm(&mut self, bpm: f64) {
        let old = self.song().bpm;
        let bpm = (bpm * 10.0).round() / 10.0;
        self.edit_merge("tempo", |s| s.set_bpm(bpm));
        let new = self.song().bpm;
        if (new - old).abs() > 1e-9 {
            self.seek(self.playhead);
        }
        let note = if (new - bpm).abs() > 1e-9 {
            " (tempos go from 20 to 400)"
        } else {
            ""
        };
        self.say(format!("Tempo {} BPM{note}.", fmt_bpm(new)), Level::Info);
    }

    /// `<` / `>`: one BPM down or up (consecutive presses are one undo step).
    pub fn nudge_bpm(&mut self, dir: f64) {
        let bpm = self.song().bpm.round() + dir;
        self.change_bpm(bpm);
    }

    /// `T`: tap the tempo. From the third tap on, the tempo is the median
    /// gap between the recent taps.
    pub fn tap_tempo(&mut self) {
        let now = self.now();
        if self.taps.last().is_some_and(|&t| now - t > TAP_GAP) {
            self.taps.clear();
        }
        self.taps.push(now);
        if self.taps.len() > TAP_KEEP {
            self.taps.remove(0);
        }
        if self.taps.len() < 3 {
            self.say(
                format!("Tap {} — keep tapping in time (T).", self.taps.len()),
                Level::Info,
            );
            return;
        }
        let mut gaps: Vec<f64> = self.taps.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(f64::total_cmp);
        let gap = gaps[gaps.len() / 2];
        if gap <= 0.0 {
            return;
        }
        let bpm = (60.0 / gap).clamp(20.0, 400.0).round();
        self.edit_merge("tap tempo", |s| s.set_bpm(bpm));
        self.seek(self.playhead);
        self.say(
            format!(
                "Tapped {} BPM ({} taps). Keep tapping to refine, u to undo.",
                fmt_bpm(bpm),
                self.taps.len()
            ),
            Level::Info,
        );
    }

    /// `:bpm 120`, `:bpm half`, `:bpm double`.
    pub fn bpm_command(&mut self, arg: &str) {
        let bpm = self.song().bpm;
        match arg.trim() {
            "half" => self.set_bpm(bpm / 2.0),
            "double" => self.set_bpm(bpm * 2.0),
            "" => self.say(
                format!(
                    "Tempo is {} BPM. Use :bpm 120, :bpm half or :bpm double.",
                    fmt_bpm(bpm)
                ),
                Level::Info,
            ),
            a => match a.parse::<f64>() {
                Ok(v) if v.is_finite() && v > 0.0 => self.set_bpm(v),
                _ => self.say(
                    "Use a number of beats per minute, e.g. :bpm 96, or :bpm half / double.",
                    Level::Error,
                ),
            },
        }
    }

    /// `:time 3/4`.
    pub fn time_command(&mut self, arg: &str) {
        match arg.parse::<TimeSig>() {
            Ok(ts) => {
                self.edit("time signature", |s| s.time_sig = ts);
                self.say(format!("Time signature {ts}."), Level::Info);
            }
            Err(_) => self.say(
                "Use a time signature like :time 4/4, :time 3/4 or :time 6/8.",
                Level::Error,
            ),
        }
    }

    /// `:key A minor`, `:key none`.
    pub fn key_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if matches!(arg, "none" | "off" | "-") {
            self.edit("key", |s| s.key = None);
            self.say("No key: snap to key is off.", Level::Info);
            return;
        }
        match arg.parse::<Key>() {
            Ok(k) => {
                self.edit("key", |s| s.key = Some(k));
                self.say(format!("Key {k}."), Level::Info);
            }
            Err(_) => self.say(
                "Use a key like :key A minor, :key C, :key F#m, or :key none.",
                Level::Error,
            ),
        }
    }

    /// `:snap 1/8`, `:snap off`.
    pub fn snap_command(&mut self, arg: &str) {
        let arg = arg.trim();
        if matches!(arg, "off" | "none") {
            self.edit("snap", |s| s.snap = None);
            self.say(
                "Snap off: clips go exactly where you put them; h/l move one beat.",
                Level::Info,
            );
            return;
        }
        match arg.parse::<Grid>() {
            Ok(g) => {
                self.edit("snap", |s| s.snap = Some(g));
                self.say(format!("Snap {g}."), Level::Info);
            }
            Err(_) => self.say(
                "Use a note length: :snap 1/4, 1/8, 1/16, 1/8t (triplets) or off.",
                Level::Error,
            ),
        }
    }
}
