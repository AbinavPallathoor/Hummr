//! General MIDI instruments from a SoundFont (`.sf2`) file, played with
//! rustysynth, with the built-in preview synth as the fallback.
//!
//! Hummr ships without a SoundFont (see docs/DECISIONS.md). Point it at one
//! with the `HUMMR_SOUNDFONT` environment variable or a `soundfont = "…"`
//! line in `~/.config/hummr/config.toml`.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::project::Note;
use crate::voice::{InstrumentVoice, PreviewSynth};
use crate::{Error, Result};

/// A loaded SoundFont.
#[derive(Clone)]
pub struct Loaded {
    pub path: PathBuf,
    /// The bank name stored in the file.
    pub name: String,
    pub font: Arc<SoundFont>,
}

static ACTIVE: RwLock<Option<Loaded>> = RwLock::new(None);

/// Reads a SoundFont file.
pub fn load_file(path: &Path) -> Result<Loaded> {
    let file = File::open(path).map_err(|e| {
        Error::Project(format!(
            "Couldn't open the SoundFont {}: {e}. Check the path in HUMMR_SOUNDFONT or your config file.",
            path.display()
        ))
    })?;
    let font = SoundFont::new(&mut BufReader::new(file)).map_err(|e| {
        Error::Project(format!(
            "{} isn't a SoundFont Hummr can read ({e:?}). Use a .sf2 file (SoundFont 2; .sf3 is not supported).",
            path.display()
        ))
    })?;
    let name = font.get_info().get_bank_name().trim().to_string();
    let name = if name.is_empty() {
        path.file_stem()
            .map_or_else(|| "SoundFont".into(), |s| s.to_string_lossy().into_owned())
    } else {
        name
    };
    Ok(Loaded {
        path: path.to_path_buf(),
        name,
        font: Arc::new(font),
    })
}

/// Makes `loaded` (or the built-in synth, for `None`) the voice for every
/// instrument from now on.
pub fn set_active(loaded: Option<Loaded>) {
    if let Ok(mut a) = ACTIVE.write() {
        *a = loaded;
    }
}

/// The SoundFont in use, if any.
pub fn active() -> Option<Loaded> {
    ACTIVE.read().ok().and_then(|a| a.clone())
}

/// Where the user asked for a SoundFont: `HUMMR_SOUNDFONT`, else
/// `soundfont = "…"` in the config file.
pub fn configured_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HUMMR_SOUNDFONT").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let text = std::fs::read_to_string(config_file()?).ok()?;
    let table: toml::Table = text.parse().ok()?;
    table
        .get("soundfont")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

/// `$XDG_CONFIG_HOME/hummr/config.toml`, else `~/.config/hummr/config.toml`.
pub fn config_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hummr").join("config.toml"))
}

/// Loads the configured SoundFont (if any) and returns one plain sentence
/// for the status line.
pub fn init() -> String {
    match configured_path() {
        None => {
            set_active(None);
            "Instruments use the built-in synth. For realistic instruments, set HUMMR_SOUNDFONT to a .sf2 file."
                .into()
        }
        Some(path) => match load_file(&path) {
            Ok(l) => {
                let msg = format!("Instruments use the SoundFont \"{}\".", l.name);
                set_active(Some(l));
                msg
            }
            Err(e) => {
                set_active(None);
                format!("{e} Using the built-in synth instead.")
            }
        },
    }
}

/// The voice that plays General MIDI `program`.
pub fn instrument(program: u8) -> Box<dyn InstrumentVoice> {
    choose(active(), program)
}

/// The SoundFont voice if one is loaded, else the preview synth.
pub fn choose(loaded: Option<Loaded>, program: u8) -> Box<dyn InstrumentVoice> {
    match loaded {
        Some(l) => Box::new(SoundFontVoice { loaded: l, program }),
        None => Box::new(PreviewSynth { program }),
    }
}

/// Renders the synth until both channels are `to` samples long.
fn render_to(synth: &mut Synthesizer, l: &mut Vec<f32>, r: &mut Vec<f32>, to: usize) {
    let from = l.len();
    if to > from {
        l.resize(to, 0.0);
        r.resize(to, 0.0);
        synth.render(&mut l[from..to], &mut r[from..to]);
    }
}

