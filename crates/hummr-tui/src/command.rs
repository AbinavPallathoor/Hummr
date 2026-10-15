//! The `:` command line.

use hummr_core::fuzzy;

use crate::song::Placement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Quit {
        force: bool,
    },
    Write(Option<String>),
    WriteQuit,
    Export(String),
    Open(String),
    Key(String),
    Tempo(String),
    Quantize(String),
    Instrument(String),
    /// `:track`, `:track 2`, `:track new cello`, `:track del`, `:track name Bass`.
    Track(String),
    Transpose(i32),
    /// `:notes C4 1/4 E4 1/8`: type notes in at the playhead.
    Notes(String),
    /// `:record` (new layer) or `:record overwrite` (this track).
    Record(Placement),
    Play,
    Stop,
    Animations(bool),
    Mouse(bool),
    Retranscribe,
    Undo,
    Help,
}

/// Names shown in suggestions, with their usage.
pub const COMMANDS: [(&str, &str); 19] = [
    ("w", ":w [file.mid]       save MIDI"),
    ("q", ":q  :q!             quit / discard"),
    ("wq", ":wq                 save + quit"),
    ("export", ":export song.wav    WAV or .mid"),
    ("open", ":open take.wav      .wav or .mid"),
    ("record", ":record [overwrite] same as r / R"),
    ("notes", ":notes C4 1/4 E4    type notes at ▼"),
    ("track", ":track new cello    or 2, del, name X"),
    ("instrument", ":instrument violin  same as i"),
    ("transpose", ":transpose -12      shift this track"),
    ("key", ":key D minor        or auto / none"),
    ("tempo", ":tempo 100          or auto"),
    ("quantize", ":quantize 1/8       1/16, 1/8t, off"),
    ("play", ":play               same as space"),
    ("stop", ":stop               stop playback"),
    ("retranscribe", ":retranscribe       redo the 1st take"),
    ("anim", ":anim on|off        animations"),
    ("mouse", ":mouse on|off       mouse support"),
    ("help", ":help               this help"),
];

fn on_off(arg: &str, name: &str) -> Result<bool, String> {
    match arg.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" | "" => Ok(true),
        "off" | "no" | "false" | "0" => Ok(false),
        _ => Err(format!("usage: :{name} on|off")),
    }
}

fn need(arg: &str, usage: &str) -> Result<String, String> {
    if arg.is_empty() {
        Err(format!("usage: {usage}"))
    } else {
        Ok(arg.to_string())
    }
}

pub fn parse(input: &str) -> Result<Cmd, String> {
    let input = input.trim().trim_start_matches(':').trim();
    let (name, arg) = match input.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (input, ""),
    };
    let opt = |a: &str| (!a.is_empty()).then(|| a.to_string());
    Ok(match name {
        "q" | "quit" => Cmd::Quit { force: false },
        "q!" | "quit!" => Cmd::Quit { force: true },
        "w" | "write" | "save" => Cmd::Write(opt(arg)),
        "wq" | "x" => Cmd::WriteQuit,
        "export" | "exp" => Cmd::Export(need(arg, ":export song.wav (or .mid)")?),
        "open" | "o" | "e" | "edit" => Cmd::Open(need(arg, ":open take.wav")?),
        "key" | "k" => Cmd::Key(need(arg, ":key D minor | auto | none")?),
        "tempo" | "bpm" | "t" => Cmd::Tempo(need(arg, ":tempo 100 | auto")?),
        "quantize" | "quant" | "grid" => Cmd::Quantize(need(arg, ":quantize 1/8 | off")?),
        "instrument" | "inst" | "i" => Cmd::Instrument(need(arg, ":instrument violin")?),
        "record" | "rec" | "r" => Cmd::Record(match arg.to_ascii_lowercase().as_str() {
            "" | "o" | "overlay" | "layer" | "new" => Placement::Overlay,
            "w" | "overwrite" | "over" | "replace" => Placement::Overwrite,
            _ => return Err("usage: :record (new layer) or :record overwrite (this track)".into()),
        }),
        "track" | "tr" => Cmd::Track(arg.to_string()),
        "notes" | "n" | "note" => Cmd::Notes(need(arg, ":notes C4 1/4 E4 1/8 r 1/8 G4 1/2")?),
        "transpose" | "tp" => Cmd::Transpose(
            arg.trim_start_matches('+')
                .parse()
                .map_err(|_| "usage: :transpose -12 (semitones; 12 is an octave)".to_string())?,
        ),
        "play" | "p" => Cmd::Play,
        "stop" => Cmd::Stop,
        "anim" | "animations" => Cmd::Animations(on_off(arg, "anim")?),
        "mouse" => Cmd::Mouse(on_off(arg, "mouse")?),
        "retranscribe" | "re" | "transcribe" => Cmd::Retranscribe,
        "u" | "undo" => Cmd::Undo,
        "h" | "help" | "?" => Cmd::Help,
        "" => return Err("type a command, e.g. :help".into()),
        other => {
            return Err(match suggest(other) {
                Some(n) => format!("unknown command :{other}; did you mean :{n}?"),
                None => format!("unknown command :{other}; try :help"),
            });
        }
    })
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != *cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest command by typo distance, falling back to fuzzy subsequence.
fn suggest(typed: &str) -> Option<&'static str> {
    let close = COMMANDS
        .iter()
        .map(|(n, _)| (*n, edit_distance(typed, n)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d);
    close.map(|(n, _)| n).or_else(|| {
        fuzzy::rank(typed, COMMANDS.iter().map(|(n, _)| (*n, *n)))
            .first()
            .map(|(n, _)| *n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse(":tempo 100"), Ok(Cmd::Tempo("100".into())));
        assert_eq!(parse("key D minor"), Ok(Cmd::Key("D minor".into())));
        assert_eq!(parse(":quantize 1/8"), Ok(Cmd::Quantize("1/8".into())));
        assert_eq!(
            parse(":export song.wav"),
            Ok(Cmd::Export("song.wav".into()))
        );
        assert_eq!(parse(":w"), Ok(Cmd::Write(None)));
        assert_eq!(
            parse(":w lead.mid"),
            Ok(Cmd::Write(Some("lead.mid".into())))
        );
        assert_eq!(parse(":q!"), Ok(Cmd::Quit { force: true }));
        assert_eq!(parse(":anim off"), Ok(Cmd::Animations(false)));
        assert_eq!(parse(":i violin"), Ok(Cmd::Instrument("violin".into())));
        assert_eq!(parse(":rec"), Ok(Cmd::Record(Placement::Overlay)));
        assert_eq!(
            parse(":record overwrite"),
            Ok(Cmd::Record(Placement::Overwrite))
        );
        assert_eq!(
            parse(":track new cello"),
            Ok(Cmd::Track("new cello".into()))
        );
        assert_eq!(parse(":transpose +12"), Ok(Cmd::Transpose(12)));
        assert_eq!(parse(":notes C4 1/4"), Ok(Cmd::Notes("C4 1/4".into())));
        assert!(parse(":transpose up").is_err());
        assert!(parse(":record sideways").is_err());
    }

    #[test]
    fn helpful_errors() {
        assert!(parse(":tempo").unwrap_err().contains("usage"));
        assert!(parse(":tmepo 90").unwrap_err().contains(":tempo"));
        assert!(parse(":anim maybe").unwrap_err().contains("on|off"));
    }
}
