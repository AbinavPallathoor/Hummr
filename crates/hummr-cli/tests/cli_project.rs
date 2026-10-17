//! `hummr new` and `hummr export` on a real project folder.

use std::path::PathBuf;
use std::process::Command;

use hummr_core::AudioBuffer;
use hummr_core::io::wav;
use hummr_core::project::{Clip, Project};
use hummr_core::synthetic::sine;

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hummr-cli-proj-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hummr(dir: &PathBuf, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hummr"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn new_then_export() {
    let dir = tmp_dir("new");
    let (ok, out, err) = hummr(&dir, &["new", "Beat Song"]);
    assert!(ok, "{err}");
    assert!(out.contains("beat-song/beat-song.hummr"), "{out}");
    let path = dir.join("beat-song/beat-song.hummr");
    assert!(path.exists());

    let (ok, _, err) = hummr(&dir, &["new", "Beat Song"]);
    assert!(!ok && err.contains("already exists"), "{err}");

    // Exporting an empty song explains what to do.
    let (ok, _, err) = hummr(
        &dir,
        &["export", "beat-song/beat-song.hummr", "-o", "x.wav"],
    );
    assert!(!ok && err.contains("record or import"), "{err}");

    let mut p = Project::open(&path).unwrap();
    let id = p.add_audio(
        "tone",
        AudioBuffer::new(sine(220.0, 48_000, 1.0, 0.5), 48_000),
        None,
    );
    let t = p.song.add_track("Tone");
    p.song
        .insert_clip(t, Clip::audio(0, "tone", &id, 0.0, 1.0, 0.0));
    p.save().unwrap();

    let (ok, out, err) = hummr(
        &dir,
        &["export", "beat-song/beat-song.hummr", "-o", "mix.wav"],
    );
    assert!(ok, "{err}");
    assert!(out.contains("mix.wav"), "{out}");
    let mix = wav::read_mono(&dir.join("mix.wav")).unwrap();
    assert_eq!(mix.sample_rate, 48_000);
    assert_eq!(mix.samples.len(), 48_000);
    assert!(mix.peak() > 0.3);

    let (ok, _, err) = hummr(
        &dir,
        &["export", "beat-song/beat-song.hummr", "-o", "n.mid"],
    );
    assert!(!ok && err.contains("no notes"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn detect_reports_tempo_in_plain_words() {
    let dir = tmp_dir("detect");
    let (audio, _) = hummr_core::synthetic::drum_loop(96.0, 4, 0, 0.2, 0.01, 5, 48_000);
    wav::write_mono(
        &dir.join("loop.wav"),
        &AudioBuffer::new(audio, 48_000),
        wav::WavFormat::Pcm16,
    )
    .unwrap();
    let (ok, out, err) = hummr(&dir, &["detect", "loop.wav", "--hits"]);
    assert!(ok, "{err}");
    assert!(out.contains("96 BPM, sure"), "{out}");
    assert!(out.contains("hits found"), "{out}");

    wav::write_mono(
        &dir.join("noise.wav"),
        &AudioBuffer::new(hummr_core::synthetic::white_noise(96_000, 0.2, 1), 48_000),
        wav::WavFormat::Pcm16,
    )
    .unwrap();
    let (ok, out, _) = hummr(&dir, &["detect", "noise.wav"]);
    assert!(ok);
    assert!(out.contains("no steady beat"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn snap_moves_hits_onto_the_grid() {
    let dir = tmp_dir("snap");
    let (audio, _) = hummr_core::synthetic::drum_groove(96.0, 2, 0, 0.3, 0.015, 3, 48_000);
    wav::write_mono(
        &dir.join("take.wav"),
        &AudioBuffer::new(audio.clone(), 48_000),
        wav::WavFormat::Float32,
    )
    .unwrap();
    let (ok, out, err) = hummr(
        &dir,
        &[
            "snap",
            "take.wav",
            "--bpm",
            "96",
            "--strength",
            "1",
            "--grid",
            "1/8",
        ],
    );
    assert!(ok, "{err}");
    assert!(out.contains("take-snapped.wav"), "{out}");
    let snapped = wav::read_mono(&dir.join("take-snapped.wav")).unwrap();
    assert_eq!(snapped.samples.len(), audio.len());
    let phase = hummr_core::beat::phase_for_tempo(&audio, 48_000, 96.0);
    let step = 60.0 / 96.0 / 2.0;
    for o in hummr_core::onset::detect_onsets(&snapped.samples, 48_000) {
        let d = ((o.time - phase) / step).rem_euclid(1.0);
        assert!(d.min(1.0 - d) * step < 0.005, "hit at {}", o.time);
    }
    // Strength 0 writes the take back unchanged.
    let (ok, _, err) = hummr(
        &dir,
        &[
            "snap",
            "take.wav",
            "--bpm",
            "96",
            "--strength",
            "0",
            "-o",
            "same.wav",
        ],
    );
    assert!(ok, "{err}");
    let same = wav::read_mono(&dir.join("same.wav")).unwrap();
    let worst = same
        .samples
        .iter()
        .zip(&audio)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-3, "strength 0 changed the take by {worst}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clean_turns_down_hiss() {
    let dir = tmp_dir("clean");
    let sr = 48_000u32;
    let mut state = 11u64;
    let audio: Vec<f32> = (0..2 * sr as usize)
        .map(|i| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let n = ((state >> 33) as f32 / (1u64 << 31) as f32 * 2.0 - 1.0) * 0.02;
            let t = if i >= sr as usize {
                0.3 * (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin()
            } else {
                0.0
            };
            n + t
        })
        .collect();
    wav::write_mono(
        &dir.join("take.wav"),
        &AudioBuffer::new(audio.clone(), sr),
        wav::WavFormat::Float32,
    )
    .unwrap();
    let (ok, out, err) = hummr(&dir, &["clean", "take.wav", "--noise", "0.1:0.5"]);
    assert!(ok, "{err}");
    assert!(out.contains("take-clean.wav"), "{out}");
    let cleaned = wav::read_mono(&dir.join("take-clean.wav")).unwrap();
    assert_eq!(cleaned.samples.len(), audio.len());
    let db = |x: &[f32]| hummr_core::dsp::amp_to_db(hummr_core::dsp::rms(x));
    let quiet = 4_800..43_000;
    assert!(db(&audio[quiet.clone()]) - db(&cleaned.samples[quiet]) >= 12.0);
    let (ok, _, err) = hummr(&dir, &["clean", "take.wav", "--noise", "nope"]);
    assert!(!ok);
    assert!(err.contains("start and length"), "{err}");
}

#[test]
fn a_broken_soundfont_falls_back_with_a_message() {
    let dir = tmp_dir("sf");
    let (ok, _, err) = hummr(&dir, &["new", "Sf"]);
    assert!(ok, "{err}");
    let path = dir.join("sf/sf.hummr");
    let mut p = Project::open(&path).unwrap();
    let t = p.song.add_track("Keys");
    let mut c = Clip::audio(0, "keys", "", 0.0, 2.0, 0.0);
    c.kind = hummr_core::project::ClipKind::Notes {
        length_beats: 4.0,
        voice: hummr_core::project::Voice::Instrument { program: 0 },
        notes: vec![hummr_core::project::Note {
            pitch: 60,
            beat: 0.0,
            len: 1.0,
            vel: 100,
        }],
    };
    p.song.insert_clip(t, c);
    p.save().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hummr"))
        .current_dir(&dir)
        .env("HUMMR_SOUNDFONT", dir.join("missing.sf2"))
        .args(["export", "sf/sf.hummr", "-o", "out.wav"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("built-in synth"), "{err}");
    assert!(dir.join("out.wav").exists());
}

#[test]
fn help_says_hummr_alone_opens_or_starts_a_song() {
    let dir = tmp_dir("help");
    let (ok, out, _) = hummr(&dir, &["--help"]);
    assert!(ok);
    assert!(out.contains("open or start a song here"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
