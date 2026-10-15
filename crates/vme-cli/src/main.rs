//! `vme`: Voice Music Editor command-line interface.

mod transcribe;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "vme",
    version,
    about = "Voice Music Editor: hum a part, get real instrument notes",
    long_about = "Voice Music Editor: hum or sing a part, and vme turns it into notes \
                  you can render with a real instrument sound.\n\n\
                  Example:\n  vme transcribe take.wav --key auto --quantize 1/8 -o lead.mid --explain"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Turn a hummed recording (WAV) into MIDI notes.
    Transcribe(transcribe::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Transcribe(args) => transcribe::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
