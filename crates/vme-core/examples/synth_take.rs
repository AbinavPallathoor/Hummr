//! Writes a synthetic hummed take (Twinkle Twinkle, slightly flat, with
//! vibrato and breath noise) so `vme transcribe` can be tried without a mic.
//!
//! cargo run -p vme-core --example synth_take -- take.wav [bpm]

use std::path::PathBuf;

use vme_core::AudioBuffer;
use vme_core::io::wav::{WavFormat, write_mono};
use vme_core::synthetic::{self, HumParams, TWINKLE};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "take.wav".into()));
    let bpm: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(100.0);
    let params = HumParams {
        vibrato_cents: 35.0,
        detune_cents: -28.0,
        noise_db: -38.0,
        ..HumParams::new(44_100)
    };
    let tones = synthetic::melody(&TWINKLE, bpm, 0.35, 0.02, 1);
    let audio = AudioBuffer::new(synthetic::hum(&tones, &params, 1), 44_100);
    write_mono(&out, &audio, WavFormat::Pcm16).expect("write wav");
    println!(
        "wrote {} ({:.1} s at {bpm} BPM)",
        out.display(),
        audio.duration_seconds()
    );
}
