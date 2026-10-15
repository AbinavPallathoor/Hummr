use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use vme_core::audio::Recorder;
use vme_core::dsp::amp_to_db;
use vme_core::io::wav;

#[derive(clap::Args)]
pub struct Args {
    /// Where to write the take.
    #[arg(short, long, default_value = "take.wav")]
    output: PathBuf,

    /// Stop automatically after this many seconds (default: press Enter).
    #[arg(long)]
    seconds: Option<f64>,
}

pub fn run(args: Args) -> Result<()> {
    let rec = Recorder::start()?;
    println!(
        "recording from {} at {} Hz. Hum away!{}",
        rec.device_name(),
        rec.sample_rate(),
        if args.seconds.is_none() {
            " Press Enter to stop."
        } else {
            ""
        }
    );

    let done = Arc::new(AtomicBool::new(false));
    if args.seconds.is_none() {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            // Only a real line stops recording; a closed stdin (EOF) does not.
            if let Some(Ok(_)) = std::io::stdin().lock().lines().next() {
                done.store(true, Ordering::Relaxed);
            }
        });
    }

    let start = Instant::now();
    let mut warned = false;
    while !done.load(Ordering::Relaxed)
        && args
            .seconds
            .is_none_or(|s| start.elapsed().as_secs_f64() < s)
    {
        std::thread::sleep(Duration::from_millis(50));
        let db = amp_to_db(rec.take_peak()).max(-60.0);
        let bars = ((db + 60.0) / 60.0 * 30.0) as usize;
        print!(
            "\r  {:5.1}s  [{:<30}] {:>5.1} dBFS ",
            rec.seconds(),
            "█".repeat(bars),
            db
        );
        let _ = std::io::stdout().flush();
        if !warned && rec.seconds() > 1.5 && !rec.heard_anything() {
            warned = true;
            println!(
                "\n  warning: the microphone is sending pure silence. {}",
                vme_core::audio::mic_permission_hint()
            );
        }
    }
    println!();
    let audio = rec.stop()?;
    wav::write_mono(&args.output, &audio, wav::WavFormat::Pcm16)?;
    println!(
        "wrote {} ({:.1} s). Next: vme transcribe {} --explain   or   vme edit {}",
        args.output.display(),
        audio.duration_seconds(),
        args.output.display(),
        args.output.display()
    );
    Ok(())
}
