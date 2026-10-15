//! General MIDI instrument names, friendly aliases and lookup.

use crate::{Error, Result, fuzzy};

pub const GM_NAMES: [&str; 128] = [
    "Acoustic Grand Piano",
    "Bright Acoustic Piano",
    "Electric Grand Piano",
    "Honky-tonk Piano",
    "Electric Piano 1",
    "Electric Piano 2",
    "Harpsichord",
    "Clavinet",
    "Celesta",
    "Glockenspiel",
    "Music Box",
    "Vibraphone",
    "Marimba",
    "Xylophone",
    "Tubular Bells",
    "Dulcimer",
    "Drawbar Organ",
    "Percussive Organ",
    "Rock Organ",
    "Church Organ",
    "Reed Organ",
    "Accordion",
    "Harmonica",
    "Tango Accordion",
    "Acoustic Guitar (nylon)",
    "Acoustic Guitar (steel)",
    "Electric Guitar (jazz)",
    "Electric Guitar (clean)",
    "Electric Guitar (muted)",
    "Overdriven Guitar",
    "Distortion Guitar",
    "Guitar Harmonics",
    "Acoustic Bass",
    "Electric Bass (finger)",
    "Electric Bass (pick)",
    "Fretless Bass",
    "Slap Bass 1",
    "Slap Bass 2",
    "Synth Bass 1",
    "Synth Bass 2",
    "Violin",
    "Viola",
    "Cello",
    "Contrabass",
    "Tremolo Strings",
    "Pizzicato Strings",
    "Orchestral Harp",
    "Timpani",
    "String Ensemble 1",
    "String Ensemble 2",
    "Synth Strings 1",
    "Synth Strings 2",
    "Choir Aahs",
    "Voice Oohs",
    "Synth Voice",
    "Orchestra Hit",
    "Trumpet",
    "Trombone",
    "Tuba",
    "Muted Trumpet",
    "French Horn",
    "Brass Section",
    "Synth Brass 1",
    "Synth Brass 2",
    "Soprano Sax",
    "Alto Sax",
    "Tenor Sax",
    "Baritone Sax",
    "Oboe",
    "English Horn",
    "Bassoon",
    "Clarinet",
    "Piccolo",
    "Flute",
    "Recorder",
    "Pan Flute",
    "Blown Bottle",
    "Shakuhachi",
    "Whistle",
    "Ocarina",
    "Lead 1 (square)",
    "Lead 2 (sawtooth)",
    "Lead 3 (calliope)",
    "Lead 4 (chiff)",
    "Lead 5 (charang)",
    "Lead 6 (voice)",
    "Lead 7 (fifths)",
    "Lead 8 (bass + lead)",
    "Pad 1 (new age)",
    "Pad 2 (warm)",
    "Pad 3 (polysynth)",
    "Pad 4 (choir)",
    "Pad 5 (bowed)",
    "Pad 6 (metallic)",
    "Pad 7 (halo)",
    "Pad 8 (sweep)",
    "FX 1 (rain)",
    "FX 2 (soundtrack)",
    "FX 3 (crystal)",
    "FX 4 (atmosphere)",
    "FX 5 (brightness)",
    "FX 6 (goblins)",
    "FX 7 (echoes)",
    "FX 8 (sci-fi)",
    "Sitar",
    "Banjo",
    "Shamisen",
    "Koto",
    "Kalimba",
    "Bagpipe",
    "Fiddle",
    "Shanai",
    "Tinkle Bell",
    "Agogo",
    "Steel Drums",
    "Woodblock",
    "Taiko Drum",
    "Melodic Tom",
    "Synth Drum",
    "Reverse Cymbal",
    "Guitar Fret Noise",
    "Breath Noise",
    "Seashore",
    "Bird Tweet",
    "Telephone Ring",
    "Helicopter",
    "Applause",
    "Gunshot",
];

/// Short names people actually type.
const ALIASES: [(&str, u8); 24] = [
    ("piano", 0),
    ("grand", 0),
    ("epiano", 4),
    ("rhodes", 4),
    ("organ", 16),
    ("guitar", 25),
    ("acoustic guitar", 25),
    ("electric guitar", 27),
    ("distortion", 30),
    ("bass", 33),
    ("synth bass", 38),
    ("strings", 48),
    ("choir", 52),
    ("trumpet", 56),
    ("horn", 60),
    ("brass", 61),
    ("sax", 65),
    ("saxophone", 65),
    ("flute", 73),
    ("whistle", 78),
    ("synth", 80),
    ("lead", 81),
    ("pad", 89),
    ("harp", 46),
];

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn name(program: u8) -> &'static str {
    GM_NAMES[usize::from(program.min(127))]
}

/// Finds a GM program by exact name, alias, program number or fuzzy match.
pub fn find(query: &str) -> Result<u8> {
    let q = normalize(query);
    if q.is_empty() {
        return Err(Error::parse(
            "instrument",
            query,
            "give an instrument name like violin",
        ));
    }
    if let Ok(n) = q.parse::<u8>() {
        if n <= 127 {
            return Ok(n);
        }
    }
    if let Some(i) = GM_NAMES.iter().position(|n| normalize(n) == q) {
        return Ok(i as u8);
    }
    if let Some((_, p)) = ALIASES.iter().find(|(a, _)| *a == q) {
        return Ok(*p);
    }
    search(query).first().map(|(p, _)| *p).ok_or_else(|| {
        Error::parse(
            "instrument",
            query,
            "no General MIDI instrument matches; try violin, flute, piano, sax",
        )
    })
}

/// Fuzzy search over names and aliases, best first, one entry per program.
pub fn search(query: &str) -> Vec<(u8, i32)> {
    let names = GM_NAMES.iter().enumerate().map(|(i, n)| (i as u8, *n));
    let aliases = ALIASES.iter().map(|(a, p)| (*p, *a));
    let mut ranked = fuzzy::rank(query, names.chain(aliases));
    let mut seen = [false; 128];
    ranked.retain(|(p, _)| !std::mem::replace(&mut seen[usize::from(*p)], true));
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(find("violin").unwrap(), 40);
        assert_eq!(find("Violin").unwrap(), 40);
        assert_eq!(find("flute").unwrap(), 73);
        assert_eq!(find("sax").unwrap(), 65);
        assert_eq!(find("acoustic guitar nylon").unwrap(), 24);
        assert_eq!(find("40").unwrap(), 40);
        assert_eq!(find("cllo").unwrap(), 42);
        assert!(find("zzzz").is_err());
        assert_eq!(name(40), "Violin");
    }

    #[test]
    fn search_is_deduplicated() {
        let r = search("flute");
        assert_eq!(r[0].0, 73);
        let mut ps: Vec<u8> = r.iter().map(|x| x.0).collect();
        ps.sort_unstable();
        ps.dedup();
        assert_eq!(ps.len(), r.len());
        assert_eq!(search("").len(), 128);
    }
}
