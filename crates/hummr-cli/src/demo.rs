//! `hummr demo`: writes example WAVs into ./demo/ so you can hear what
//! each feature does. Everything is made from built-in synthetic sounds
//! (a hummed melody, a beatboxed loop), so the results are the same on
//! every machine.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hummr_core::AudioBuffer;
use hummr_core::fx::Preset;
use hummr_core::kit::{Sound, drum_hit};
use hummr_core::project::convert::{Target, convert};
use hummr_core::project::ops::{MatchMode, analyze, match_tempo};
use hummr_core::project::{Clip, PROJECT_RATE, Project, export};
use hummr_core::synthetic::{HumParams, TWINKLE, Tone, drum_groove, hum, melody, white_noise};

const SR: u32 = PROJECT_RATE;
const BPM: f64 = 96.0;

#[derive(clap::Args)]
pub struct Args {
    /// Folder to write into.
    #[arg(long, default_value = "demo")]
    out: PathBuf,
}

/// A song with the same tempo as the demos and nothing in it yet.
fn song(name: &str) -> Project {
    let mut p = Project::new(name);
    p.song.bpm = BPM;
    p
}

fn seconds(beats: f64) -> f64 {
    beats * 60.0 / BPM
}

/// Adds `audio` as a source and a clip of all of it at `beat` on `track`.
fn place(p: &mut Project, track: usize, name: &str, audio: &[f32], beat: f64) -> u64 {
    let id = p.add_audio(name, AudioBuffer::new(audio.to_vec(), SR), None);
    let len = audio.len() as f64 / f64::from(SR);
    p.song
        .insert_clip(track, Clip::audio(0, name, &id, 0.0, len, beat))
        .expect("track exists")
}

/// A quiet closed hat on every eighth note: the grid you hear against.
fn add_hats(p: &mut Project, beats: usize) {
    let t = p.song.add_track("Grid");
    p.song.tracks[t].volume_db = -14.0;
    let hat = drum_hit(Sound::HatClosed, 0.6, SR, 1);
    for i in 0..beats * 2 {
        place(p, t, "hat", &hat, i as f64 * 0.5);
    }
}

fn hummed_twinkle(seed: u64) -> Vec<f32> {
    hum(
        &melody(&TWINKLE, BPM, 0.0, 0.0, seed),
        &HumParams::new(SR),
        seed,
    )
}

fn hummed_bass() -> Vec<f32> {
    // A, C, G, E: two beats each, an octave and a half below the melody.
    let tones: Vec<Tone> = [45.0, 48.0, 43.0, 40.0, 45.0, 48.0, 43.0, 43.0]
        .iter()
        .enumerate()
        .map(|(i, &m)| Tone::new(m, seconds(2.0 * i as f64), seconds(1.9)))
        .collect();
    hum(&tones, &HumParams::new(SR), 21)
}

/// A beatboxed two-bar groove at the song tempo, with hits up to
/// `jitter` seconds early or late.
fn beatbox(bars: usize, jitter: f64, seed: u64) -> Vec<f32> {
    drum_groove(BPM, bars, 0, 0.0, jitter, seed, SR).0
}

/// Replaces clip `id` with what `f` makes of it.
fn change(p: &mut Project, id: u64, f: impl FnOnce(&Project, &Clip, &AudioBuffer) -> Clip) {
    let clip = p.song.clip(id).cloned().expect("clip");
    let audio = p.sources.get(&clip.source).cloned().expect("source");
    let new = f(p, &clip, &audio);
    *p.song.clip_mut(id).expect("clip") = new;
}

fn turn(p: &mut Project, id: u64, target: Target) -> Result<String> {
    let clip = p.song.clip(id).cloned().expect("clip");
    let audio = p.sources.get(&clip.source).cloned().expect("source");
    let c = convert(&p.song, &clip, &audio, target)?;
    *p.song.clip_mut(id).expect("clip") = c.clip;
    if p.song.key.is_none() {
        p.song.key = c.key;
    }
    Ok(c.summary)
}

fn cut_and_snap(p: &mut Project, id: u64) {
    let bpm = p.song.bpm;
    change(p, id, |_, clip, audio| {
        let est = analyze(clip, audio);
        match_tempo(
            clip,
            audio,
            &est,
            bpm,
            MatchMode::CutAndSnap {
                grid: 0.5,
                strength: 1.0,
            },
        )
    });
}

