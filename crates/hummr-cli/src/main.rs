//! `hummr`: Hummr command-line interface.

mod detect;
mod project;
mod record;
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
                  Run `hummr` with no arguments to open the editor.\n\n\
                  Examples:\n  hummr                          open the editor\n  \
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
    /// Open the terminal editor (the default when no command is given).
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
}

#[derive(Args, Clone, Default)]
struct EditArgs {
    /// A .wav take or .mid file to open.
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
    hummr_tui::run(hummr_tui::Options {
        file: args.file,
        animations,
        mouse: args.mouse,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        None => edit(cli.edit),
        Some(Command::Edit(args)) => edit(args),
        Some(Command::Transcribe(args)) => transcribe::run(args),
        Some(Command::Record(args)) => record::run(args),
        Some(Command::New(args)) => project::new(args),
        Some(Command::Export(args)) => project::export(args),
        Some(Command::Detect(args)) => detect::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
