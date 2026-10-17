//! Starting with no file: open the project in this folder, or a starter
//! song with a beat to hum over and a three-step guide.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hummr_core::AudioBuffer;
use hummr_core::project::file::slug;
use hummr_core::project::{Clip, PROJECT_RATE, Project};
use hummr_core::synthetic::drum_groove;

use super::Studio;
use crate::app::Level;

/// Name of the song made when there is none.
pub const STARTER_NAME: &str = "My Song";

/// `.hummr` files in `dir` and its direct subfolders (where `hummr new`
/// puts them), most recently changed first.
pub fn find_projects(dir: &Path) -> Vec<PathBuf> {
    let is_project = |p: &Path| p.is_file() && p.extension().is_some_and(|e| e == "hummr");
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if is_project(&p) {
                found.push(p);
            } else if p.is_dir() {
                if let Ok(inner) = std::fs::read_dir(&p) {
                    found.extend(inner.flatten().map(|e| e.path()).filter(|p| is_project(p)));
                }
            }
        }
    }
    let modified = |p: &PathBuf| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };
    found.sort_by(|a, b| modified(b).cmp(&modified(a)).then(a.cmp(b)));
    found
}

/// A new song with a two-bar beat (playing twice) on "Beat" and an empty
/// "Voice" track to record on. Not written to disk until saved; it will be
/// saved as `dir/my-song/my-song.hummr`.
pub fn starter_project(dir: &Path) -> Project {
    let mut p = Project::new(STARTER_NAME);
    p.song.bpm = 96.0;
    let (beat, _) = drum_groove(96.0, 2, 0, 0.0, 0.0, 1, PROJECT_RATE);
    let id = p.add_audio(
        "starter beat",
        AudioBuffer::new(beat, PROJECT_RATE),
        Some(96.0),
    );
    let t = p.song.add_track("Beat");
    let mut clip = Clip::audio(0, "starter beat", &id, 0.0, 8.0 * 60.0 / 96.0, 0.0);
    clip.repeat = 2;
    p.song.insert_clip(t, clip);
    p.song.add_track("Voice");
    let name = slug(STARTER_NAME);
    p.path = Some(dir.join(&name).join(format!("{name}.hummr")));
    p
}

/// What `hummr` with no arguments opens in `dir`: the most recent project
/// there, else a starter song. Returns the project, a message, and whether
/// it is the starter (which shows the guide).
pub fn open_or_create(dir: &Path) -> (Project, String, bool) {
    let found = find_projects(dir);
    for path in &found {
        if let Ok(p) = Project::open(path) {
            let shown = path.strip_prefix(dir).unwrap_or(path).display().to_string();
            let others = found.len() - 1;
            let msg = if others == 0 {
                format!("Opened {shown}. Press ? for help.")
            } else {
                format!(
                    "Opened {shown}, the most recent of {} projects here. Open another with: hummr FILE.hummr",
                    found.len()
                )
            };
            return (p, msg, false);
        }
    }
    let p = starter_project(dir);
    let shown = p
        .path
        .as_ref()
        .and_then(|f| f.strip_prefix(dir).ok())
        .map(|f| f.display().to_string())
        .unwrap_or_default();
    (
        p,
        format!("A new song with a beat to start from. :w saves it as {shown}."),
        true,
    )
}

/// The first-run guide: three steps, ticked off as they are done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guide {
    pub recorded: bool,
    pub played: bool,
    pub inspected: bool,
}

impl Guide {
    pub fn steps(&self) -> [(bool, &'static str); 3] {
        [
            (self.recorded, "Record: press r, then hum, sing or beatbox"),
            (self.played, "Press space to play it with the beat"),
            (self.inspected, "Press Enter on a clip to change its sound"),
        ]
    }

    pub fn done(&self) -> bool {
        self.recorded && self.played && self.inspected
    }
}

impl Studio {
    /// Ticks off guide steps; hides the guide when all are done.
    pub fn guide_step(&mut self, f: impl FnOnce(&mut Guide)) {
        let Some(g) = &mut self.guide else { return };
        f(g);
        if g.done() {
            self.guide = None;
            self.say(
                "That's the basics. Press ? any time to see every key.",
                Level::Info,
            );
        }
    }
}
