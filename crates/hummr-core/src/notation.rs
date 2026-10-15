//! Plain-text note notation used in project files and CLI output:
//! `C4 1/4 E4 1/8 r 1/8 G4 3/8`.
//!
//! Each note or rest (`r`) is followed by a length as a fraction of a whole
//! note. A length may end in `.` (dotted) or `t` (triplet). If a length is
//! omitted, the previous one is reused. `|` bar lines are ignored.

use crate::theory::{Key, parse_note};
use crate::{Error, Result};

/// One parsed item: a note or rest with a length in beats (quarter notes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Item {
    Note { pitch: u8, beats: f64 },
    Rest { beats: f64 },
}

fn parse_length(token: &str) -> Option<f64> {
    let t = token.trim().to_ascii_lowercase();
    let (body, mult) = if let Some(b) = t.strip_suffix('.') {
        (b.to_string(), 1.5)
    } else if let Some(b) = t.strip_suffix('t') {
        (b.to_string(), 2.0 / 3.0)
    } else {
        (t.clone(), 1.0)
    };
    let (n, d) = match body.split_once('/') {
        Some((n, d)) => (n.parse::<u32>().ok()?, d.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 1),
    };
    (n > 0 && d > 0).then(|| 4.0 * f64::from(n) / f64::from(d) * mult)
}

fn looks_like_length(token: &str) -> bool {
    token.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Parses notation into items.
pub fn parse(text: &str) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let mut last_len: Option<f64> = None;
    let tokens: Vec<&str> = text.split_whitespace().filter(|t| *t != "|").collect();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        let is_rest = matches!(tok.to_ascii_lowercase().as_str(), "r" | "rest" | "-");
        let pitch = if is_rest {
            None
        } else {
            Some(parse_note(tok).map_err(|_| {
                Error::parse(
                    "notes",
                    tok,
                    format!("token {} should be a note like C4 or a rest 'r'", i + 1),
                )
            })?)
        };
        let beats = match tokens.get(i + 1).filter(|t| looks_like_length(t)) {
            Some(len_tok) => {
                i += 1;
                parse_length(len_tok).ok_or_else(|| {
                    Error::parse(
                        "note length",
                        len_tok,
                        "expected a length like 1/4, 1/8, 3/8, 1/4. or 1/8t",
                    )
                })?
            }
            None => last_len.ok_or_else(|| {
                Error::parse("notes", tok, "the first note needs a length, e.g. 'C4 1/4'")
            })?,
        };
        last_len = Some(beats);
        items.push(match pitch {
            Some(pitch) => Item::Note { pitch, beats },
            None => Item::Rest { beats },
        });
        i += 1;
    }
    Ok(items)
}

/// Formats a length in beats as a whole-note fraction (`1/8`, `3/8`, `1/12`).
pub fn format_length(beats: f64) -> String {
    let whole = beats / 4.0;
    for den in [1u32, 2, 4, 8, 16, 32, 64, 3, 6, 12, 24, 48, 96] {
        let num = whole * f64::from(den);
        if (num - num.round()).abs() < 1e-6 && num.round() >= 1.0 {
            let n = num.round() as u32;
            let g = gcd(n, den);
            let (n, d) = (n / g, den / g);
            return if d == 1 {
                n.to_string()
            } else {
                format!("{n}/{d}")
            };
        }
    }
    let n = (whole * 64.0).round().max(1.0) as u32;
    let g = gcd(n, 64);
    format!("{}/{}", n / g, 64 / g)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Formats monophonic notes `(pitch, start_beats, duration_beats)` with rests
/// for gaps, spelling notes for `key`.
pub fn format(notes: &[(u8, f64, f64)], key: Option<&Key>) -> String {
    let mut sorted = notes.to_vec();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut out: Vec<String> = Vec::new();
    let mut cursor = sorted.first().map_or(0.0, |n| n.1.min(0.0));
    for (pitch, start, dur) in sorted {
        if start > cursor + 1e-6 {
            out.push(format!("r {}", format_length(start - cursor)));
        }
        let name = key.map_or_else(|| crate::theory::note_name(pitch), |k| k.note_name(pitch));
        out.push(format!("{name} {}", format_length(dur)));
        cursor = cursor.max(start + dur);
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theory::Scale;

    #[test]
    fn parses_notes_rests_and_carried_lengths() {
        let items = parse("C4 1/4 E4 1/8 | r 1/8 G4 A4 3/8 B4 1/4. C5 1/8t").unwrap();
        assert_eq!(
            items,
            vec![
                Item::Note {
                    pitch: 60,
                    beats: 1.0
                },
                Item::Note {
                    pitch: 64,
                    beats: 0.5
                },
                Item::Rest { beats: 0.5 },
                Item::Note {
                    pitch: 67,
                    beats: 0.5
                },
                Item::Note {
                    pitch: 69,
                    beats: 1.5
                },
                Item::Note {
                    pitch: 71,
                    beats: 1.5
                },
                Item::Note {
                    pitch: 72,
                    beats: 1.0 / 3.0
                },
            ]
        );
    }

    #[test]
    fn parse_errors_are_specific() {
        let e = parse("C4 1/4 Q4 1/4").unwrap_err().to_string();
        assert!(e.contains("Q4") && e.contains("token 3"), "{e}");
        let e = parse("C4").unwrap_err().to_string();
        assert!(e.contains("needs a length"), "{e}");
        assert!(parse("C4 1/0").is_err());
    }

    #[test]
    fn formats_lengths() {
        assert_eq!(format_length(1.0), "1/4");
        assert_eq!(format_length(0.5), "1/8");
        assert_eq!(format_length(1.5), "3/8");
        assert_eq!(format_length(4.0), "1");
        assert_eq!(format_length(8.0), "2");
        assert_eq!(format_length(1.0 / 3.0), "1/12");
    }

    #[test]
    fn format_round_trips_with_rests() {
        let notes = [(62u8, 0.0, 1.0), (65, 1.0, 0.5), (70, 2.0, 1.0)];
        let text = format(&notes, Some(&Key::new(2, Scale::Minor)));
        assert_eq!(text, "D4 1/4 F4 1/8 r 1/8 Bb4 1/4");
        let items = parse(&text).unwrap();
        assert_eq!(items.len(), 4);
    }
}
