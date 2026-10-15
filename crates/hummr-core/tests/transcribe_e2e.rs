//! End-to-end transcription of synthetic hummed melodies.

use hummr_core::AudioBuffer;
use hummr_core::io::midi;
use hummr_core::rhythm::Grid;
use hummr_core::synthetic::{self, HumParams, TWINKLE, Tone};
use hummr_core::theory::{Key, Scale};
use hummr_core::transcribe::{
    ChangeKind, KeyChoice, KeySource, TempoSource, TranscribeOptions, transcribe,
};

fn hummed(notes: &[(u8, f64)], bpm: f64, params: HumParams, seed: u64) -> AudioBuffer {
    let sr = params.sample_rate;
    let tones = synthetic::melody(notes, bpm, 0.35, 0.02, seed);
    AudioBuffer::new(synthetic::hum(&tones, &params, seed), sr)
}

fn sloppy_singer(sr: u32) -> HumParams {
    HumParams {
        vibrato_cents: 35.0,
        detune_cents: -28.0,
        drift_cents: 10.0,
        noise_db: -38.0,
        ..HumParams::new(sr)
    }
}

#[test]
fn twinkle_comes_out_as_the_right_notes() {
    for (sr, bpm, seed) in [(44_100, 100.0, 1), (48_000, 88.0, 2), (22_050, 120.0, 3)] {
        let audio = hummed(&TWINKLE, bpm, sloppy_singer(sr), seed);
        let t = transcribe(&audio, &TranscribeOptions::default()).unwrap();

        let got: Vec<u8> = t.notes.iter().map(|n| n.pitch).collect();
        let want: Vec<u8> = TWINKLE.iter().map(|n| n.0).collect();
        assert_eq!(got, want, "pitches at {sr} Hz / {bpm} BPM");

        assert_eq!(t.key, Some(Key::new(0, Scale::Major)), "key at {bpm} BPM");
        assert!(matches!(t.key_source, KeySource::Detected(_)));
        assert!(
            (t.tempo_bpm - bpm).abs() < 2.0,
            "tempo {} vs {bpm}",
            t.tempo_bpm
        );
        assert!(matches!(t.tempo_source, TempoSource::Detected(_)));
        assert!(
            (t.tuning_offset + 0.28).abs() < 0.1,
            "tuning {}",
            t.tuning_offset
        );

        // Rhythm: quarter notes with half notes at positions 6 and 13.
        let beats: Vec<f64> = t.notes.iter().map(|n| n.start_beats).collect();
        let want_beats: Vec<f64> = TWINKLE
            .iter()
            .scan(0.0, |acc, n| {
                let b = *acc;
                *acc += n.1;
                Some(b)
            })
            .collect();
        assert_eq!(beats, want_beats, "onsets at {bpm} BPM");
        assert_eq!(t.notation().split_whitespace().count(), 28);
    }
}

#[test]
fn out_of_key_note_is_snapped_and_explained() {
    // C major phrase with one F# sung slightly flat: should become F.
    let melody = [
        (60, 1.0),
        (64, 1.0),
        (67, 1.0),
        (66, 1.0),
        (64, 1.0),
        (62, 1.0),
        (60, 2.0),
    ];
    let tones: Vec<Tone> = synthetic::melody(&melody, 100.0, 0.3, 0.0, 9)
        .into_iter()
        .map(|mut t| {
            if t.midi == 66.0 {
                t.midi = 65.8;
            }
            t
        })
        .collect();
    let params = HumParams {
        drift_cents: 0.0,
        ..HumParams::new(44_100)
    };
    let audio = AudioBuffer::new(synthetic::hum(&tones, &params, 9), 44_100);
    let opts = TranscribeOptions {
        key: KeyChoice::Fixed(Key::new(0, Scale::Major)),
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    let got: Vec<u8> = t.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(got, vec![60, 64, 67, 65, 64, 62, 60]);

    let pitch_changes: Vec<&str> = t
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Pitch)
        .map(|c| c.message.as_str())
        .collect();
    assert_eq!(pitch_changes.len(), 1, "{pitch_changes:?}");
    assert!(
        pitch_changes[0].starts_with("moved F#4 to F4: outside C major"),
        "{}",
        pitch_changes[0]
    );
    assert_eq!(
        t.changes
            .iter()
            .find(|c| c.kind == ChangeKind::Pitch)
            .unwrap()
            .note,
        Some(3)
    );
}

#[test]
fn key_off_keeps_chromatic_notes() {
    let melody = [(60, 1.0), (61, 1.0), (62, 1.0), (63, 1.0), (64, 2.0)];
    let audio = hummed(&melody, 100.0, HumParams::new(44_100), 4);
    let opts = TranscribeOptions {
        key: KeyChoice::Off,
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    let got: Vec<u8> = t.notes.iter().map(|n| n.pitch).collect();
    assert_eq!(got, vec![60, 61, 62, 63, 64]);
    assert_eq!(t.key, None);
}

#[test]
fn no_quantize_keeps_seconds_timing() {
    let audio = hummed(&TWINKLE[..7], 100.0, HumParams::new(44_100), 5);
    let opts = TranscribeOptions {
        quantize: None,
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    assert_eq!(t.notes.len(), 7);
    assert_eq!(t.notes[0].start_beats, 0.0);
    // Off-grid values survive.
    assert!(
        t.notes
            .iter()
            .any(|n| (n.start_beats * 2.0).fract().abs() > 0.01)
    );
}

#[test]
fn transcription_midi_golden() {
    // Deterministic synthetic take → MIDI dump snapshot. If the DSP changes
    // on purpose, review the diff with `cargo insta review`.
    let audio = hummed(&TWINKLE, 96.0, sloppy_singer(44_100), 42);
    let opts = TranscribeOptions {
        quantize: Some(Grid::EIGHTH),
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    let bytes = midi::write_sequence(&t.to_sequence("twinkle"));
    insta::assert_snapshot!(midi::dump(&bytes).unwrap());
}
