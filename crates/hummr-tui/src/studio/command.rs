//! The timeline's `:` command line.

use std::path::PathBuf;

use hummr_core::project::export;

use super::Studio;
use crate::app::Level;

/// Commands shown in `?` help, with their usage.
pub const COMMANDS: &[(&str, &str)] = &[
    (":w [file.hummr]", "save (to a new file if given)"),
    (":q  :q!  :wq", "quit / quit without saving / save and quit"),
    (
        ":export song.wav",
        "export a WAV (or .mid with the notes clips)",
    ),
    (":anim on|off", "animations"),
    (":help", "this help"),
];

/// Runs one command line (without the leading `:`).
pub fn run(s: &mut Studio, line: &str) {
    let line = line.trim().trim_start_matches(':').trim();
    let (name, arg) = match line.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (line, ""),
    };
    match name {
        "" => {}
        "w" | "write" | "save" => {
            save(s, arg);
        }
        "q" | "quit" => s.request_quit(false),
        "q!" | "quit!" => s.request_quit(true),
        "wq" | "x" => {
            if save(s, arg) {
                s.request_quit(true);
            }
        }
        "export" => export_to(s, arg),
        "track" => {
            let (sub, rest) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
            match sub {
                "name" | "rename" => s.rename_track(rest),
                "delete" | "del" | "remove" => s.delete_track(),
                "new" | "add" => {
                    let name = (!rest.trim().is_empty()).then(|| rest.trim().to_string());
                    s.new_track(name);
                }
                _ => s.say(
                    "Use :track name NAME, :track new [NAME] or :track delete.",
                    Level::Error,
                ),
            }
        }
        "anim" | "animations" => match arg {
            "on" | "" => {
                s.animations = true;
                s.say("Animations on.", Level::Info);
            }
            "off" => {
                s.animations = false;
                s.say("Animations off.", Level::Info);
            }
            _ => s.say("Use :anim on or :anim off.", Level::Error),
        },
        "help" | "h" => s.mode = super::Mode::Help,
        other => s.say(
            format!("There is no :{other} command. Press ? to see the commands."),
            Level::Error,
        ),
    }
}

/// Saves; returns whether it worked.
fn save(s: &mut Studio, arg: &str) -> bool {
    let result = if arg.is_empty() {
        s.project.save()
    } else {
        let mut path = PathBuf::from(arg);
        if path.extension().is_none() {
            path.set_extension("hummr");
        }
        s.project.save_as(&path)
    };
    match result {
        Ok(()) => {
            s.mark_saved();
            let shown = s
                .project
                .path
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string());
            s.say(format!("Saved {shown}."), Level::Info);
            true
        }
        Err(e) => {
            s.say(format!("Couldn't save: {e}"), Level::Error);
            false
        }
    }
}

fn export_to(s: &mut Studio, arg: &str) {
    if arg.is_empty() {
        s.say(
            "Say where to export, e.g. :export song.wav or :export song.mid",
            Level::Error,
        );
        return;
    }
    let mut path = PathBuf::from(arg);
    if path.is_relative() {
        if let Some(dir) = s.project.dir() {
            path = dir.join(path);
        }
    }
    match export::export(&s.project.song, &s.project.sources, &path) {
        Ok(e) if e.notes > 0 => s.say(
            format!("Exported {} notes to {}.", e.notes, path.display()),
            Level::Info,
        ),
        Ok(e) => s.say(
            format!("Exported {:.1} s to {}.", e.seconds, path.display()),
            Level::Info,
        ),
        Err(e) => s.say(format!("Couldn't export: {e}"), Level::Error),
    }
}
