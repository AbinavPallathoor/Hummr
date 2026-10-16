//! `hummr snap`: tighten the timing of a recording to a beat grid.

use std::path::PathBuf;

use anyhow::{Result, bail};
use hummr_core::AudioBuffer;
use hummr_core::beat::phase_for_tempo;
use hummr_core::io::wav;
use hummr_core::onset::detect_onsets;
use hummr_core::rhythm::Grid;
use hummr_core::slice::{cut_slices, render_slices, snap_slices};

#[derive(clap::Args)]
pub struct Args {
    /// Recording to snap (WAV).
    input: PathBuf,

    /// Tempo of the recording in BPM.
    #[arg(long)]
    bpm: f64,

    /// How far each hit moves towards the grid: 0 (not at all) to 1 (exactly on it).
    #[arg(long, default_value_t = 1.0)]
    strength: f64,

    /// Grid to snap to: 1/4, 1/8, 1/16 or 1/8t.
    #[arg(long, default_value = "1/16")]
    grid: String,

    /// Output WAV [default: input name with -snapped].
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    if !(20.0..=400.0).contains(&args.bpm) {
        bail!("--bpm must be between 20 and 400 (try `hummr detect` to find it)");
    }
    if !(0.0..=1.0).contains(&args.strength) {
        bail!("--strength must be between 0 and 1");
    }
    let grid: Grid = args.grid.parse()?;
    let audio = wav::read_mono(&args.input)?;
    let sr = audio.sample_rate;
    let output = args.output.clone().unwrap_or_else(|| {
        let stem = args
            .input
            .file_stem()
            .map_or_else(|| "take".into(), |s| s.to_string_lossy().into_owned());
        args.input.with_file_name(format!("{stem}-snapped.wav"))
    });

    let spb = 60.0 / args.bpm;
    let phase = phase_for_tempo(&audio.samples, sr, args.bpm);
    let onsets: Vec<f64> = detect_onsets(&audio.samples, sr)
        .iter()
        .map(|o| o.time)
        .collect();
    // Beat positions measured from the first beat; the output keeps the
    // input's timing frame, so unmoved hits stay exactly where they were.
    let k0 = (phase / spb).ceil();
    let shift = k0 * spb - phase;
    let dur = audio.duration_seconds();
    let mut slices = cut_slices(0.0, dur, &onsets, sr, |t| (t + shift) / spb);
    snap_slices(&mut slices, 0.0, grid.beats(), args.strength, spb);
    let rendered = render_slices(&audio, &slices, spb, 0.0, false);
    let skip = ((shift * f64::from(sr)).round() as usize).min(rendered.len());
    let mut out = rendered[skip..].to_vec();
    out.resize(audio.samples.len(), 0.0);
    wav::write_mono(&output, &AudioBuffer::new(out, sr), wav::WavFormat::Pcm16)?;
    let moved = slices.iter().filter(|s| s.hit.is_some()).count();
    println!(
        "snapped {moved} hits to {grid} at {} BPM (strength {:.0}%) → {}",
        hummr_core::beat::fmt_bpm(args.bpm),
        args.strength * 100.0,
        output.display()
    );
    Ok(())
}
