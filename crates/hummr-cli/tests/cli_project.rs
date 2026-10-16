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
