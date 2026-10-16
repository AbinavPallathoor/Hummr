//! Project files: a human-readable TOML `*.hummr` file next to a `sources/`
//! folder of WAV files.
//!
//! Imported files are copied byte for byte (originals are never touched);
//! recordings are written as 32-bit float WAV. Paths inside the project file
//! are relative, so a project folder can be moved or shared as a whole.
//! Saving writes a temporary file and renames it over the old one, so a
//! crash mid-save never leaves a half-written project.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{FORMAT, PROJECT_RATE, Song, SourceBank, SourceInfo};
use crate::io::wav;
use crate::resample::resample;
use crate::{AudioBuffer, Error, Result};

/// A song plus its decoded sources and where it is saved.
#[derive(Clone, Debug)]
pub struct Project {
    pub song: Song,
    pub sources: SourceBank,
    /// The `.hummr` file, once known.
    pub path: Option<PathBuf>,
    /// Sources not yet written into `sources/`.
    pending: HashMap<String, PendingSource>,
    /// Problems found while opening (e.g. a missing source file).
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
enum PendingSource {
    /// Copy this original file.
    Copy(PathBuf),
    /// Write this audio (at its own rate).
    Write(Arc<AudioBuffer>),
}

/// Serialises a song to the project file text.
pub fn to_toml(song: &Song) -> Result<String> {
    toml::to_string(song).map_err(|e| Error::Project(format!("could not write project: {e}")))
}

/// Parses project file text.
pub fn from_toml(text: &str) -> Result<Song> {
    let song: Song = toml::from_str(text).map_err(|e| {
        Error::Project(format!(
            "this is not a valid Hummr project file: {}",
            e.to_string().trim()
        ))
    })?;
    if song.format > FORMAT {
        return Err(Error::Project(format!(
            "this project was saved by a newer Hummr (format {}, this one reads format {FORMAT}); \
             update Hummr to open it",
            song.format
        )));
    }
    if !(20.0..=400.0).contains(&song.bpm) {
        return Err(Error::Project(format!(
            "the project says {} BPM; it must be between 20 and 400",
            song.bpm
        )));
    }
    Ok(song)
}

/// Writes `bytes` to `path` atomically (temp file in the same folder, then
/// rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .map_or_else(String::new, |e| e.to_string_lossy().into_owned())
    ));
    std::fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(path, e)
    })
}

/// A file-name-safe version of `name`.
pub fn slug(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "sound".into() } else { s }
}

/// Reads a WAV and converts it to the project sample rate.
pub fn decode_to_project_rate(path: &Path) -> Result<AudioBuffer> {
    let a = wav::read_mono(path)?;
    Ok(to_project_rate(&a))
}

pub fn to_project_rate(a: &AudioBuffer) -> AudioBuffer {
    if a.sample_rate == PROJECT_RATE {
        return a.clone();
    }
    AudioBuffer::new(
        resample(&a.samples, a.sample_rate, PROJECT_RATE),
        PROJECT_RATE,
    )
}

