//! mp3, flac and ogg (Vorbis) decoding through symphonia (cargo feature
//! `decode`). Everything is mixed down to mono at the file's own rate.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::{AudioBuffer, Error, Result};

fn fail(path: &Path, what: impl std::fmt::Display) -> Error {
    Error::Audio(format!(
        "couldn't read {} ({what}). Try converting it to WAV",
        path.display()
    ))
}

/// Decodes an audio file to mono.
pub fn decode_file(path: &Path) -> Result<AudioBuffer> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| fail(path, e))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| fail(path, "no audio in it"))?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| fail(path, "unknown codec"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| fail(path, e))?;
    let track_id = track.id;
    let mut mono: Vec<f32> = Vec::new();
    let mut rate = 0u32;
    let mut frame: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(fail(path, e)),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let channels = buf.spec().channels().count().max(1);
                rate = buf.spec().rate();
                frame.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut frame);
                mono.extend(
                    frame
                        .chunks(channels)
                        .map(|c| c.iter().sum::<f32>() / channels as f32),
                );
            }
            Err(SymError::DecodeError(_)) => {}
            Err(e) => return Err(fail(path, e)),
        }
    }
    if rate == 0 || mono.is_empty() {
        return Err(fail(path, "it contains no sound"));
    }
    Ok(AudioBuffer::new(mono, rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc8(data: &[u8]) -> u8 {
        let mut c = 0u8;
        for &b in data {
            c ^= b;
            for _ in 0..8 {
                c = if c & 0x80 != 0 {
                    (c << 1) ^ 0x07
                } else {
                    c << 1
                };
            }
        }
        c
    }

    fn crc16(data: &[u8]) -> u16 {
        let mut c = 0u16;
        for &b in data {
            c ^= u16::from(b) << 8;
            for _ in 0..8 {
                c = if c & 0x8000 != 0 {
                    (c << 1) ^ 0x8005
                } else {
                    c << 1
                };
            }
        }
        c
    }

    /// A minimal FLAC file: mono, 16-bit, uncompressed ("verbatim") frames.
    fn flac(samples: &[i16], rate: u32) -> Vec<u8> {
        let block = 4_096usize;
        let mut out = b"fLaC".to_vec();
        out.extend([0x80, 0, 0, 34]);
        out.extend((block as u16).to_be_bytes());
        out.extend((block as u16).to_be_bytes());
        out.extend([0u8; 6]);
        // rate (20 bits) | channels-1 (3) | bps-1 (5) | total samples (36).
        let packed: u64 = (u64::from(rate) << 44) | (15u64 << 36) | samples.len() as u64;
        out.extend(packed.to_be_bytes());
        out.extend([0u8; 16]);
        for (n, chunk) in samples.chunks(block).enumerate() {
            assert!(n < 128);
            let mut f = vec![0xFF, 0xF8, 0x70, 0x08, n as u8];
            f.extend(((chunk.len() - 1) as u16).to_be_bytes());
            f.push(crc8(&f));
            f.push(0x02);
            for s in chunk {
                f.extend(s.to_be_bytes());
            }
            f.extend(crc16(&f).to_be_bytes());
            out.extend(f);
        }
        out
    }

    #[test]
    fn decodes_flac() {
        let rate = 44_100;
        let x: Vec<i16> = (0..rate as usize)
            .map(|i| {
                (8_000.0 * (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin()) as i16
            })
            .collect();
        let dir = std::env::temp_dir().join(format!("hummr-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.flac");
        std::fs::write(&path, flac(&x, rate)).unwrap();
        let a = decode_file(&path).unwrap();
        assert_eq!(a.sample_rate, rate);
        assert_eq!(a.samples.len(), x.len());
        for (got, want) in a.samples.iter().zip(&x) {
            assert!((got - f32::from(*want) / 32_768.0).abs() < 1e-4);
        }
        // Imported into a project, it is resampled to the project rate.
        let mut p = crate::project::Project::new("t");
        let id = p.import_file(&path).unwrap();
        let src = p.sources.get(&id).unwrap();
        assert_eq!(src.sample_rate, crate::project::PROJECT_RATE);
        assert!((src.duration_seconds() - 1.0).abs() < 1e-3);
        let e = decode_file(&dir.join("missing.mp3"))
            .unwrap_err()
            .to_string();
        assert!(!e.is_empty());
    }
}
