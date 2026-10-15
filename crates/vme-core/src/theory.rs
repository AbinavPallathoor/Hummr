//! Notes, scales, keys, key detection and snapping to a key.

use std::fmt;
use std::str::FromStr;

use crate::{Error, Result};

const SHARP_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLAT_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Name of a MIDI note using sharps, with middle C (60) as `C4`.
pub fn note_name(midi: u8) -> String {
    note_name_spelled(midi, false)
}

pub fn note_name_spelled(midi: u8, flats: bool) -> String {
    let names = if flats { &FLAT_NAMES } else { &SHARP_NAMES };
    let octave = i32::from(midi) / 12 - 1;
    format!("{}{}", names[usize::from(midi % 12)], octave)
}

/// Formats a pitch offset in cents as e.g. `+12¢` or `-30¢`.
pub fn format_cents(semitones: f32) -> String {
    let c = (semitones * 100.0).round() as i32;
    if c >= 0 {
        format!("+{c}¢")
    } else {
        format!("{c}¢")
    }
}

/// Parses a letter plus accidentals (`C`, `f#`, `Bb`, `E♭`, `C##`) from the
/// start of `s`. Returns the (unwrapped) pitch class and the remainder, so
/// `Cb` gives -1 and `B#` gives 12.
fn parse_pc_prefix(s: &str) -> Option<(i32, &str)> {
    let mut chars = s.char_indices();
    let (_, letter) = chars.next()?;
    let mut pc = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut rest = &s[letter.len_utf8()..];
    for (i, c) in chars {
        match c {
            '#' | '♯' => pc += 1,
            'b' | '♭' => pc -= 1,
            _ => {
                rest = &s[i..];
                return Some((pc, rest));
            }
        }
        rest = &s[i + c.len_utf8()..];
    }
    Some((pc, rest))
}

/// Parses a note name with octave, such as `C4`, `f#3`, `Bb2` or `C-1`.
pub fn parse_note(s: &str) -> Result<u8> {
    let hint = "expected a note like C4, F#3 or Bb2 (middle C is C4)";
    let t = s.trim();
    let (pc, rest) = parse_pc_prefix(t).ok_or_else(|| Error::parse("note", s, hint))?;
    let octave: i32 = rest.parse().map_err(|_| Error::parse("note", s, hint))?;
    let midi = (octave + 1) * 12 + pc;
    u8::try_from(midi)
        .ok()
        .filter(|m| *m <= 127)
        .ok_or_else(|| Error::parse("note", s, "note is outside the MIDI range C-1..G9"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scale {
    Major,
    Minor,
    HarmonicMinor,
    Dorian,
    Mixolydian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    Chromatic,
}

impl Scale {
    pub const ALL: [Scale; 9] = [
        Scale::Major,
        Scale::Minor,
        Scale::HarmonicMinor,
        Scale::Dorian,
        Scale::Mixolydian,
        Scale::MajorPentatonic,
        Scale::MinorPentatonic,
        Scale::Blues,
        Scale::Chromatic,
    ];

    /// Semitone offsets from the tonic.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scale::Major => "major",
            Scale::Minor => "minor",
            Scale::HarmonicMinor => "harmonic minor",
            Scale::Dorian => "dorian",
            Scale::Mixolydian => "mixolydian",
            Scale::MajorPentatonic => "major pentatonic",
            Scale::MinorPentatonic => "minor pentatonic",
            Scale::Blues => "blues",
            Scale::Chromatic => "chromatic",
        }
    }

    pub fn is_minor(self) -> bool {
        matches!(
            self,
            Scale::Minor
                | Scale::HarmonicMinor
                | Scale::Dorian
                | Scale::MinorPentatonic
                | Scale::Blues
        )
    }

    /// Semitones from this scale's tonic up to the tonic of the major key
    /// sharing its key signature.
    fn relative_major_offset(self) -> u8 {
        match self {
            Scale::Major | Scale::MajorPentatonic | Scale::Chromatic => 0,
            Scale::Minor | Scale::HarmonicMinor | Scale::MinorPentatonic | Scale::Blues => 3,
            Scale::Dorian => 10,
            Scale::Mixolydian => 5,
        }
    }

    fn parse(s: &str) -> Option<Scale> {
        if s == "M" {
            return Some(Scale::Major);
        }
        let norm: String = s
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect::<String>()
            .to_lowercase();
        Some(match norm.as_str() {
            "" | "maj" | "major" | "ionian" => Scale::Major,
            "m" | "min" | "minor" | "aeolian" | "naturalminor" => Scale::Minor,
            "harmonicminor" | "harmonic" | "harm" => Scale::HarmonicMinor,
            "dorian" => Scale::Dorian,
            "mixolydian" | "mixo" => Scale::Mixolydian,
            "pentatonic" | "pent" | "majorpentatonic" | "majpent" => Scale::MajorPentatonic,
            "minorpentatonic" | "minpent" | "mpent" | "mpentatonic" => Scale::MinorPentatonic,
            "blues" => Scale::Blues,
            "chromatic" => Scale::Chromatic,
            _ => return None,
        })
    }
}

/// A key: a tonic pitch class (0 = C) and a scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub tonic: u8,
    pub scale: Scale,
}

