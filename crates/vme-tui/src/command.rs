//! The `:` command line.

use vme_core::fuzzy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Quit { force: bool },
    Write(Option<String>),
    WriteQuit,
    Export(String),
    Open(String),
    Key(String),
    Tempo(String),
    Quantize(String),
    Instrument(String),
    Record,
    Play,
    Stop,
    Animations(bool),
    Mouse(bool),
    Retranscribe,
    Undo,
    Help,
}

/// Names shown in suggestions, with their usage.
pub const COMMANDS: [(&str, &str); 16] = [
    ("w", ":w [file.mid]       save MIDI"),
    ("q", ":q  :q!             quit / discard"),
    ("wq", ":wq                 save and quit"),
    ("export", ":export song.wav    WAV mix or .mid"),
    ("open", ":open take.wav      open .wav or .mid"),
    ("key", ":key D minor        or auto / none"),
    ("tempo", ":tempo 100          or auto"),
    ("quantize", ":quantize 1/8       1/16, 1/8t, off"),
    ("instrument", ":instrument violin  or press i"),
    ("record", ":record             or press r"),
    ("play", ":play               or press space"),
    ("stop", ":stop               stop playback"),
    ("retranscribe", ":retranscribe       redo from the take"),
    ("anim", ":anim on|off        animations"),
    ("mouse", ":mouse on|off       mouse support"),
    ("help", ":help               this screen (?)"),
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
        "record" | "rec" | "r" => Cmd::Record,
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
    }

    #[test]
    fn helpful_errors() {
        assert!(parse(":tempo").unwrap_err().contains("usage"));
        assert!(parse(":tmepo 90").unwrap_err().contains(":tempo"));
        assert!(parse(":anim maybe").unwrap_err().contains("on|off"));
    }
}
