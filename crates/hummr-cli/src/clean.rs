//! `hummr clean`: turn down steady background noise in a recording.

use std::path::PathBuf;

use anyhow::{Result, bail};
use hummr_core::AudioBuffer;
use hummr_core::denoise;
use hummr_core::io::wav;

#[derive(clap::Args)]
#[command(after_help = denoise::HELP)]
pub struct Args {
    /// Recording to clean (WAV).
    input: PathBuf,

    /// How much to remove: 0 (nothing) to 1 (as much as possible).
    #[arg(long, default_value_t = 0.8)]
    strength: f64,

    /// Learn the noise from this stretch, START:LENGTH in seconds
    /// [default: the first 300 ms].
    #[arg(long, value_name = "START:LENGTH")]
    noise: Option<String>,

    /// Output WAV [default: input name with -clean].
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

fn parse_region(s: &str) -> Result<(f64, f64)> {
    let parsed = s
        .split_once(':')
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
    match parsed {
        Some((start, len)) if start >= 0.0 && len > 0.0 => Ok((start, len)),
        _ => bail!("--noise should look like 0.5:1 (start and length in seconds)"),
    }
}

pub fn run(args: Args) -> Result<()> {
    if !(0.0..=1.0).contains(&args.strength) {
        bail!("--strength must be between 0 and 1");
    }
    let audio = wav::read_mono(&args.input)?;
    let sr = audio.sample_rate;
    let output = args.output.clone().unwrap_or_else(|| {
        let stem = args
            .input
            .file_stem()
            .map_or_else(|| "take".into(), |s| s.to_string_lossy().into_owned());
        args.input.with_file_name(format!("{stem}-clean.wav"))
    });
    let range = match &args.noise {
        Some(s) => {
            let (start, len) = parse_region(s)?;
            let a = (start * f64::from(sr)) as usize;
            let b = ((start + len) * f64::from(sr)) as usize;
            if b > audio.samples.len() {
                bail!(
                    "--noise goes past the end of the recording ({:.1} s long)",
                    audio.duration_seconds()
                );
            }
            a..b
        }
        None => denoise::default_region(audio.samples.len(), sr),
    };
    let profile = denoise::learn(&audio.samples[range.clone()], sr);
    let out = denoise::denoise(&audio.samples, sr, &profile, args.strength);
    let before = hummr_core::dsp::rms(&audio.samples[range.clone()]);
    let after = hummr_core::dsp::rms(&out[range.clone()]);
    wav::write_mono(&output, &AudioBuffer::new(out, sr), wav::WavFormat::Pcm16)?;
    println!(
        "learned the noise from {:.2}–{:.2} s; it is {:.1} dB quieter there → {}",
        range.start as f64 / f64::from(sr),
        range.end as f64 / f64::from(sr),
        hummr_core::dsp::amp_to_db(before) - hummr_core::dsp::amp_to_db(after.max(1e-9)),
        output.display()
    );
    Ok(())
}
