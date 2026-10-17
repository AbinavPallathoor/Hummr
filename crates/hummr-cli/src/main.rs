//! `hummr`: Hummr command-line interface.

mod clean;
mod detect;
mod project;
mod record;
mod snap;
mod transcribe;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "hummr",
    version,
    about = "Hummr: hum a part, get real instrument notes",
    long_about = "Hummr: hum or sing a part, and hummr turns it into notes \
                  you can play with a real instrument sound.\n\n\
                  Run `hummr` with no arguments to open the song in this folder \
                  (or start a new one with a beat to hum over).\n\n\
                  Examples:\n  hummr                          open or start a song here\n  \
                  hummr edit takes/take-001.wav  open a recording in the editor\n  \
                  hummr transcribe take.wav --key auto --quantize 1/8 -o lead.mid --explain"
)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    edit: EditArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Open a project on the timeline, or a .wav/.mid in the notes editor.
    Edit(EditArgs),
    /// Turn a hummed recording (WAV) into MIDI notes.
    Transcribe(transcribe::Args),
    /// Record from the default microphone to a WAV file.
    Record(record::Args),
    /// Create a new song project (NAME/NAME.hummr).
    New(project::NewArgs),
    /// Render a project to a WAV mix (or its notes to MIDI).
    Export(project::ExportArgs),
    /// Find the tempo, beat and hits of a recording.
    Detect(detect::Args),
    /// Move the hits of a recording onto a beat grid.
    Snap(snap::Args),
    /// Turn down steady background hiss or hum in a recording.
    Clean(clean::Args),
}

#[derive(Args, Clone, Default)]
struct EditArgs {
    /// A .hummr project (opens the timeline), or a .wav take or .mid file
    /// (opens the notes editor).
    file: Option<PathBuf>,

    /// Turn off animations (fades and sweeps).
    #[arg(long)]
    no_anim: bool,

    /// Enable mouse support (click to select notes, scroll the roll).
    #[arg(long)]
    mouse: bool,
}

fn edit(args: EditArgs) -> anyhow::Result<()> {
    let animations = !args.no_anim && std::env::var_os("HUMMR_NO_ANIM").is_none();
    let opts = hummr_tui::Options {
        file: args.file.clone(),
        animations,
        mouse: args.mouse,
    };
    match &args.file {
        // Projects open on the timeline; .wav and .mid in the notes editor.
        Some(f) if f.extension().is_some_and(|e| e == "hummr") => hummr_tui::run_project(f, opts),
        _ => hummr_tui::run(opts),
    }
}

/// `hummr` alone: the project in this folder, or a starter song.
fn start(args: EditArgs) -> anyhow::Result<()> {
    let animations = !args.no_anim && std::env::var_os("HUMMR_NO_ANIM").is_none();
    hummr_tui::run_start(hummr_tui::Options {
        file: None,
        animations,
        mouse: args.mouse,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        None if cli.edit.file.is_none() => start(cli.edit),
        None => edit(cli.edit),
        Some(Command::Edit(args)) => edit(args),
        Some(Command::Transcribe(args)) => transcribe::run(args),
        Some(Command::Record(args)) => record::run(args),
        Some(Command::New(args)) => project::new(args),
        Some(Command::Export(args)) => project::export(args),
        Some(Command::Detect(args)) => detect::run(args),
        Some(Command::Snap(args)) => snap::run(args),
        Some(Command::Clean(args)) => clean::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
