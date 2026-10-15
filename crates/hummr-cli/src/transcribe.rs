use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hummr_core::dsp::amp_to_db;
use hummr_core::io::{midi, wav};
use hummr_core::rhythm::Grid;
use hummr_core::theory::format_cents;
use hummr_core::transcribe::{
    ChangeKind, KeyChoice, KeySource, TempoChoice, TempoSource, TranscribeOptions, transcribe,
};

#[derive(clap::Args)]
pub struct Args {
    /// Recording to transcribe (WAV, any sample rate, mono or stereo).
    input: PathBuf,

    /// Output MIDI file [default: input name with .mid].
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Key to snap notes into: `auto`, `none`, or a key like `C major`, `D minor`, `F#m`.
    #[arg(long, default_value = "auto")]
    key: String,

    /// Grid to quantise to: `1/4`, `1/8`, `1/16`, `1/8t` (triplets) or `off`.
    #[arg(long, default_value = "1/8")]
    quantize: String,

    /// Tempo in BPM, or `auto` to detect it from the note onsets.
    #[arg(long, default_value = "auto")]
    tempo: String,

    /// Print why each note was changed (snapped, moved, merged or dropped).
    #[arg(long)]
    explain: bool,

    /// Do not correct for singing consistently sharp or flat.
    #[arg(long)]
    no_tuning: bool,

    /// Write the frame-by-frame pitch track as CSV (for debugging).
    #[arg(long, value_name = "FILE")]
    pitch_csv: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let key: KeyChoice = args.key.parse()?;
    let quantize = Grid::parse_optional(&args.quantize)?;
    let tempo: TempoChoice = args.tempo.parse()?;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| args.input.with_extension("mid"));

    let audio = wav::read_mono(&args.input)?;
    let mut opts = TranscribeOptions {
        key,
        quantize,
        tempo,
        ..TranscribeOptions::default()
    };
    opts.segment.correct_tuning = !args.no_tuning;

    let t = transcribe(&audio, &opts)
        .with_context(|| format!("transcribing {}", args.input.display()))?;

    if let Some(csv) = &args.pitch_csv {
        std::fs::write(csv, t.pitch_track.to_csv())
            .with_context(|| format!("writing {}", csv.display()))?;
        println!("wrote pitch track to {}", csv.display());
    }

    let name = args.input.display();
    println!(
        "{name}  {:.1} s · {:.1} kHz",
        t.duration_seconds,
        f64::from(audio.sample_rate) / 1000.0
    );

    if t.notes.is_empty() {
        let peak_db = amp_to_db(audio.peak());
        if peak_db < -60.0 {
            bail!(
                "no sung notes found: the recording is silent (peak {peak_db:.0} dBFS).\n\
                 If you recorded it with this machine's microphone, check that your terminal app \
                 has microphone permission."
            );
        }
        bail!(
            "no sung notes found (peak level {peak_db:.0} dBFS). Try humming louder and closer to the \
             microphone, with clear pitched sounds like 'mmm', 'doo' or 'la'."
        );
    }

    let cents = (t.tuning_offset * 100.0).round();
    if cents.abs() >= 3.0 {
        println!(
            "  tuning   sung {}¢ {} on average; corrected",
            cents.abs(),
            if cents < 0.0 { "flat" } else { "sharp" }
        );
    } else {
        println!("  tuning   in tune");
    }

    match (t.key, t.key_source) {
        (Some(k), KeySource::Detected(est)) => println!(
            "  key      {k}  (detected, score {:.2}; runner-up {} {:.2})",
            est.score, est.runner_up, est.runner_up_score
        ),
        (Some(k), _) => println!("  key      {k}  (given)"),
        (None, _) => println!("  key      none (notes kept at nearest semitone)"),
    }

    let grid = t
        .grid
        .map_or("no quantize".to_string(), |g| format!("{g} grid"));
    match t.tempo_source {
        TempoSource::Detected(e) => println!(
            "  tempo    {} BPM  (detected, {grid}, fit {:.2})",
            e.bpm, e.fit
        ),
        TempoSource::Given(e) => println!(
            "  tempo    {} BPM  (given, {grid}, fit {:.2})",
            e.bpm, e.fit
        ),
        TempoSource::Default => {
            println!("  tempo    120 BPM  (too few notes to detect; use --tempo)")
        }
    }

    let snapped = t.changes_of(ChangeKind::Pitch);
    let dropped = t.changes_of(ChangeKind::Dropped) + t.changes_of(ChangeKind::Merged);
    let mut extras = Vec::new();
    if snapped > 0 {
        extras.push(format!("{snapped} snapped to key"));
    }
    if dropped > 0 {
        extras.push(format!("{dropped} blips removed"));
    }
    let extras = if extras.is_empty() {
        String::new()
    } else {
        format!("  ({})", extras.join(", "))
    };
    println!("  notes    {}{extras}", t.notes.len());
    println!();
    for line in wrap(&t.notation(), 76) {
        println!("  {line}");
    }
    println!();

    if args.explain {
        print_explanation(&t);
    } else if !t.changes.is_empty() {
        println!(
            "  ({} adjustments made; add --explain to see why)",
            t.changes.len()
        );
        println!();
    }

    let stem = args
        .input
        .file_stem()
        .map_or_else(|| "take".to_string(), |s| s.to_string_lossy().into_owned());
    midi::write_file(&output, &t.to_sequence(&stem))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn print_explanation(t: &hummr_core::transcribe::Transcription) {
    println!("why notes changed:");
    if t.changes.is_empty() {
        println!("  nothing: every note was already in key and on the grid");
    }
    for c in &t.changes {
        let id = c
            .note
            .map_or("  -".to_string(), |i| format!("#{:<2}", i + 1));
        let kind = match c.kind {
            ChangeKind::Pitch => "pitch",
            ChangeKind::Timing => "timing",
            ChangeKind::Length => "length",
            ChangeKind::Merged => "merged",
            ChangeKind::Dropped => "dropped",
        };
        println!("  {id}  {:>7.3}s  {kind:<7} {}", c.time, c.message);
    }
    println!();
    println!("notes as sung:");
    for (i, n) in t.notes.iter().enumerate() {
        let d = n.detected;
        let nearest = d.nearest_pitch();
        let name = t.key.map_or_else(
            || hummr_core::theory::note_name(nearest),
            |k| k.note_name(nearest),
        );
        println!(
            "  #{:<2} {:>7.3}s  {:>4} ms  sung {name:<4} {:>5}  vel {}",
            i + 1,
            d.start,
            (d.duration() * 1000.0).round(),
            format_cents(d.sung_pitch - f32::from(nearest)),
            n.velocity
        );
    }
    println!();
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    // Keep "note length" pairs together.
    let tokens: Vec<&str> = text.split_whitespace().collect();
    for pair in tokens.chunks(2) {
        let item = pair.join(" ");
        if !line.is_empty() && line.len() + 1 + item.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&item);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
