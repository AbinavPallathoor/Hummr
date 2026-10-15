//! WAV reading (any common PCM/float format, downmixed to mono) and writing.

use std::path::Path;

use crate::{AudioBuffer, Error, Result};

/// Reads a WAV file and downmixes it to mono `f32`.
pub fn read_mono(path: &Path) -> Result<AudioBuffer> {
    let wav_err = |reason: String| Error::Wav {
        path: path.to_path_buf(),
        reason,
    };
    if !path.exists() {
        return Err(Error::io(
            path,
            std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        ));
    }
    let reader = hound::WavReader::open(path).map_err(|e| match e {
        hound::Error::IoError(io) => Error::io(path, io),
        other => wav_err(format!(
            "{other} (only .wav is supported; convert other formats first, e.g. `ffmpeg -i take.m4a take.wav`)"
        )),
    })?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err(wav_err("file declares zero channels".into()));
    }
    let channels = usize::from(spec.channels);

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| wav_err(e.to_string()))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| wav_err(e.to_string()))?
        }
    };

    let samples = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(AudioBuffer::new(samples, spec.sample_rate))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Float32,
}

/// Writes interleaved samples. Values outside [-1, 1] are clipped for PCM.
pub fn write(
    path: &Path,
    samples: &[f32],
    channels: u16,
    sample_rate: u32,
    format: WavFormat,
) -> Result<()> {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: match format {
            WavFormat::Pcm16 => 16,
            WavFormat::Float32 => 32,
        },
        sample_format: match format {
            WavFormat::Pcm16 => hound::SampleFormat::Int,
            WavFormat::Float32 => hound::SampleFormat::Float,
        },
    };
    let to_err = |e: hound::Error| match e {
        hound::Error::IoError(io) => Error::io(path, io),
        other => Error::Wav {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(to_err)?;
    for &s in samples {
        match format {
            WavFormat::Pcm16 => w
                .write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                .map_err(to_err)?,
            WavFormat::Float32 => w.write_sample(s).map_err(to_err)?,
        }
    }
    w.finalize().map_err(to_err)
}

pub fn write_mono(path: &Path, audio: &AudioBuffer, format: WavFormat) -> Result<()> {
    write(path, &audio.samples, 1, audio.sample_rate, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::sine;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("hummr-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn round_trips_pcm16_and_float() {
        let audio = AudioBuffer::new(sine(440.0, 22_050, 0.1, 0.5), 22_050);
        for fmt in [WavFormat::Pcm16, WavFormat::Float32] {
            let p = tmp(&format!("{fmt:?}.wav"));
            write_mono(&p, &audio, fmt).unwrap();
            let back = read_mono(&p).unwrap();
            std::fs::remove_file(&p).ok();
            assert_eq!(back.sample_rate, 22_050);
            assert_eq!(back.samples.len(), audio.samples.len());
            for (a, b) in back.samples.iter().zip(&audio.samples) {
                assert!((a - b).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn stereo_is_downmixed() {
        let p = tmp("stereo.wav");
        write(&p, &[0.5, -0.5, 0.25, 0.75], 2, 8_000, WavFormat::Float32).unwrap();
        let back = read_mono(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(back.samples, vec![0.0, 0.5]);
    }

    #[test]
    fn helpful_errors() {
        let missing = read_mono(Path::new("/definitely/not/here.wav")).unwrap_err();
        assert!(missing.to_string().contains("not found"), "{missing}");

        let p = tmp("garbage.wav");
        std::fs::write(&p, b"this is not a wav file at all").unwrap();
        let bad = read_mono(&p).unwrap_err();
        std::fs::remove_file(&p).ok();
        assert!(bad.to_string().contains("not a usable WAV"), "{bad}");
    }
}
