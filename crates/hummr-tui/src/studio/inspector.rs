//! The inspector (Enter on a clip): turn it into something else, add
//! effects, fix it. Every change is undoable and, with sound, the clip
//! loops on its own so you hear each change.

use hummr_core::beat::{BeatEstimate, fmt_bpm};
use hummr_core::fx::Preset;
use hummr_core::project::convert::{Target, convert};
use hummr_core::project::ops::{MatchMode, analyze, default_mode, match_tempo};
use hummr_core::project::{Clip, ClipKind, Stretch, Voice};
use hummr_core::slice::resnap;

use super::{Studio, short_instrument};
use crate::app::Level;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    TurnInto,
    Effects,
    Fix,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::TurnInto, Panel::Effects, Panel::Fix];

    pub fn title(self) -> &'static str {
        match self {
            Panel::TurnInto => "Turn into",
            Panel::Effects => "Effects",
            Panel::Fix => "Fix",
        }
    }

    pub fn controls(self) -> &'static [Ctl] {
        match self {
            Panel::TurnInto => &[
                Ctl::TurnNotes,
                Ctl::Instrument,
                Ctl::TurnOwn,
                Ctl::TurnDrums,
                Ctl::TurnNone,
            ],
            Panel::Effects => &[
                Ctl::Preset,
                Ctl::Amount,
                Ctl::Drive,
                Ctl::Reverb,
                Ctl::Echo,
                Ctl::Filter,
                Ctl::Pitch,
                Ctl::Speed,
                Ctl::Volume,
                Ctl::Pan,
                Ctl::FadeIn,
                Ctl::FadeOut,
                Ctl::Reverse,
                Ctl::Loop,
            ],
            Panel::Fix => &[
                Ctl::Clean,
                Ctl::NoiseStart,
                Ctl::NoiseLen,
                Ctl::Match,
                Ctl::Snap,
                Ctl::SnapKey,
                Ctl::Follow,
            ],
        }
    }
}

/// One control in a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctl {
    TurnNotes,
    Instrument,
    TurnOwn,
    TurnDrums,
    TurnNone,
    Preset,
    Amount,
    Drive,
    Reverb,
    Echo,
    Filter,
    Pitch,
    Speed,
    Volume,
    Pan,
    FadeIn,
    FadeOut,
    Reverse,
    Loop,
    Clean,
    NoiseStart,
    NoiseLen,
    Match,
    Snap,
    SnapKey,
    Follow,
}

/// Instruments offered for "turn into notes" (General MIDI programs).
pub const INSTRUMENTS: [u8; 14] = [0, 4, 11, 19, 24, 33, 38, 48, 56, 65, 73, 80, 81, 89];

const MODES: [&str; 3] = ["cut and snap", "stretch", "keep sound"];

/// Inspector state.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspector {
    pub clip: u64,
    pub panel: Panel,
    pub row: usize,
    /// Instrument for "turn into notes".
    pub program: u8,
    /// Match-tempo choice (index into the modes).
    pub mode: usize,
    /// Tempo analysis of the clip, done when first needed.
    pub estimate: Option<BeatEstimate>,
    /// Whether the clip loops on its own.
    pub preview: bool,
}

fn slider(v: f64, lo: f64, hi: f64) -> String {
    let frac = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    let n = (frac * 10.0).round() as usize;
    format!("{}{}", "█".repeat(n), "─".repeat(10 - n))
}