impl Project {
    /// A new, unsaved project.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            song: Song::new(name),
            sources: SourceBank::default(),
            path: None,
            pending: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Folder the project file lives in.
    pub fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .map(|p| p.parent().map_or_else(PathBuf::new, Path::to_path_buf))
    }

    /// Creates `NAME/NAME.hummr` (with an empty `sources/`) under `parent`.
    /// Fails if the project already exists.
    pub fn create_in(parent: &Path, name: &str) -> Result<Self> {
        let dir = parent.join(slug(name));
        let path = dir.join(format!("{}.hummr", slug(name)));
        if path.exists() {
            return Err(Error::Project(format!(
                "{} already exists; open it with `hummr {}`",
                path.display(),
                path.display()
            )));
        }
        std::fs::create_dir_all(dir.join("sources")).map_err(|e| Error::io(&dir, e))?;
        let mut p = Project::new(name);
        p.path = Some(path);
        p.save()?;
        Ok(p)
    }

    /// Opens a project file and decodes its sources. Missing or unreadable
    /// sources become silence and are listed in `warnings`.
    pub fn open(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let song = from_toml(&text)?;
        let dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        let mut sources = SourceBank::default();
        let mut warnings = Vec::new();
        for s in &song.sources {
            let file = dir.join(&s.file);
            match decode_to_project_rate(&file) {
                Ok(a) => sources.insert(s.id.clone(), Arc::new(a)),
                Err(e) => {
                    warnings.push(format!(
                        "sound '{}' could not be loaded and will be silent: {e}",
                        s.name
                    ));
                    sources.insert(
                        s.id.clone(),
                        Arc::new(AudioBuffer::new(Vec::new(), PROJECT_RATE)),
                    );
                }
            }
        }
        Ok(Self {
            song,
            sources,
            path: Some(path.to_path_buf()),
            pending: HashMap::new(),
            warnings,
        })
    }

    fn new_source(&mut self, name: &str, recorded_bpm: Option<f64>) -> SourceInfo {
        let id = format!("s{}", self.song.alloc_id());
        SourceInfo {
            file: format!("sources/{id}-{}.wav", slug(name)),
            id,
            name: name.to_string(),
            recorded_bpm,
        }
    }

    /// Imports a WAV file as a new source (copied into `sources/` on save).
    /// Returns the source id.
    pub fn import_wav(&mut self, path: &Path) -> Result<String> {
        let audio = decode_to_project_rate(path)?;
        let name = path
            .file_stem()
            .map_or_else(|| "sample".into(), |s| s.to_string_lossy().into_owned());
        let info = self.new_source(&name, None);
        let id = info.id.clone();
        self.sources.insert(id.clone(), Arc::new(audio));
        self.pending
            .insert(id.clone(), PendingSource::Copy(path.to_path_buf()));
        self.song.sources.push(info);
        self.flush_sources()?;
        Ok(id)
    }

    /// Adds audio (a recording, a generated sample) as a new source.
    /// Returns the source id.
    pub fn add_audio(
        &mut self,
        name: &str,
        audio: AudioBuffer,
        recorded_bpm: Option<f64>,
    ) -> String {
        let info = self.new_source(name, recorded_bpm);
        let id = info.id.clone();
        let original = Arc::new(audio);
        self.sources
            .insert(id.clone(), Arc::new(to_project_rate(&original)));
        self.pending
            .insert(id.clone(), PendingSource::Write(original));
        self.song.sources.push(info);
        // Write it straight away when we know where; otherwise on save.
        if let Err(e) = self.flush_sources() {
            self.warnings.push(e.to_string());
        }
        id
    }

    /// Writes any sources not yet in `sources/`.
    fn flush_sources(&mut self) -> Result<()> {
        let Some(dir) = self.dir() else {
            return Ok(());
        };
        if self.pending.is_empty() {
            return Ok(());
        }
        let src_dir = dir.join("sources");
        std::fs::create_dir_all(&src_dir).map_err(|e| Error::io(&src_dir, e))?;
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        for id in ids {
            let Some(info) = self.song.source(&id).cloned() else {
                self.pending.remove(&id);
                continue;
            };
            let dest = dir.join(&info.file);
            match &self.pending[&id] {
                PendingSource::Copy(orig) => {
                    let bytes = std::fs::read(orig).map_err(|e| Error::io(orig, e))?;
                    write_atomic(&dest, &bytes)?;
                }
                PendingSource::Write(audio) => {
                    let tmp = dest.with_extension("wav.tmp");
                    wav::write_mono(&tmp, audio, wav::WavFormat::Float32)?;
                    std::fs::rename(&tmp, &dest).map_err(|e| Error::io(&dest, e))?;
                }
            }
            self.pending.remove(&id);
        }
        Ok(())
    }

    /// Saves to [`Project::path`].
    pub fn save(&mut self) -> Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(Error::Project(
                "this project has no file yet; save it with :w name.hummr".into(),
            ));
        };
        self.save_as(&path)
    }

    /// Saves to `path` (which becomes the project's path), writing pending
    /// sources first. Moving a project to a new folder copies its sources.
    pub fn save_as(&mut self, path: &Path) -> Result<()> {
        let old_dir = self.dir();
        let new_dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        if let Some(old) = old_dir.filter(|d| *d != new_dir) {
            for s in &self.song.sources {
                if !self.pending.contains_key(&s.id) && old.join(&s.file).exists() {
                    self.pending
                        .insert(s.id.clone(), PendingSource::Copy(old.join(&s.file)));
                }
            }
        }
        if !new_dir.as_os_str().is_empty() {
            std::fs::create_dir_all(&new_dir).map_err(|e| Error::io(&new_dir, e))?;
        }
        self.path = Some(path.to_path_buf());
        self.flush_sources()?;
        write_atomic(path, to_toml(&self.song)?.as_bytes())
    }

    /// Whether every source has been written to disk.
    pub fn sources_saved(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::{Effect, FxChain};
    use crate::project::{Clip, ClipKind, Note, Slice, Stretch, TimeSig, Voice};
    use crate::synthetic::sine;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hummr-proj-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rich_song() -> Song {
        let mut s = Song::new("Beatbox Song");
        s.bpm = 96.0;
        s.time_sig = TimeSig::THREE_FOUR;
        s.key = Some("A minor".parse().unwrap());
        s.sources.push(SourceInfo {
            id: "s1".into(),
            file: "sources/s1-beatbox.wav".into(),
            name: "beatbox".into(),
            recorded_bpm: Some(96.0),
        });
        let t = s.add_track("Beat");
        let mut a = Clip::audio(10, "beatbox", "s1", 0.25, 2.5, 0.0);
        a.gain_db = -3.5;
        a.repeat = 2;
        a.stretch = Stretch::Follow { bpm: 96.0 };
        a.fx = FxChain {
            preset: Some("Lo-fi".into()),
            amount: 0.6,
            reverb: 0.2,
            raw: vec![Effect::Echo {
                beats: 0.75,
                feedback: 0.4,
                mix: 0.3,
            }],
            ..FxChain::default()
        };
        s.tracks[t].clips.push(a);
        let mut b = Clip::audio(11, "chop", "s1", 0.0, 1.0, 4.0);
        b.kind = ClipKind::Sliced {
            length_beats: 2.0,
            slices: vec![Slice::new(0.0, 0.3, 0.0), Slice::new(0.3, 0.7, 0.5)],
            grid: 0.0,
            strength: 0.0,
        };
        s.tracks[t].clips.push(b);
        let t2 = s.add_track("Lead");
        s.tracks[t2].volume_db = -6.0;
        s.tracks[t2].pan = 0.25;
        s.tracks[t2].solo = true;
        let mut n = Clip::audio(12, "hum → piano", "s1", 0.0, 2.0, 8.0);
        n.kind = ClipKind::Notes {
            length_beats: 4.0,
            voice: Voice::Instrument { program: 0 },
            notes: vec![Note {
                pitch: 69,
                beat: 0.0,
                len: 1.5,
                vel: 90,
            }],
        };
        s.tracks[t2].clips.push(n);
        s
    }

    #[test]
    fn song_round_trips_through_toml_unchanged() {
        let song = rich_song();
        let text = to_toml(&song).unwrap();
        let back = from_toml(&text).unwrap();
        assert_eq!(back, song);
        // Byte-stable: saving what was loaded gives the same file.
        assert_eq!(to_toml(&back).unwrap(), text);
        assert!(text.contains("format = 1"), "{text}");
        assert!(text.contains("key = \"A minor\""), "{text}");
        assert!(text.contains("time_signature = \"3/4\""), "{text}");
    }

    #[test]
    fn project_saves_sources_and_reopens() {
        let dir = tmp("save");
        let mut p = Project::create_in(&dir, "My Song").unwrap();
        let path = p.path.clone().unwrap();
        assert!(path.ends_with("my-song/my-song.hummr"));

        // Import a 44.1 kHz WAV: copied verbatim, decoded at project rate.
        let orig = dir.join("hit.wav");
        let a = AudioBuffer::new(sine(440.0, 44_100, 0.5, 0.5), 44_100);
        wav::write_mono(&orig, &a, wav::WavFormat::Pcm16).unwrap();
        let id = p.import_wav(&orig).unwrap();
        assert_eq!(p.sources.get(&id).unwrap().sample_rate, PROJECT_RATE);
        assert_eq!(p.sources.get(&id).unwrap().samples.len(), 24_000);
        let rec = p.add_audio(
            "my take",
            AudioBuffer::new(vec![0.1; 4_800], 48_000),
            Some(90.0),
        );
        let t = p.song.add_track("Beat");
        let cid = p.song.alloc_id();
        p.song.tracks[t]
            .clips
            .push(Clip::audio(cid, "hit", &id, 0.0, 0.5, 0.0));
        p.save().unwrap();
        assert!(p.sources_saved());

        let copied = path
            .parent()
            .unwrap()
            .join(&p.song.source(&id).unwrap().file);
        assert_eq!(
            std::fs::read(&copied).unwrap(),
            std::fs::read(&orig).unwrap()
        );

        let q = Project::open(&path).unwrap();
        assert!(q.warnings.is_empty(), "{:?}", q.warnings);
        assert_eq!(q.song, p.song);
        assert_eq!(q.sources.get(&rec).unwrap().samples.len(), 4_800);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            to_toml(&q.song).unwrap()
        );
        assert!(Project::create_in(&dir, "My Song").is_err(), "no overwrite");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_sources_open_silently_with_a_warning() {
        let dir = tmp("missing");
        let mut p = Project::create_in(&dir, "x").unwrap();
        p.add_audio("gone", AudioBuffer::new(vec![0.0; 10], 48_000), None);
        p.save().unwrap();
        let file = p.dir().unwrap().join(&p.song.sources[0].file);
        std::fs::remove_file(file).unwrap();
        let q = Project::open(p.path.as_ref().unwrap()).unwrap();
        assert_eq!(q.warnings.len(), 1);
        assert!(q.warnings[0].contains("gone"), "{:?}", q.warnings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn friendly_errors() {
        let e = from_toml("this is not toml = = =").unwrap_err().to_string();
        assert!(e.contains("not a valid Hummr project"), "{e}");
        let newer = to_toml(&Song::new("x"))
            .unwrap()
            .replace("format = 1", "format = 9");
        let e = from_toml(&newer).unwrap_err().to_string();
        assert!(e.contains("newer Hummr"), "{e}");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Song!"), "my-song");
        assert_eq!(slug("hum → piano"), "hum-piano");
        assert_eq!(slug("!!!"), "sound");
    }
}