/// Number of sharps (positive) or flats (negative) in each major key.
const MAJOR_SIGNATURES: [i8; 12] = [0, -5, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5];

impl Key {
    pub fn new(tonic: u8, scale: Scale) -> Self {
        Self {
            tonic: tonic % 12,
            scale,
        }
    }

    /// Whether the pitch class of `midi` belongs to this key.
    pub fn contains(&self, midi: u8) -> bool {
        let rel = (midi % 12 + 12 - self.tonic) % 12;
        self.scale.intervals().contains(&rel)
    }

    /// Key-signature sharps (positive) or flats (negative).
    pub fn sharps(&self) -> i8 {
        let major = (self.tonic + self.scale.relative_major_offset()) % 12;
        MAJOR_SIGNATURES[usize::from(major)]
    }

    pub fn prefers_flats(&self) -> bool {
        self.sharps() < 0
    }

    /// Spells a MIDI note the way this key would (Bb in F major, A# in B major).
    pub fn note_name(&self, midi: u8) -> String {
        note_name_spelled(midi, self.prefers_flats())
    }

    pub fn tonic_name(&self) -> &'static str {
        let names = if self.prefers_flats() {
            &FLAT_NAMES
        } else {
            &SHARP_NAMES
        };
        names[usize::from(self.tonic)]
    }

    /// The in-key pitch closest to the sung (fractional MIDI) pitch.
    pub fn snap(&self, sung: f32) -> Snap {
        let nearest = sung.round().clamp(0.0, 127.0) as u8;
        if self.contains(nearest) {
            return Snap {
                pitch: nearest,
                nearest_chromatic: nearest,
                other_option: None,
            };
        }
        let below = (1..12)
            .map(|d| i32::from(nearest) - d)
            .find(|&p| p >= 0 && self.contains(p as u8));
        let above = (1..12)
            .map(|d| i32::from(nearest) + d)
            .find(|&p| p <= 127 && self.contains(p as u8));
        let (pitch, other) = match (below, above) {
            (Some(b), Some(a)) => {
                // Ties go down: humming tends to overshoot upwards.
                if (a as f32 - sung).abs() < (sung - b as f32).abs() {
                    (a, Some(b))
                } else {
                    (b, Some(a))
                }
            }
            (Some(b), None) => (b, None),
            (None, Some(a)) => (a, None),
            (None, None) => (i32::from(nearest), None),
        };
        Snap {
            pitch: pitch as u8,
            nearest_chromatic: nearest,
            other_option: other.map(|p| p as u8),
        }
    }
}

/// Result of [`Key::snap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snap {
    pub pitch: u8,
    /// The pitch the singer was closest to before snapping.
    pub nearest_chromatic: u8,
    /// The in-key neighbour on the other side, if the note moved.
    pub other_option: Option<u8>,
}

impl Snap {
    pub fn moved(&self) -> bool {
        self.pitch != self.nearest_chromatic
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.tonic_name(), self.scale.name())
    }
}

impl FromStr for Key {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let hint =
            "expected something like 'C major', 'D minor', 'F#m', 'Bb' or 'A minor pentatonic'";
        let t = s.trim();
        let (pc, rest) = parse_pc_prefix(t).ok_or_else(|| Error::parse("key", s, hint))?;
        let scale = Scale::parse(rest.trim()).ok_or_else(|| Error::parse("key", s, hint))?;
        Ok(Key::new(pc.rem_euclid(12) as u8, scale))
    }
}

// Krumhansl–Kessler key profiles.
const KK_MAJOR: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const KK_MINOR: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyEstimate {
    pub key: Key,
    /// Pearson correlation of the pitch-class histogram with the key profile.
    pub score: f64,
    pub runner_up: Key,
    pub runner_up_score: f64,
}

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        num += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        db += (b[i] - mb).powi(2);
    }
    if da == 0.0 || db == 0.0 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

