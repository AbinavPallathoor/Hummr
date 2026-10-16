//! `hummr detect`: tempo, beat and hits of a recording.

use std::path::PathBuf;

use anyhow::Result;
use hummr_core::beat::{estimate, fmt_bpm};
use hummr_core::io::wav;

#[derive(clap::Args)]
pub struct Args {
    /// Recording to analyse (WAV).
    input: PathBuf,

    /// Also list every hit with its time.
    #[arg(long)]
    hits: bool,
}

pub fn run(args: Args) -> Result<()> {
    let audio = wav::read_mono(&args.input)?;
    let e = estimate(&audio.samples, audio.sample_rate);
    println!("{}: {}", args.input.display(), e.describe());
    if e.has_beat() {
        println!(
            "  tempo {} BPM (confidence {:.0}%){}",
            fmt_bpm(e.bpm),
            e.confidence * 100.0,
            match e.alternatives.first() {
                Some(a) => format!(", could also be counted as {} BPM", fmt_bpm(*a)),
                None => String::new(),
            }
        );
        println!(
            "  first beat at {:.3} s, bar 1 likely starts at {:.3} s",
            e.phase, e.downbeat
        );
    } else {
        println!(
            "  Hummr found no steady beat. That's normal for humming or singing; \
             for a beat, tap or beatbox along to the metronome."
        );
    }
    println!("  {} hits found", e.onsets.len());
    if args.hits {
        for o in &e.onsets {
            println!("    {:8.3} s  strength {:.2}", o.time, o.strength);
        }
    }
    Ok(())
}