/// One General MIDI program of a SoundFont.
pub struct SoundFontVoice {
    pub loaded: Loaded,
    pub program: u8,
}

/// The longest a note may ring after it ends (seconds).
const MAX_TAIL: f64 = 4.0;

impl InstrumentVoice for SoundFontVoice {
    fn describe(&self) -> String {
        format!(
            "{} ({})",
            crate::instruments::name(self.program),
            self.loaded.name
        )
    }

    fn render(&self, notes: &[Note], bpm: f64, sample_rate: u32) -> Vec<f32> {
        let mut settings = SynthesizerSettings::new(sample_rate as i32);
        // Hummr's own effects do reverb; keep the synth dry and exact.
        settings.enable_reverb_and_chorus = false;
        let Ok(mut synth) = Synthesizer::new(&self.loaded.font, &settings) else {
            return PreviewSynth {
                program: self.program,
            }
            .render(notes, bpm, sample_rate);
        };
        synth.process_midi_message(0, 0xC0, i32::from(self.program), 0);
        let spb = 60.0 / bpm;
        let sr = f64::from(sample_rate);
        // (sample, is_on, key, velocity); note-offs sort before note-ons.
        let mut events: Vec<(usize, bool, i32, i32)> = Vec::with_capacity(notes.len() * 2);
        for n in notes {
            let on = (n.beat.max(0.0) * spb * sr).round() as usize;
            let off = ((n.beat + n.len).max(0.0) * spb * sr).round() as usize;
            events.push((on, true, i32::from(n.pitch), i32::from(n.vel.max(1))));
            events.push((off.max(on + 1), false, i32::from(n.pitch), 0));
        }
        events.sort_by_key(|e| (e.0, e.1));
        let end = events.last().map_or(0, |e| e.0);
        let mut left = Vec::with_capacity(end + sample_rate as usize);
        let mut right = Vec::with_capacity(end + sample_rate as usize);
        for (at, on, key, vel) in events {
            render_to(&mut synth, &mut left, &mut right, at);
            if on {
                synth.note_on(0, key, vel);
            } else {
                synth.note_off(0, key);
            }
        }
        // Let the release ring out, stopping once it is silent.
        let block = (sample_rate / 20) as usize;
        let limit = end + (MAX_TAIL * sr) as usize;
        while left.len() < limit {
            let from = left.len();
            render_to(&mut synth, &mut left, &mut right, from + block);
            let peak = left[from..]
                .iter()
                .chain(&right[from..])
                .fold(0.0f32, |m, s| m.max(s.abs()));
            if peak < 1e-5 {
                break;
            }
        }
        left.iter()
            .zip(&right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::pitch::yin::yin_pitch;

    const SR: u32 = 48_000;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend(p);
        }
        chunk(b"LIST", &body)
    }

    fn name20(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.resize(20, 0);
        v
    }

