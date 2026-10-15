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

#[test]
fn anchored_grid_lines_up_with_a_count_in() {
    // One bar of silence (the count-in) at 100 BPM, then the melody starts
    // exactly on beat 0; a stray note during the count-in must be dropped.
    let bpm = 100.0;
    let count_in = 4.0 * 60.0 / bpm;
    let mut tones = synthetic::melody(&[(64, 1.0), (67, 1.0), (69, 2.0)], bpm, count_in, 0.01, 7);
    tones.insert(0, Tone::new(60.0, 0.6, 0.4));
    let audio = AudioBuffer::new(synthetic::hum(&tones, &HumParams::new(44_100), 7), 44_100);
    let opts = TranscribeOptions {
        tempo: hummr_core::transcribe::TempoChoice::Fixed(bpm),
        key: KeyChoice::Fixed(Key::new(0, Scale::Major)),
        grid_origin: Some(count_in),
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    let got: Vec<(u8, f64)> = t.notes.iter().map(|n| (n.pitch, n.start_beats)).collect();
    assert_eq!(got, vec![(64, 0.0), (67, 1.0), (69, 2.0)]);
    assert!((t.origin_seconds - count_in).abs() < 1e-9);
    assert!(t.changes.iter().any(|c| c.message.contains("count-in")));
}

/// Hums `notes` at 100 BPM with each sung tone shaped by `shape(t, tone)`.
fn shaped(
    notes: &[(u8, f64)],
    params: HumParams,
    shape: impl Fn(f64, &Tone) -> f32,
) -> AudioBuffer {
    let tones = synthetic::melody(notes, 100.0, 0.4, 0.0, 5);
    let mut x = synthetic::hum(&tones, &params, 5);
    let sr = f64::from(params.sample_rate);
    for (i, s) in x.iter_mut().enumerate() {
        let t = i as f64 / sr;
        if let Some(tone) = tones
            .iter()
            .find(|tn| t >= tn.start && t < tn.start + tn.duration)
        {
            *s *= shape(t - tone.start, tone);
        }
    }
    AudioBuffer::new(x, params.sample_rate)
}

#[test]
fn held_notes_keep_their_full_length() {
    // Short notes around a 4-beat and a 3-beat hold, each sung with a
    // natural breath afterwards (15% of the note).
    let melody = [
        (60, 1.0),
        (62, 1.0),
        (64, 4.0),
        (62, 0.5),
        (60, 0.5),
        (67, 3.0),
        (65, 1.0),
    ];
    let p = || HumParams::new(44_100);
    let cases: Vec<(&str, AudioBuffer)> = vec![
        ("steady", shaped(&melody, p(), |_, _| 1.0)),
        // The voice fades by 18 dB over the hold.
        (
            "fading",
            shaped(&melody, p(), |t, _| 10f32.powf(-(t as f32) * 7.5 / 20.0)),
        ),
        // Slow loudness swells inside each note must not split it.
        (
            "swelling",
            shaped(&melody, p(), |t, _| {
                (1.0 - 0.55 * (0.5 + 0.5 * (t * std::f64::consts::TAU * 1.3).sin())) as f32
            }),
        ),
        (
            "wide vibrato",
            shaped(
                &melody,
                HumParams {
                    vibrato_cents: 70.0,
                    ..p()
                },
                |_, _| 1.0,
            ),
        ),
        (
            "drifting",
            shaped(
                &melody,
                HumParams {
                    drift_cents: 30.0,
                    ..p()
                },
                |_, _| 1.0,
            ),
        ),
    ];
    for (name, audio) in cases {
        let t = transcribe(&audio, &TranscribeOptions::default()).unwrap();
        let got: Vec<(u8, f64)> = t
            .notes
            .iter()
            .map(|n| (n.pitch, n.duration_beats))
            .collect();
        assert_eq!(got, melody.to_vec(), "{name}");
    }
}

#[test]
fn rests_after_held_notes_are_kept() {
    // Half note then a quarter rest, whole note then a half rest: real rests
    // are longer than a breath and must survive.
    let tones = vec![
        Tone::new(64.0, 0.4, 1.2 * 0.85),
        Tone::new(67.0, 0.4 + 1.8, 2.4 * 0.85),
        Tone::new(65.0, 0.4 + 1.8 + 3.6, 0.6 * 0.85),
        Tone::new(64.0, 0.4 + 1.8 + 3.6 + 0.6, 0.6 * 0.85),
    ];
    let audio = AudioBuffer::new(synthetic::hum(&tones, &HumParams::new(44_100), 2), 44_100);
    let opts = TranscribeOptions {
        tempo: hummr_core::transcribe::TempoChoice::Fixed(100.0),
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts).unwrap();
    assert_eq!(t.notation(), "E4 1/2 r 1/4 G4 1 r 1/2 F4 1/4 E4 1/4");
}
