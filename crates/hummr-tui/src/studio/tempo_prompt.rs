//! What happens to the tempo when a sample comes in: the first clip in an
//! empty song sets the song's tempo; a later clip at a clearly different
//! tempo (more than 2% off) asks what to do.

use hummr_core::beat::{BeatEstimate, fmt_bpm};
use hummr_core::project::ops::{MatchMode, analyze, is_rhythmic, match_tempo};

use super::{Mode, Studio};
use crate::app::Level;

/// Tempos closer than this (as a fraction) count as the same.
pub const SAME_TEMPO: f64 = 0.02;
/// Clips shorter than this (seconds) are one-shots: no tempo to speak of.
const MIN_SECONDS: f64 = 1.5;

/// The "this sample is at a different tempo" question.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoPrompt {
    pub clip: u64,
    pub estimate: BeatEstimate,
    /// The tempo heard, after any half/double switch.
    pub bpm: f64,
    /// Whether the clip is made of hits (so matching cuts and snaps it
    /// rather than stretching it).
    pub rhythmic: bool,
    /// 0: match to song, 1: change song, 2: leave as is.
    pub sel: usize,
    /// When the prompt opened (`Studio::clock`), for the fade-in.
    pub opened: f64,
}

impl TempoPrompt {
    /// The three choices: (label, what it does).
    pub fn choices(&self, song_bpm: f64) -> [(String, String); 3] {
        [
            (
                "Match to song".into(),
                if self.rhythmic {
                    format!(
                        "cut at the hits and put them on the song's beats ({} BPM)",
                        fmt_bpm(song_bpm)
                    )
                } else {
                    format!("stretch it to {} BPM, same pitch", fmt_bpm(song_bpm))
                },
            ),
            (
                format!("Change song to {}", fmt_bpm(self.bpm)),
                "the whole song takes this sample's tempo".into(),
            ),
            (
                "Leave as is".into(),
                "keep its sound; it may drift against the beat".into(),
            ),
        ]
    }

    /// "It sounds like 118 BPM (sure)."
    pub fn heard(&self) -> String {
        let sure = if self.estimate.is_sure() {
            "sure"
        } else {
            "not sure"
        };
        format!(
            "This sample sounds like {} BPM ({sure}).",
            fmt_bpm(self.bpm)
        )
    }
}

fn differs(a: f64, b: f64) -> bool {
    (a / b - 1.0).abs() > SAME_TEMPO
}

impl Studio {
    /// Called when clip `id` was just added from a sample with no known
    /// tempo. `first` says whether the song had no clips before.
    pub fn check_new_clip_tempo(&mut self, id: u64, first: bool) {
        let Some(clip) = self.song().clip(id).cloned() else {
            return;
        };
        if clip.source_len < MIN_SECONDS {
            return;
        }
        let Some(audio) = self.project.sources.get(&clip.source).cloned() else {
            return;
        };
        let est = analyze(&clip, &audio);
        if !est.has_beat() {
            return;
        }
        let song_bpm = self.song().bpm;
        if first {
            let bpm = (est.bpm * 10.0).round() / 10.0;
            self.edit("tempo from the first clip", |s| s.set_bpm(bpm));
            let msg = if est.is_sure() {
                format!("Song tempo set to {} BPM from this clip.", fmt_bpm(bpm))
            } else {
                let alt = est
                    .alternatives
                    .first()
                    .map(|a| format!(" (or maybe {})", fmt_bpm(*a)))
                    .unwrap_or_default();
                format!(
                    "Song tempo set to {} BPM from this clip, but not sure{alt}. Change it with :bpm or tap it with T.",
                    fmt_bpm(bpm)
                )
            };
            self.say(
                msg,
                if est.is_sure() {
                    Level::Info
                } else {
                    Level::Warn
                },
            );
            return;
        }
        // Half or double the heard tempo may already be the song's.
        if std::iter::once(est.bpm)
            .chain(est.alternatives.iter().copied())
            .any(|b| !differs(b, song_bpm))
        {
            return;
        }
        self.mode = Mode::TempoPrompt(TempoPrompt {
            clip: id,
            bpm: est.bpm,
            rhythmic: is_rhythmic(&est, clip.source_len),
            estimate: est,
            sel: 0,
            opened: self.clock,
        });
    }

    /// `h`/`d` in the prompt: the heard tempo is half or double.
    pub fn prompt_scale(&mut self, factor: f64) {
        if let Mode::TempoPrompt(p) = &mut self.mode {
            let bpm = p.bpm * factor;
            if (20.0..=400.0).contains(&bpm) {
                p.bpm = bpm;
            }
        }
    }

    /// Enter in the prompt.
    pub fn prompt_choose(&mut self, choice: usize) {
        let Mode::TempoPrompt(p) = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        let song_bpm = self.song().bpm;
        match choice {
            0 => {
                let Some(clip) = self.song().clip(p.clip).cloned() else {
                    return;
                };
                let Some(audio) = self.project.sources.get(&clip.source).cloned() else {
                    return;
                };
                let mut est = p.estimate.clone();
                est.bpm = p.bpm;
                let mode = if p.rhythmic {
                    MatchMode::CutAndSnap {
                        grid: self.song().snap_beats().min(0.5),
                        strength: 1.0,
                    }
                } else {
                    MatchMode::Stretch
                };
                let new = match_tempo(&clip, &audio, &est, song_bpm, mode);
                self.edit("match tempo", |s| {
                    if let Some(slot) = s.clip_mut(new.id) {
                        *slot = new;
                    }
                });
                self.follow_clip(p.clip);
                self.say(
                    if p.rhythmic {
                        format!(
                            "Cut at the hits and put them on the beat at {} BPM (u to undo).",
                            fmt_bpm(song_bpm)
                        )
                    } else {
                        format!(
                            "Stretched from {} to {} BPM (u to undo).",
                            fmt_bpm(p.bpm),
                            fmt_bpm(song_bpm)
                        )
                    },
                    Level::Info,
                );
            }
            1 => self.set_bpm(p.bpm),
            _ => self.say(
                "Left as is. Enter → Fix → Match tempo fits it later.",
                Level::Info,
            ),
        }
    }
}
