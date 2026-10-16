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