fn pct(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

impl Inspector {
    pub fn control(&self) -> Ctl {
        let c = self.panel.controls();
        c[self.row.min(c.len() - 1)]
    }
}

/// What a control shows: (label, value, hint).
pub fn describe(s: &Studio, ins: &Inspector, ctl: Ctl) -> (String, String, &'static str) {
    let Some(clip) = s.song().clip(ins.clip) else {
        return (String::new(), String::new(), "");
    };
    let is_notes = matches!(clip.kind, ClipKind::Notes { .. });
    let fx = &clip.fx;
    let na = || "—".to_string();
    let current = |t: Target| {
        if Target::of(clip) == t {
            "● now".to_string()
        } else {
            String::new()
        }
    };
    match ctl {
        Ctl::TurnNotes => (
            "Notes on an instrument".into(),
            current(Target::Notes),
            "Enter: follow the pitch and play it on the instrument below",
        ),
        Ctl::Instrument => (
            "  Instrument".into(),
            format!("◂ {} ▸", short_instrument(ins.program)),
            "h/l: choose the instrument",
        ),
        Ctl::TurnOwn => (
            "My own sound as the instrument".into(),
            current(Target::OwnSound),
            "Enter: play this sound at any pitch (e opens the notes)",
        ),
        Ctl::TurnDrums => (
            "Drums".into(),
            current(Target::Drums),
            "Enter: replace each hit with a clean kick, snare or hat",
        ),
        Ctl::TurnNone => (
            "None (the recording)".into(),
            current(Target::None),
            "Enter: back to the recorded sound",
        ),
        Ctl::Preset => (
            "Preset".into(),
            format!("◂ {} ▸", fx.preset.clone().unwrap_or_else(|| "none".into())),
            "h/l: choose a preset",
        ),
        Ctl::Amount => (
            "  Amount".into(),
            format!("{} {}", slider(fx.amount, 0.0, 1.0), pct(fx.amount)),
            "h/l: how much of the preset",
        ),
        Ctl::Drive => (
            "Distortion".into(),
            format!("{} {}", slider(fx.drive, 0.0, 1.0), pct(fx.drive)),
            "h/l: dirt and crunch",
        ),
        Ctl::Reverb => (
            "Reverb".into(),
            format!("{} {}", slider(fx.reverb, 0.0, 1.0), pct(fx.reverb)),
            "h/l: room sound",
        ),
        Ctl::Echo => (
            "Echo".into(),
            format!("{} {}", slider(fx.echo, 0.0, 1.0), pct(fx.echo)),
            "h/l: repeats on the beat",
        ),
        Ctl::Filter => (
            "Filter".into(),
            format!(
                "{} {}",
                slider(fx.filter, -1.0, 1.0),
                if fx.filter < 0.0 {
                    format!("darker {}", pct(-fx.filter))
                } else if fx.filter > 0.0 {
                    format!("thinner {}", pct(fx.filter))
                } else {
                    "open".into()
                }
            ),
            "h/l: darker (left) or thinner (right)",
        ),
        Ctl::Pitch => (
            "Pitch".into(),
            format!("{:+} semitones", clip.pitch.round() as i64),
            "h/l: higher or lower, same speed",
        ),
        Ctl::Speed => (
            "Speed".into(),
            if is_notes {
                na()
            } else {
                format!("{:.2}×", clip.speed)
            },
            "h/l: faster or slower, same pitch",
        ),
        Ctl::Volume => (
            "Volume".into(),
            format!("{:+.0} dB", clip.gain_db),
            "h/l: louder or quieter",
        ),
        Ctl::Pan => (
            "Pan".into(),
            if clip.pan.abs() < 1e-9 {
                "centre".into()
            } else if clip.pan < 0.0 {
                format!("left {}", pct(-clip.pan))
            } else {
                format!("right {}", pct(clip.pan))
            },
            "h/l: left or right",
        ),
        Ctl::FadeIn => (
            "Fade in".into(),
            format!("{:.2} s", clip.fade_in),
            "h/l: fade in length",
        ),
        Ctl::FadeOut => (
            "Fade out".into(),
            format!("{:.2} s", clip.fade_out),
            "h/l: fade out length",
        ),
        Ctl::Reverse => (
            "Reverse".into(),
            if is_notes {
                na()
            } else if clip.reverse {
                "on".into()
            } else {
                "off".into()
            },
            "Enter or h/l: play backwards",
        ),
        Ctl::Loop => (
            "Loop".into(),
            format!("{}×", clip.repeat),
            "h/l: how many times it plays back to back",
        ),
        Ctl::Clean => (
            "Clean noise".into(),
            if is_notes
                && !matches!(
                    &clip.kind,
                    ClipKind::Notes {
                        voice: Voice::Sampler { .. },
                        ..
                    }
                )
            {
                na()
            } else if clip.clean > 0.0 {
                format!("{} {}", slider(clip.clean, 0.0, 1.0), pct(clip.clean))
            } else {
                "off".into()
            },
            "h/l: strength. Removes steady hiss and hum, not voices or music",
        ),
        Ctl::NoiseStart => (
            "  Noise sample from".into(),
            match clip.noise_region {
                Some([a, _]) => format!("{a:.1} s into the clip"),
                None => "the start (first 300 ms)".into(),
            },
            "h/l: where a moment of only background noise starts",
        ),
        Ctl::NoiseLen => (
            "  Noise sample length".into(),
            match clip.noise_region {
                Some([_, l]) => format!("{l:.1} s"),
                None => "0.3 s".into(),
            },
            "h/l: how long that quiet moment is",
        ),
        Ctl::Match => (
            "Match tempo".into(),
            match &ins.estimate {
                Some(e) if e.has_beat() => format!("{} → ◂ {} ▸", e.describe(), MODES[ins.mode]),
                Some(e) => e.describe(),
                None => "Enter to listen for a beat".into(),
            },
            "h/l: how to fit it, Enter: fit it to the song",
        ),
        Ctl::Snap => (
            "Snap to grid".into(),
            match &clip.kind {
                ClipKind::Sliced { strength, .. } => {
                    format!("{} {}", slider(*strength, 0.0, 1.0), pct(*strength))
                }
                ClipKind::Notes { .. } => na(),
                ClipKind::Audio => "Enter: cut at the hits and snap".into(),
            },
            "h/l: how far each hit moves onto the grid",
        ),
        Ctl::SnapKey => (
            "Snap to key".into(),
            if is_notes {
                s.song().key.map_or_else(
                    || "set a key first (:key A minor)".into(),
                    |k| format!("Enter: snap to {k}"),
                )
            } else {
                na()
            },
            "Enter: move off-key notes to the nearest note in the key",
        ),
        Ctl::Follow => (
            "Follow song tempo".into(),
            match (&clip.kind, clip.stretch) {
                (ClipKind::Audio, Stretch::Keep) => "off (keeps its sound)".into(),
                (ClipKind::Audio, Stretch::Follow { bpm }) => {
                    format!("on (recorded at {})", fmt_bpm(bpm))
                }
                _ => na(),
            },
            "Enter: stretch with tempo changes, or keep the sound",
        ),
    }
}

impl Studio {
    /// Enter: opens the inspector on the selected clip.
    pub fn open_inspector(&mut self) {
        let Some(clip) = self.selected_clip().cloned() else {
            self.say(
                "No clip under the playhead on this row. Move onto a clip, then press Enter.",
                Level::Warn,
            );
            return;
        };
        let program = match clip.kind {
            ClipKind::Notes {
                voice: Voice::Instrument { program },
                ..
            } => program,
            _ => 0,
        };
        if self.playing {
            self.toggle_play();
        }
        let ins = Inspector {
            clip: clip.id,
            panel: Panel::TurnInto,
            row: 0,
            program,
            mode: 0,
            estimate: None,
            preview: self.engine.has_sound(),
        };
        self.inspector = Some(ins);
        self.mode = super::Mode::Inspector;
        self.refresh_preview();
        self.guide_step(|g| g.inspected = true);
    }

    pub fn close_inspector(&mut self) {
        self.inspector = None;
        self.mode = super::Mode::Normal;
        self.engine.end_preview();
        self.touch();
    }

    /// Re-renders the looping preview after a change.
    pub fn refresh_preview(&mut self) {
        let Some(ins) = &self.inspector else { return };
        if !ins.preview {
            self.engine.end_preview();
            return;
        }
        if let Some(clip) = self.song().clip(ins.clip).cloned() {
            let song = self.project.song.clone();
            self.engine.preview(&song, &clip, &self.project.sources);
        }
    }

    /// Changes the inspected clip as an undoable step; consecutive changes
    /// to the same control merge into one step.
    fn change_clip(&mut self, label: &str, f: impl FnOnce(&mut Clip)) {
        let Some(id) = self.inspector.as_ref().map(|i| i.clip) else {
            return;
        };
        self.edit_merge(label, |song| {
            if let Some(c) = song.clip_mut(id) {
                f(c);
            }
        });
        self.refresh_preview();
    }

    fn inspected(&self) -> Option<Clip> {
        self.inspector
            .as_ref()
            .and_then(|i| self.song().clip(i.clip).cloned())
    }

    /// Analyses the clip's tempo (once).
    fn ensure_estimate(&mut self) {
        let Some(clip) = self.inspected() else { return };
        if self
            .inspector
            .as_ref()
            .is_some_and(|i| i.estimate.is_some())
        {
            return;
        }
        let Some(audio) = self.project.sources.get(&clip.source).cloned() else {
            return;
        };
        let est = analyze(&clip, &audio);
        let mode = match default_mode(&est, clip.source_len) {
            MatchMode::CutAndSnap { .. } => 0,
            MatchMode::Stretch => 1,
            MatchMode::Keep => 2,
        };
        if let Some(i) = &mut self.inspector {
            i.estimate = Some(est);
            i.mode = mode;
        }
    }

    /// h/l on the current control (`dir` is −1 or +1).
    pub fn inspector_adjust(&mut self, dir: f64) {
        let Some(ins) = self.inspector.clone() else {
            return;
        };
        let Some(clip) = self.inspected() else { return };
        let is_notes = matches!(clip.kind, ClipKind::Notes { .. });
        let step01 = |v: f64| ((v + dir * 0.1) * 10.0).round() / 10.0;
        match ins.control() {
            Ctl::Instrument => {
                let i = INSTRUMENTS
                    .iter()
                    .position(|&p| p == ins.program)
                    .unwrap_or(0) as i64;
                let n = INSTRUMENTS.len() as i64;
                let program = INSTRUMENTS[(i + dir as i64).rem_euclid(n) as usize];
                if let Some(i) = &mut self.inspector {
                    i.program = program;
                }
                if let ClipKind::Notes {
                    voice: Voice::Instrument { .. },
                    ..
                } = clip.kind
                {
                    self.change_clip("instrument", |c| {
                        if let ClipKind::Notes { voice, .. } = &mut c.kind {
                            *voice = Voice::Instrument { program };
                        }
                    });
                }
            }
            Ctl::Preset => {
                let names: Vec<Option<&str>> = std::iter::once(None)
                    .chain(Preset::ALL.iter().map(|p| Some(p.name())))
                    .collect();
                let i = names
                    .iter()
                    .position(|n| n.map(str::to_string) == clip.fx.preset)
                    .unwrap_or(0) as i64;
                let next = names[(i + dir as i64).rem_euclid(names.len() as i64) as usize];
                self.change_clip("preset", |c| {
                    c.fx.preset = next.map(str::to_string);
                    if next.is_some() && c.fx.amount <= 0.0 {
                        c.fx.amount = 0.6;
                    }
                });
            }
            Ctl::Amount => self.change_clip("preset amount", |c| {
                c.fx.amount = step01(c.fx.amount).clamp(0.0, 1.0);
            }),
            Ctl::Drive => self.change_clip("distortion", |c| {
                c.fx.drive = step01(c.fx.drive).clamp(0.0, 1.0)
            }),
            Ctl::Reverb => self.change_clip("reverb", |c| {
                c.fx.reverb = step01(c.fx.reverb).clamp(0.0, 1.0)
            }),
            Ctl::Echo => {
                self.change_clip("echo", |c| c.fx.echo = step01(c.fx.echo).clamp(0.0, 1.0))
            }
            Ctl::Filter => self.change_clip("filter", |c| {
                c.fx.filter = step01(c.fx.filter).clamp(-1.0, 1.0)
            }),
            Ctl::Pitch => {
                self.change_clip("pitch", |c| c.pitch = (c.pitch + dir).clamp(-12.0, 12.0))
            }
            Ctl::Speed if !is_notes => self.change_clip("speed", |c| {
                c.speed = ((c.speed + dir * 0.05) * 20.0).round() / 20.0;
                c.speed = c.speed.clamp(0.5, 2.0);
            }),
            Ctl::Volume => self.change_clip("volume", |c| {
                c.gain_db = (c.gain_db + dir).clamp(-36.0, 12.0)
            }),
            Ctl::Pan => self.change_clip("pan", |c| c.pan = step01(c.pan).clamp(-1.0, 1.0)),
            Ctl::FadeIn => self.change_clip("fade in", |c| {
                c.fade_in = ((c.fade_in + dir * 0.05) * 20.0).round().max(0.0) / 20.0;
            }),
            Ctl::FadeOut => self.change_clip("fade out", |c| {
                c.fade_out = ((c.fade_out + dir * 0.05) * 20.0).round().max(0.0) / 20.0;
            }),
            Ctl::Reverse if !is_notes => self.change_clip("reverse", |c| c.reverse = !c.reverse),
            Ctl::Loop => self.change_clip("loop", |c| {
                c.repeat = (i64::from(c.repeat) + dir as i64).clamp(1, 64) as u32;
            }),
            Ctl::Clean => {
                self.change_clip("clean noise", |c| c.clean = step01(c.clean).clamp(0.0, 1.0))
            }
            Ctl::NoiseStart => self.change_clip("noise sample", |c| {
                let [a, l] = c.noise_region.unwrap_or([0.0, 0.3]);
                let a = ((a + dir * 0.1) * 10.0).round() / 10.0;
                let a = a.clamp(0.0, (c.source_len - l).max(0.0));
                c.noise_region = (a > 0.0 || (l - 0.3).abs() > 1e-9).then_some([a, l]);
            }),
            Ctl::NoiseLen => self.change_clip("noise sample", |c| {
                let [a, l] = c.noise_region.unwrap_or([0.0, 0.3]);
                let l = (((l + dir * 0.1) * 10.0).round() / 10.0).clamp(0.1, 2.0);
                c.noise_region = (a > 0.0 || (l - 0.3).abs() > 1e-9).then_some([a, l]);
            }),
            Ctl::Match => {
                self.ensure_estimate();
                if let Some(i) = &mut self.inspector {
                    i.mode = (i.mode as i64 + dir as i64).rem_euclid(MODES.len() as i64) as usize;
                }
            }
            Ctl::Snap => {
                if let ClipKind::Sliced { grid, strength, .. } = clip.kind {
                    let bpm = self.song().bpm;
                    let st = step01(strength).clamp(0.0, 1.0);
                    self.change_clip("snap strength", |c| resnap(c, grid, st, bpm));
                } else {
                    self.inspector_apply();
                }
            }
            _ => self.inspector_apply(),
        }
    }

    /// Enter on the current control.
    pub fn inspector_apply(&mut self) {
        let Some(ins) = self.inspector.clone() else {
            return;
        };
        let Some(clip) = self.inspected() else { return };
        match ins.control() {
            Ctl::TurnNotes | Ctl::Instrument => self.turn_into(Target::Notes, ins.program),
            Ctl::TurnOwn => self.turn_into(Target::OwnSound, ins.program),
            Ctl::TurnDrums => self.turn_into(Target::Drums, ins.program),
            Ctl::TurnNone => self.turn_into(Target::None, ins.program),
            Ctl::Reverse => self.inspector_adjust(1.0),
            Ctl::Match => self.match_clip(None),
            Ctl::Snap => {
                if matches!(clip.kind, ClipKind::Audio) {
                    let grid = self.song().snap_beats().min(0.5);
                    self.match_clip(Some(MatchMode::CutAndSnap {
                        grid,
                        strength: 1.0,
                    }));
                }
            }
            Ctl::SnapKey => self.snap_clip_to_key(),
            Ctl::Follow => {
                if !matches!(clip.kind, ClipKind::Audio) {
                    return;
                }
                let bpm = self
                    .song()
                    .source(&clip.source)
                    .and_then(|s| s.recorded_bpm)
                    .or_else(|| {
                        self.ensure_estimate();
                        self.inspector
                            .as_ref()
                            .and_then(|i| i.estimate.as_ref())
                            .filter(|e| e.has_beat())
                            .map(|e| e.bpm)
                    });
                match (clip.stretch, bpm) {
                    (Stretch::Follow { .. }, _) => {
                        self.change_clip("keep sound", |c| c.stretch = Stretch::Keep);
                    }
                    (Stretch::Keep, Some(bpm)) => {
                        self.change_clip("follow tempo", |c| c.stretch = Stretch::Follow { bpm });
                    }
                    (Stretch::Keep, None) => self.say(
                        "No steady beat found in this clip, so it can't follow the tempo.",
                        Level::Warn,
                    ),
                }
            }
            _ => {}
        }
    }

    fn turn_into(&mut self, target: Target, program: u8) {
        let Some(clip) = self.inspected() else { return };
        let Some(audio) = self.project.sources.get(&clip.source).cloned() else {
            self.say(
                "This clip's recording is missing, so it can't be converted.",
                Level::Error,
            );
            return;
        };
        let song = self.project.song.clone();
        match convert(&song, &clip, &audio, target) {
            Ok(mut c) => {
                if let ClipKind::Notes {
                    voice: Voice::Instrument { program: p },
                    ..
                } = &mut c.clip.kind
                {
                    *p = program;
                }
                let key = c.key;
                let summary = if target == Target::Notes {
                    c.summary.replace(
                        &hummr_core::instruments::name(0).to_string(),
                        hummr_core::instruments::name(program),
                    )
                } else {
                    c.summary.clone()
                };
                let new = c.clip;
                let label = format!("turn into {}", target.label());
                self.edit(&label, |s| {
                    if let Some(slot) = s.clip_mut(new.id) {
                        *slot = new;
                    }
                    if s.key.is_none() {
                        s.key = key;
                    }
                });
                let extra = key
                    .filter(|_| target == Target::Notes)
                    .map(|k| format!(" The song key is now {k}."))
                    .unwrap_or_default();
                self.say(format!("{summary}{extra}"), Level::Info);
                self.refresh_preview();
            }
            Err(e) => self.say(e.to_string(), Level::Warn),
        }
    }

    /// Fits the inspected clip to the song tempo (`mode` overrides the
    /// inspector's choice).
    pub fn match_clip(&mut self, mode: Option<MatchMode>) {
        self.ensure_estimate();
        let Some(clip) = self.inspected() else { return };
        let Some(ins) = self.inspector.clone() else {
            return;
        };
        let Some(est) = ins.estimate.clone() else {
            return;
        };
        let Some(audio) = self.project.sources.get(&clip.source).cloned() else {
            return;
        };
        let mode = mode.unwrap_or(match ins.mode {
            0 => MatchMode::CutAndSnap {
                grid: self.song().snap_beats().min(0.5),
                strength: 1.0,
            },
            1 => MatchMode::Stretch,
            _ => MatchMode::Keep,
        });
        if !est.has_beat() && mode != MatchMode::Keep {
            self.say(
                "No steady beat found in this clip, so there is nothing to match. It keeps its sound.",
                Level::Warn,
            );
            return;
        }
        let bpm = self.song().bpm;
        let new = match_tempo(&clip, &audio, &est, bpm, mode);
        let label = match mode {
            MatchMode::CutAndSnap { .. } => "cut and snap",
            MatchMode::Stretch => "stretch to tempo",
            MatchMode::Keep => "keep sound",
        };
        self.edit(label, |s| {
            if let Some(slot) = s.clip_mut(new.id) {
                *slot = new;
            }
        });
        self.say(
            match mode {
                MatchMode::CutAndSnap { .. } => format!(
                    "Cut at the hits and placed them on the song's beats ({} → {} BPM, no stretching).",
                    fmt_bpm(est.bpm),
                    fmt_bpm(bpm)
                ),
                MatchMode::Stretch => format!(
                    "Stretched from {} to {} BPM.",
                    fmt_bpm(est.bpm),
                    fmt_bpm(bpm)
                ),
                MatchMode::Keep => "Kept the sound as it is.".into(),
            },
            Level::Info,
        );
        self.refresh_preview();
    }

    /// Moves the inspected notes clip's notes into the song key.
    pub fn snap_clip_to_key(&mut self) {
        let Some(clip) = self.inspected() else { return };
        if !matches!(clip.kind, ClipKind::Notes { .. }) {
            self.say(
                "Snap to key works on notes. Turn the clip into notes first.",
                Level::Info,
            );
            return;
        }
        let Some(key) = self.song().key else {
            self.say("Set the song key first, e.g. :key A minor", Level::Warn);
            return;
        };
        let mut moved = 0;
        if let ClipKind::Notes { notes, .. } = &clip.kind {
            moved = notes.iter().filter(|n| !key.contains(n.pitch)).count();
        }
        self.change_clip("snap to key", |c| {
            if let ClipKind::Notes { notes, .. } = &mut c.kind {
                for n in notes.iter_mut() {
                    n.pitch = key.snap(f32::from(n.pitch)).pitch;
                }
            }
        });
        self.say(
            format!(
                "Moved {moved} note{} into {key}.",
                if moved == 1 { "" } else { "s" }
            ),
            Level::Info,
        );
    }
}
