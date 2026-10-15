//! Runs the real `hummr` binary on synthetic takes.

use std::path::PathBuf;
use std::process::Command;

use hummr_core::AudioBuffer;
use hummr_core::io::{midi, wav};
use hummr_core::synthetic::{self, HumParams, TWINKLE};

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hummr-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hummr(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hummr"))
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
fn transcribe_writes_midi_and_explains() {
    let dir = tmp_dir("ok");
    let take = dir.join("take.wav");
    let tones = synthetic::melody(&TWINKLE, 100.0, 0.3, 0.02, 1);
    let params = HumParams {
        detune_cents: -25.0,
        ..HumParams::new(44_100)
    };
    wav::write_mono(
        &take,
        &AudioBuffer::new(synthetic::hum(&tones, &params, 1), 44_100),
        wav::WavFormat::Pcm16,
    )
    .unwrap();
    let out = dir.join("lead.mid");

    let (ok, stdout, stderr) = hummr(&[
        "transcribe",
        take.to_str().unwrap(),
        "--key",
        "auto",
        "--quantize",
        "1/8",
        "-o",
        out.to_str().unwrap(),
        "--explain",
    ]);
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("C major"), "{stdout}");
    assert!(stdout.contains("100 BPM"), "{stdout}");
    assert!(stdout.contains("why notes changed"), "{stdout}");
    assert!(stdout.contains("C4 1/4 C4 1/4 G4 1/4"), "{stdout}");

    let seq = midi::read_file(&out).unwrap();
    let pitches: Vec<u8> = seq.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, TWINKLE.iter().map(|n| n.0).collect::<Vec<_>>());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn friendly_errors() {
    let (ok, _, stderr) = hummr(&["transcribe", "/no/such/take.wav"]);
    assert!(!ok);
    assert!(stderr.contains("not found"), "{stderr}");

    let dir = tmp_dir("err");
    let silent = dir.join("silent.wav");
    wav::write_mono(
        &silent,
        &AudioBuffer::new(vec![0.0; 44_100], 44_100),
        wav::WavFormat::Pcm16,
    )
    .unwrap();
    let (ok, _, stderr) = hummr(&["transcribe", silent.to_str().unwrap()]);
    assert!(!ok);
    assert!(
        stderr.contains("silent") && stderr.contains("microphone permission"),
        "{stderr}"
    );

    let (ok, _, stderr) = hummr(&["transcribe", silent.to_str().unwrap(), "--key", "H major"]);
    assert!(!ok);
    assert!(stderr.contains("invalid key"), "{stderr}");

    let (ok, _, stderr) = hummr(&["transcribe", silent.to_str().unwrap(), "--quantize", "1/7"]);
    assert!(!ok);
    assert!(stderr.contains("1/8"), "{stderr}");
    std::fs::remove_dir_all(dir).ok();
}