struct Writer {
    dir: PathBuf,
    written: Vec<(String, String)>,
}

impl Writer {
    fn write(&mut self, file: &str, what: &str, p: &Project) -> Result<()> {
        let path = self.dir.join(file);
        export::export(&p.song, &p.sources, &path)
            .with_context(|| format!("writing {}", path.display()))?;
        self.written.push((file.to_string(), what.to_string()));
        Ok(())
    }
}

pub fn run(args: Args) -> Result<()> {
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("could not create {}", args.out.display()))?;
    let mut w = Writer {
        dir: args.out.clone(),
        written: Vec::new(),
    };
    write_all(&mut w)?;
    let about: String = w
        .written
        .iter()
        .map(|(f, what)| format!("{f:<34} {what}\n"))
        .collect();
    std::fs::write(args.out.join("ABOUT.txt"), &about)?;
    println!(
        "wrote {} demo files into {}:",
        w.written.len(),
        shown(&args.out)
    );
    print!("{about}");
    println!(
        "Listen first: 00-song.wav, then 02-beatbox-snapped.wav against 01-beatbox-sloppy.wav."
    );
    Ok(())
}

fn shown(p: &Path) -> String {
    format!("{}/", p.display())
}

fn write_all(w: &mut Writer) -> Result<()> {
    // A small song: everything below in one place.
    let mut p = song("Demo song");
    let beat = p.song.add_track("Beat");
    let id = place(&mut p, beat, "beatbox", &beatbox(4, 0.025, 3), 0.0);
    cut_and_snap(&mut p, id);
    let bass = p.song.add_track("Bass");
    let id = place(&mut p, bass, "hummed bass", &hummed_bass(), 0.0);
    turn(&mut p, id, Target::Notes)?;
    if let Some(c) = p.song.clip_mut(id) {
        if let hummr_core::project::ClipKind::Notes { voice, .. } = &mut c.kind {
            *voice = hummr_core::project::Voice::Instrument { program: 33 };
        }
    }
    let lead = p.song.add_track("Lead");
    p.song.tracks[lead].volume_db = -2.0;
    let id = place(&mut p, lead, "hummed melody", &hummed_twinkle(3), 0.0);
    if let Some(c) = p.song.clip_mut(id) {
        c.fx.preset = Some(Preset::Dreamy.name().into());
        c.fx.amount = 0.5;
    }
    w.write(
        "00-song.wav",
        "a beatboxed beat (snapped), a hummed bass turned into a bass guitar, the hummed melody with the Dreamy preset",
        &p,
    )?;

    // Snapping: a sloppy beatbox loop before and after, over an on-grid hat.
    let sloppy = beatbox(2, 0.035, 7);
    let mut p = song("Snapping");
    let t = p.song.add_track("Beat");
    let id = place(&mut p, t, "beatbox", &sloppy, 0.0);
    add_hats(&mut p, 8);
    w.write(
        "01-beatbox-sloppy.wav",
        "a beatbox loop with hits up to 35 ms off, against a steady hat",
        &p,
    )?;
    cut_and_snap(&mut p, id);
    w.write(
        "02-beatbox-snapped.wav",
        "the same loop cut at each hit and snapped to 1/8 notes (no stretching)",
        &p,
    )?;

    // Tempo matching: a 120 BPM loop in a 96 BPM song.
    let fast = drum_groove(120.0, 2, 1, 0.0, 0.0, 11, SR).0;
    let reference = beatbox(4, 0.0, 12);
    for (file, what, mode) in [
        (
            "03-tempo-left-as-is.wav",
            "a 120 BPM loop dropped into a 96 BPM song as it is: it drifts against the beat",
            MatchMode::Keep,
        ),
        (
            "04-tempo-cut-and-snap.wav",
            "the 120 BPM loop cut at its hits and placed on the 96 BPM beat (sound unchanged)",
            MatchMode::CutAndSnap {
                grid: 0.5,
                strength: 1.0,
            },
        ),
        (
            "05-tempo-stretched.wav",
            "the 120 BPM loop time-stretched to 96 BPM (same pitch)",
            MatchMode::Stretch,
        ),
    ] {
        let mut p = song("Tempo match");
        let t = p.song.add_track("Song beat");
        p.song.tracks[t].volume_db = -6.0;
        place(&mut p, t, "song beat", &reference, 0.0);
        let t = p.song.add_track("Loop");
        let id = place(&mut p, t, "120 BPM loop", &fast, 0.0);
        change(&mut p, id, |p, clip, audio| {
            let est = analyze(clip, audio);
            match_tempo(clip, audio, &est, p.song.bpm, mode)
        });
        if let Some(c) = p.song.clip_mut(id) {
            c.repeat = 2;
        }
        w.write(file, what, &p)?;
    }

    // Effect presets on the hummed melody.
    let twinkle = hummed_twinkle(5);
    let mut p = song("Dry");
    let t = p.song.add_track("Voice");
    place(&mut p, t, "hum", &twinkle, 0.0);
    w.write(
        "10-preset-none.wav",
        "the hummed melody with no effects",
        &p,
    )?;
    for (i, preset) in Preset::ALL.iter().enumerate() {
        let mut p = song(preset.name());
        let t = p.song.add_track("Voice");
        let id = place(&mut p, t, "hum", &twinkle, 0.0);
        if let Some(c) = p.song.clip_mut(id) {
            c.fx.preset = Some(preset.name().into());
            c.fx.amount = 0.8;
        }
        let slug = preset.name().to_ascii_lowercase().replace([' ', '-'], "");
        w.write(
            &format!("{}-preset-{slug}.wav", 11 + i),
            &format!("the hummed melody with the {} preset at 80%", preset.name()),
            &p,
        )?;
    }

    // Noise cleanup: hiss under the hum, before and after. The first 300
    // ms are hiss only, which is where cleanup learns the noise.
    let lead_in = (0.3 * f64::from(SR)) as usize;
    let mut noisy = vec![0.0f32; lead_in];
    noisy.extend_from_slice(&twinkle);
    let hiss = white_noise(noisy.len(), 0.03, 99);
    for (s, n) in noisy.iter_mut().zip(&hiss) {
        *s += n;
    }
    let mut p = song("Noisy");
    let t = p.song.add_track("Voice");
    let id = place(&mut p, t, "noisy hum", &noisy, 0.0);
    w.write(
        "20-clean-before.wav",
        "the hum with steady hiss (about -30 dBFS)",
        &p,
    )?;
    if let Some(c) = p.song.clip_mut(id) {
        c.clean = 0.8;
    }
    w.write(
        "21-clean-after.wav",
        "the same after Clean noise at 80%: the hiss is turned down, the voice kept",
        &p,
    )?;

    // The three conversions.
    let mut p = song("Notes");
    let t = p.song.add_track("Voice");
    let id = place(&mut p, t, "hum", &twinkle, 0.0);
    let summary = turn(&mut p, id, Target::Notes)?;
    if let Some(c) = p.song.clip_mut(id) {
        // The preview piano is quiet next to the voice.
        c.gain_db = 10.0;
    }
    w.write(
        "30-turn-into-notes.wav",
        &format!("the hum turned into notes on a piano ({summary})"),
        &p,
    )?;
    let mut p = song("Own sound");
    let t = p.song.add_track("Voice");
    let id = place(&mut p, t, "hum", &twinkle, 0.0);
    turn(&mut p, id, Target::OwnSound)?;
    if let Some(c) = p.song.clip_mut(id) {
        c.pitch = 12.0;
        c.gain_db = 6.0;
    }
    w.write(
        "31-turn-into-own-sound.wav",
        "the hum as its own instrument, played an octave up",
        &p,
    )?;
    let mut p = song("Drums");
    let t = p.song.add_track("Beat");
    let id = place(&mut p, t, "beatbox", &beatbox(2, 0.0, 8), 0.0);
    w.write(
        "32-beatbox-original.wav",
        "a beatboxed loop as recorded",
        &p,
    )?;
    turn(&mut p, id, Target::Drums)?;
    w.write(
        "33-turn-into-drums.wav",
        "the beatbox loop with each hit replaced by a kick, snare or hat",
        &p,
    )?;
    Ok(())
}
