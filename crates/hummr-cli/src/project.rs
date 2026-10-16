//! `hummr new` and `hummr export`.

use std::path::PathBuf;

use anyhow::{Result, bail};
use hummr_core::project::{Project, export};

#[derive(clap::Args)]
pub struct NewArgs {
    /// Name of the song; creates NAME/NAME.hummr and NAME/sources/.
    name: String,
}

pub fn new(args: NewArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let p = Project::create_in(&cwd, &args.name)?;
    let path = p.path.expect("created projects have a path");
    let shown = path
        .strip_prefix(&cwd)
        .unwrap_or(&path)
        .display()
        .to_string();
    println!("created {shown}. Open it with: hummr {shown}");
    Ok(())
}

#[derive(clap::Args)]
pub struct ExportArgs {
    /// The project to export (a .hummr file).
    project: PathBuf,

    /// Output file: .wav for the audio mix, .mid for the notes clips.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

pub fn export(args: ExportArgs) -> Result<()> {
    if args.project.extension().is_none_or(|e| e != "hummr") {
        bail!(
            "{} is not a Hummr project (.hummr). To turn a recording into MIDI use `hummr transcribe`",
            args.project.display()
        );
    }
    let p = Project::open(&args.project)?;
    for w in &p.warnings {
        eprintln!("warning: {w}");
    }
    let out = args
        .output
        .unwrap_or_else(|| args.project.with_extension("wav"));
    let e = export::export(&p.song, &p.sources, &out)?;
    if e.notes > 0 {
        println!(
            "wrote {} ({} notes in {} part{})",
            out.display(),
            e.notes,
            e.parts,
            if e.parts == 1 { "" } else { "s" }
        );
    } else {
        println!("wrote {} ({:.1} s stereo mix)", out.display(), e.seconds);
    }
    Ok(())
}
