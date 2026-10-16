//! `i`: the sample picker. Lists the built-in starter samples and the WAV
//! files in the project folder, its `sources/` and the current folder.

use std::path::{Path, PathBuf};

use hummr_core::AudioBuffer;
use hummr_core::kit::Sound;
use hummr_core::project::PROJECT_RATE;

use super::{Mode, Studio};
use crate::app::Level;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Starter(Sound),
    File(PathBuf),
}

impl Item {
    pub fn label(&self) -> String {
        match self {
            Item::Starter(s) => format!("{} (built-in)", s.name()),
            Item::File(p) => p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub items: Vec<Item>,
    pub sel: usize,
}

/// File types the importer can read.
pub fn readable(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        hummr_core::project::file::IMPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
    })
}

fn wavs_in(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && readable(p))
        .collect();
    found.sort();
    for p in found {
        let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        if !out
            .iter()
            .any(|q| q.canonicalize().unwrap_or_else(|_| q.clone()) == canon)
        {
            out.push(p);
        }
    }
}

impl Studio {
    /// Opens the sample picker.
    pub fn open_import(&mut self) {
        let mut files = Vec::new();
        if let Some(dir) = self.project.dir() {
            wavs_in(&dir, &mut files);
            wavs_in(&dir.join("sources"), &mut files);
        }
        if let Ok(cwd) = std::env::current_dir() {
            wavs_in(&cwd, &mut files);
        }
        let items = Sound::ALL
            .iter()
            .map(|s| Item::Starter(*s))
            .chain(files.into_iter().map(Item::File))
            .collect();
        self.mode = Mode::Import(Picker { items, sel: 0 });
    }

    /// Imports the chosen item as a clip at the playhead.
    pub fn import_item(&mut self, item: &Item) -> Option<u64> {
        let track = self.current_track();
        let at = self.playhead;
        match item {
            Item::Starter(sound) => {
                let audio = AudioBuffer::new(sound.render(PROJECT_RATE), PROJECT_RATE);
                Some(self.add_clip_from_audio(sound.name(), audio, None, track, at))
            }
            Item::File(path) => match self.project.import_file(path) {
                Ok(src) => {
                    let name = self
                        .song()
                        .source(&src)
                        .map_or_else(|| "sample".into(), |s| s.name.clone());
                    Some(self.add_clip_of_source(&src, &name, track, at))
                }
                Err(e) => {
                    self.say(
                        format!("Couldn't import {}: {e}", path.display()),
                        Level::Error,
                    );
                    None
                }
            },
        }
    }
}