/// Krumhansl–Schmuckler key finding over `(midi pitch, weight)` pairs,
/// usually weighted by note duration. Considers major and natural minor keys.
pub fn detect_key(notes: &[(u8, f64)]) -> Option<KeyEstimate> {
    let mut hist = [0.0f64; 12];
    for &(p, w) in notes {
        hist[usize::from(p % 12)] += w.max(0.0);
    }
    if hist.iter().all(|&h| h == 0.0) {
        return None;
    }
    let mut scored: Vec<(Key, f64)> = Vec::with_capacity(24);
    for tonic in 0..12u8 {
        for (scale, profile) in [(Scale::Major, &KK_MAJOR), (Scale::Minor, &KK_MINOR)] {
            let mut rotated = [0.0; 12];
            for (i, r) in rotated.iter_mut().enumerate() {
                *r = profile[(i + 12 - usize::from(tonic)) % 12];
            }
            scored.push((Key::new(tonic, scale), pearson(&hist, &rotated)));
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Some(KeyEstimate {
        key: scored[0].0,
        score: scored[0].1,
        runner_up: scored[1].0,
        runner_up_score: scored[1].1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_round_trip() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
        assert_eq!(note_name_spelled(70, true), "Bb4");
        for m in 0..=127u8 {
            assert_eq!(parse_note(&note_name(m)).unwrap(), m);
            assert_eq!(parse_note(&note_name_spelled(m, true)).unwrap(), m);
        }
        assert_eq!(parse_note("Cb4").unwrap(), 59);
        assert_eq!(parse_note("B#3").unwrap(), 60);
        assert_eq!(parse_note("e♭4").unwrap(), 63);
        assert!(parse_note("H4").is_err());
        assert!(parse_note("C").is_err());
        assert!(parse_note("G10").is_err());
    }

    #[test]
    fn hz_midi_conversion() {
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((hz_to_midi(261.6256) - 60.0).abs() < 1e-3);
    }

    #[test]
    fn parse_keys() {
        let k: Key = "D minor".parse().unwrap();
        assert_eq!(k, Key::new(2, Scale::Minor));
        assert_eq!("F#m".parse::<Key>().unwrap(), Key::new(6, Scale::Minor));
        assert_eq!("Bb".parse::<Key>().unwrap(), Key::new(10, Scale::Major));
        assert_eq!("Bbm".parse::<Key>().unwrap(), Key::new(10, Scale::Minor));
        assert_eq!("C".parse::<Key>().unwrap(), Key::new(0, Scale::Major));
        assert_eq!("AM".parse::<Key>().unwrap(), Key::new(9, Scale::Major));
        assert_eq!(
            "a minor pentatonic".parse::<Key>().unwrap(),
            Key::new(9, Scale::MinorPentatonic)
        );
        assert!("X major".parse::<Key>().is_err());
        assert!("C lydian-ish".parse::<Key>().is_err());
    }

    #[test]
    fn key_display_and_signature() {
        assert_eq!(Key::new(10, Scale::Major).to_string(), "Bb major");
        assert_eq!(Key::new(2, Scale::Minor).to_string(), "D minor");
        assert_eq!(Key::new(2, Scale::Minor).sharps(), -1);
        assert_eq!(Key::new(9, Scale::Minor).sharps(), 0);
        assert_eq!(Key::new(4, Scale::Major).sharps(), 4);
    }

    #[test]
    fn snap_out_of_key_note_to_nearest() {
        let c = Key::new(0, Scale::Major);
        // Sung F#4 slightly flat: F4 is nearer than G4.
        let s = c.snap(65.8);
        assert_eq!(s.nearest_chromatic, 66);
        assert_eq!(s.pitch, 65);
        assert_eq!(s.other_option, Some(67));
        assert!(s.moved());
        // Sung F#4 slightly sharp: G4 wins.
        assert_eq!(c.snap(66.2).pitch, 67);
        // In-key notes stay.
        let e = c.snap(64.3);
        assert_eq!(e.pitch, 64);
        assert!(!e.moved());
    }

    #[test]
    fn detects_c_major_and_a_minor() {
        // C major scale up and down, tonic held longer.
        let c_major: Vec<(u8, f64)> = [60, 62, 64, 65, 67, 69, 71, 72, 67, 64, 60]
            .iter()
            .enumerate()
            .map(|(i, &p)| (p, if i == 0 || i == 10 { 2.0 } else { 1.0 }))
            .collect();
        assert_eq!(detect_key(&c_major).unwrap().key, Key::new(0, Scale::Major));

        let a_minor: Vec<(u8, f64)> = [57, 60, 64, 57, 59, 60, 62, 64, 57, 52, 57]
            .iter()
            .map(|&p| (p, if p == 57 { 2.0 } else { 1.0 }))
            .collect();
        assert_eq!(detect_key(&a_minor).unwrap().key, Key::new(9, Scale::Minor));
    }

    #[test]
    fn detects_g_major_from_f_sharp() {
        let g: Vec<(u8, f64)> = [67, 69, 71, 72, 74, 76, 78, 79, 74, 71, 67]
            .iter()
            .map(|&p| (p, if p == 67 || p == 79 { 2.0 } else { 1.0 }))
            .collect();
        assert_eq!(detect_key(&g).unwrap().key, Key::new(7, Scale::Major));
    }
}
