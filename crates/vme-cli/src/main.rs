//! `vme`: Voice Music Editor command-line interface.

mod record;
mod transcribe;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "vme",
    version,
    about = "Voice Music Editor: hum a part, get real instrument notes",
    long_about = "Voice Music Editor: hum or sing a part, and vme turns it into notes \
                  you can play with a real instrument sound.\n\n\
                  Run `vme` with no arguments to open the editor.\n\n\
                  Examples:\n  vme                         open the editor\n  \
                  vme edit takes/take-001.wav open a recording in the editor\n  \
                  vme transcribe take.wav --key auto --quantize 1/8 -o lead.mid --explain"
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
    let animations = !args.no_anim && std::env::var_os("VME_NO_ANIM").is_none();
    vme_tui::run(vme_tui::Options {
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
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