    fn u16s(xs: &[u16]) -> Vec<u8> {
        xs.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A minimal valid SoundFont: one preset (bank 0, program 0, "Test
    /// Sine") playing a looped 440 Hz sine recorded at 44 kHz, root A4.
    pub(crate) fn tiny_sf2() -> Vec<u8> {
        let rate = 44_000u32;
        let len = 2_000u32;
        let mut wave: Vec<i16> = (0..len)
            .map(|i| (12_000.0 * (std::f32::consts::TAU * i as f32 / 100.0).sin()) as i16)
            .collect();
        wave.extend([0i16; 46]);
        let smpl: Vec<u8> = wave.iter().flat_map(|s| s.to_le_bytes()).collect();

        let info = list(
            b"INFO",
            &[
                chunk(b"ifil", &u16s(&[2, 1])),
                chunk(b"isng", b"EMU8000\0"),
                chunk(b"INAM", b"Tiny Test\0"),
            ],
        );
        let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);

        let mut phdr = name20("Test Sine");
        phdr.extend(u16s(&[0, 0, 0]));
        phdr.extend([0u8; 12]);
        phdr.extend(name20("EOP"));
        phdr.extend(u16s(&[0, 0, 1]));
        phdr.extend([0u8; 12]);
        let pbag = u16s(&[0, 0, 1, 0]);
        let pmod = vec![0u8; 10];
        // instrument = 0, then the terminator.
        let pgen = u16s(&[41, 0, 0, 0]);
        let mut inst = name20("Sine");
        inst.extend(u16s(&[0]));
        inst.extend(name20("EOI"));
        inst.extend(u16s(&[1]));
        let ibag = u16s(&[0, 0, 2, 0]);
        let imod = vec![0u8; 10];
        // sampleModes = 1 (loop), sampleID = 0 (last), terminator.
        let igen = u16s(&[54, 1, 53, 0, 0, 0]);
        let mut shdr = name20("sine");
        for x in [0u32, len, 100, 1_900, rate] {
            shdr.extend(x.to_le_bytes());
        }
        shdr.extend([69u8, 0]);
        shdr.extend(u16s(&[0, 1]));
        shdr.extend(name20("EOS"));
        shdr.extend([0u8; 26]);
        let pdta = list(
            b"pdta",
            &[
                chunk(b"phdr", &phdr),
                chunk(b"pbag", &pbag),
                chunk(b"pmod", &pmod),
                chunk(b"pgen", &pgen),
                chunk(b"inst", &inst),
                chunk(b"ibag", &ibag),
                chunk(b"imod", &imod),
                chunk(b"igen", &igen),
                chunk(b"shdr", &shdr),
            ],
        );
        let mut body = b"sfbk".to_vec();
        body.extend(info);
        body.extend(sdta);
        body.extend(pdta);
        chunk(b"RIFF", &body)
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hummr-sf-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn note(pitch: u8, beat: f64, len: f64) -> Note {
        Note {
            pitch,
            beat,
            len,
            vel: 110,
        }
    }

    fn hz(x: &[f32]) -> f32 {
        yin_pitch(&x[4_800..4_800 + 4_096], SR, 50.0, 2_000.0, 0.15).unwrap()
    }

    #[test]
    fn plays_notes_at_the_right_pitch_and_time() {
        let path = tmp("ok").join("tiny.sf2");
        std::fs::write(&path, tiny_sf2()).unwrap();
        let loaded = load_file(&path).unwrap();
        assert_eq!(loaded.name, "Tiny Test");
        let v = choose(Some(loaded), 0);
        assert!(v.describe().contains("Tiny Test"), "{}", v.describe());
        // One beat at 120 BPM = 0.5 s.
        let a4 = v.render(&[note(69, 0.0, 1.0)], 120.0, SR);
        assert!((hz(&a4) / 440.0 - 1.0).abs() < 0.003, "{}", hz(&a4));
        let e5 = v.render(&[note(76, 0.0, 1.0)], 120.0, SR);
        let want = 440.0 * 2f32.powf(7.0 / 12.0);
        assert!((hz(&e5) / want - 1.0).abs() < 0.003, "{}", hz(&e5));
        // Sounds while held, stops after the release.
        let level = |x: &[f32]| crate::dsp::rms(x);
        assert!(level(&a4[4_800..19_200]) > 0.01);
        assert!(a4.len() < 24_000 + (MAX_TAIL * 48_000.0) as usize);
        assert!(level(&a4[a4.len() - 2_400..]) < 1e-4);
        // A late note starts late.
        let late = v.render(&[note(69, 2.0, 0.5)], 120.0, SR);
        assert!(level(&late[..47_000]) < 1e-6);
        assert!(level(&late[48_500..55_000]) > 0.01);
        // Deterministic.
        assert_eq!(late, v.render(&[note(69, 2.0, 0.5)], 120.0, SR));
    }

    #[test]
    fn missing_or_broken_files_explain_what_to_do() {
        let e = load_file(Path::new("/nonexistent/x.sf2"))
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains("Couldn't open") && e.contains("HUMMR_SOUNDFONT"),
            "{e}"
        );
        let path = tmp("bad").join("bad.sf2");
        std::fs::write(&path, b"not a soundfont").unwrap();
        let e = load_file(&path).err().unwrap().to_string();
        assert!(e.contains("isn't a SoundFont"), "{e}");
    }

    #[test]
    fn without_a_soundfont_the_preview_synth_plays() {
        let v = choose(None, 40);
        assert!(v.describe().contains("built-in"), "{}", v.describe());
    }
}
