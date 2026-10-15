//! Standard MIDI File reading and writing via `midly`.

use std::fmt::Write as _;
use std::path::Path;

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use crate::sequence::{PPQ, SeqNote, Sequence};
use crate::theory::{Key, Scale};
use crate::{Error, Result};

fn beats_to_ticks(beats: f64) -> u64 {
    (beats.max(0.0) * f64::from(PPQ)).round() as u64
}

/// Encodes a sequence as a format-0 Standard MIDI File.
pub fn write_sequence(seq: &Sequence) -> Vec<u8> {
    let channel = u4::new(seq.channel.min(15));
    let mut timed: Vec<(u64, u8, TrackEventKind<'_>)> = Vec::new();

    timed.push((
        0,
        0,
        TrackEventKind::Meta(MetaMessage::TrackName(seq.name.as_bytes())),
    ));
    let us_per_quarter = (60_000_000.0 / seq.tempo_bpm.clamp(10.0, 1000.0)).round() as u32;
    timed.push((
        0,
        0,
        TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us_per_quarter))),
    ));
    timed.push((
        0,
        0,
        TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
    ));
    if let Some(key) = seq.key {
        timed.push((
            0,
            0,
            TrackEventKind::Meta(MetaMessage::KeySignature(
                key.sharps(),
                key.scale.is_minor(),
            )),
        ));
    }
    if let Some(program) = seq.program {
        timed.push((
            0,
            0,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::ProgramChange {
                    program: u7::new(program.min(127)),
                },
            },
        ));
    }

    for n in &seq.notes {
        let on = beats_to_ticks(n.start_beats);
        let off = beats_to_ticks(n.start_beats + n.duration_beats).max(on + 1);
        let key = u7::new(n.pitch.min(127));
        // Order within a tick: meta (0), note-off (1), note-on (2), so a
        // repeated note is released before it is struck again.
        timed.push((
            off,
            1,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
        timed.push((
            on,
            2,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key,
                    vel: u7::new(n.velocity.clamp(1, 127)),
                },
            },
        ));
    }
    timed.sort_by_key(|(t, order, _)| (*t, *order));

    let mut track = Vec::with_capacity(timed.len() + 1);
    let mut last = 0u64;
    for (t, _, kind) in timed {
        track.push(TrackEvent {
            delta: u28::new((t - last) as u32),
            kind,
        });
        last = t;
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(PPQ))),
        tracks: vec![track],
    };
    let mut out = Vec::new();
    smf.write_std(&mut out)
        .expect("writing to a Vec cannot fail");
    out
}

pub fn write_file(path: &Path, seq: &Sequence) -> Result<()> {
    std::fs::write(path, write_sequence(seq)).map_err(|e| Error::io(path, e))
}

/// One instrument part of a multi-track song.
#[derive(Clone, Copy, Debug)]
pub struct SongTrack<'a> {
    pub name: &'a str,
    pub program: u8,
    pub notes: &'a [SeqNote],
}

/// A multi-track song read from a MIDI file.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub tempo_bpm: f64,
    pub key: Option<Key>,
    /// One sequence per (track chunk, channel) that contains notes.
    pub tracks: Vec<Sequence>,
}

/// MIDI channel for the `i`-th part, skipping channel 10 (index 9, drums).
pub fn channel_for(index: usize) -> u8 {
    let c = index % 15;
    if c >= 9 { c as u8 + 1 } else { c as u8 }
}

type Timed<'a> = Vec<(u64, u8, TrackEventKind<'a>)>;

fn to_track(mut timed: Timed<'_>) -> Vec<TrackEvent<'_>> {
    timed.sort_by_key(|(t, order, _)| (*t, *order));
    let mut track = Vec::with_capacity(timed.len() + 1);
    let mut last = 0u64;
    for (t, _, kind) in timed {
        track.push(TrackEvent {
            delta: u28::new((t - last) as u32),
            kind,
        });
        last = t;
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    track
}

fn push_notes(timed: &mut Timed<'_>, channel: u4, notes: &[SeqNote]) {
    for n in notes {
        let on = beats_to_ticks(n.start_beats);
        let off = beats_to_ticks(n.start_beats + n.duration_beats).max(on + 1);
        let key = u7::new(n.pitch.min(127));
        timed.push((
            off,
            1,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
        timed.push((
            on,
            2,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key,
                    vel: u7::new(n.velocity.clamp(1, 127)),
                },
            },
        ));
    }
}

/// Encodes a multi-track song as a format-1 Standard MIDI File: a conductor
/// track (tempo, time and key signature) followed by one track per part,
/// each on its own channel with its own instrument.
pub fn write_song(
    name: &str,
    tempo_bpm: f64,
    key: Option<Key>,
    parts: &[SongTrack<'_>],
) -> Vec<u8> {
    let mut conductor: Timed<'_> = vec![
        (
            0,
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
        ),
        (
            0,
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(
                (60_000_000.0 / tempo_bpm.clamp(10.0, 1000.0)).round() as u32,
            ))),
        ),
        (
            0,
            0,
            TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
        ),
    ];
    if let Some(k) = key {
        conductor.push((
            0,
            0,
            TrackEventKind::Meta(MetaMessage::KeySignature(k.sharps(), k.scale.is_minor())),
        ));
    }
    let mut tracks = vec![to_track(conductor)];
    for (i, part) in parts.iter().enumerate() {
        let channel = u4::new(channel_for(i));
        let mut timed: Timed<'_> = vec![
            (
                0,
                0,
                TrackEventKind::Meta(MetaMessage::TrackName(part.name.as_bytes())),
            ),
            (
                0,
                0,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::ProgramChange {
                        program: u7::new(part.program.min(127)),
                    },
                },
            ),
        ];
        push_notes(&mut timed, channel, part.notes);
        tracks.push(to_track(timed));
    }
    let smf = Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(PPQ))),
        tracks,
    };
    let mut out = Vec::new();
    smf.write_std(&mut out)
        .expect("writing to a Vec cannot fail");
    out
}

pub fn write_song_file(
    path: &Path,
    name: &str,
    tempo_bpm: f64,
    key: Option<Key>,
    parts: &[SongTrack<'_>],
) -> Result<()> {
    std::fs::write(path, write_song(name, tempo_bpm, key, parts)).map_err(|e| Error::io(path, e))
}

/// Parses a MIDI file into separate parts, one per (track chunk, channel)
/// that has notes. Works for both format-0 and format-1 files.
pub fn read_song(bytes: &[u8]) -> Result<Song> {
    let smf = Smf::parse(bytes).map_err(|e| Error::Midi(e.to_string()))?;
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => f64::from(t.as_int()),
        Timing::Timecode(..) => {
            return Err(Error::Midi(
                "SMPTE timecode MIDI files are not supported; re-export with musical (PPQ) timing"
                    .into(),
            ));
        }
    };
    let mut song = Song {
        tempo_bpm: 120.0,
        key: None,
        tracks: Vec::new(),
    };
    let mut tempo_set = false;
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0u64;
        let mut name = String::new();
        let mut programs: [Option<u8>; 16] = [None; 16];
        let mut parts: std::collections::BTreeMap<u8, Vec<SeqNote>> = Default::default();
        let mut open: std::collections::HashMap<(u8, u8), Vec<(u64, u8)>> = Default::default();
        for ev in track {
            tick += u64::from(ev.delta.as_int());
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) if !tempo_set => {
                    song.tempo_bpm = 60_000_000.0 / f64::from(t.as_int().max(1));
                    tempo_set = true;
                }
                TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor))
                    if song.key.is_none() =>
                {
                    song.key = key_from_signature(sharps, minor);
                }
                TrackEventKind::Meta(MetaMessage::TrackName(n)) if name.is_empty() => {
                    name = String::from_utf8_lossy(n).into_owned();
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::ProgramChange { program } => {
                            programs[usize::from(ch)].get_or_insert(program.as_int());
                        }
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            open.entry((ch, key.as_int()))
                                .or_default()
                                .push((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(stack) = open.get_mut(&(ch, key.as_int())) {
                                if !stack.is_empty() {
                                    let (start, vel) = stack.remove(0);
                                    parts.entry(ch).or_default().push(SeqNote {
                                        pitch: key.as_int(),
                                        start_beats: start as f64 / ppq,
                                        duration_beats: (tick - start) as f64 / ppq,
                                        velocity: vel,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for (ch, mut notes) in parts {
            notes.sort_by(|a, b| {
                a.start_beats
                    .total_cmp(&b.start_beats)
                    .then(a.pitch.cmp(&b.pitch))
            });
            let mut seq = Sequence::new(
                if name.is_empty() {
                    format!("track {}", ti + 1)
                } else {
                    name.clone()
                },
                song.tempo_bpm,
            );
            seq.channel = ch;
            seq.program = programs[usize::from(ch)];
            seq.notes = notes;
            song.tracks.push(seq);
        }
    }
    for t in &mut song.tracks {
        t.tempo_bpm = song.tempo_bpm;
        t.key = song.key;
    }
    Ok(song)
}

pub fn read_song_file(path: &Path) -> Result<Song> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    read_song(&bytes).map_err(|e| match e {
        Error::Midi(m) => Error::Midi(format!("{}: {m}", path.display())),
        other => other,
    })
}

fn key_from_signature(sharps: i8, minor: bool) -> Option<Key> {
    // Inverse of the circle of fifths: major tonic = 7 * sharps (mod 12).
    let major_tonic = (i32::from(sharps) * 7).rem_euclid(12) as u8;
    Some(if minor {
        Key::new((major_tonic + 9) % 12, Scale::Minor)
    } else {
        Key::new(major_tonic, Scale::Major)
    })
}

/// Parses a Standard MIDI File into a single sequence. All tracks and
/// channels are merged; the first tempo, key signature and program win.
pub fn read_sequence(bytes: &[u8]) -> Result<Sequence> {
    let smf = Smf::parse(bytes).map_err(|e| Error::Midi(e.to_string()))?;
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => f64::from(t.as_int()),
        Timing::Timecode(..) => {
            return Err(Error::Midi(
                "SMPTE timecode MIDI files are not supported; re-export with musical (PPQ) timing"
                    .into(),
            ));
        }
    };
    let mut seq = Sequence::new("", 120.0);
    let mut tempo_set = false;
    for track in &smf.tracks {
        let mut tick = 0u64;
        // Open notes per (channel, key): start tick and velocity.
        let mut open: std::collections::HashMap<(u8, u8), Vec<(u64, u8)>> = Default::default();
        for ev in track {
            tick += u64::from(ev.delta.as_int());
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) if !tempo_set => {
                    seq.tempo_bpm = 60_000_000.0 / f64::from(t.as_int().max(1));
                    tempo_set = true;
                }
                TrackEventKind::Meta(MetaMessage::KeySignature(sharps, minor))
                    if seq.key.is_none() =>
                {
                    seq.key = key_from_signature(sharps, minor);
                }
                TrackEventKind::Meta(MetaMessage::TrackName(name)) if seq.name.is_empty() => {
                    seq.name = String::from_utf8_lossy(name).into_owned();
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::ProgramChange { program } if seq.program.is_none() => {
                            seq.program = Some(program.as_int());
                            seq.channel = ch;
                        }
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            open.entry((ch, key.as_int()))
                                .or_default()
                                .push((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(stack) = open.get_mut(&(ch, key.as_int())) {
                                if !stack.is_empty() {
                                    let (start, vel) = stack.remove(0);
                                    seq.notes.push(SeqNote {
                                        pitch: key.as_int(),
                                        start_beats: start as f64 / ppq,
                                        duration_beats: (tick - start) as f64 / ppq,
                                        velocity: vel,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    seq.notes.sort_by(|a, b| {
        a.start_beats
            .total_cmp(&b.start_beats)
            .then(a.pitch.cmp(&b.pitch))
    });
    Ok(seq)
}

pub fn read_file(path: &Path) -> Result<Sequence> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    read_sequence(&bytes).map_err(|e| match e {
        Error::Midi(m) => Error::Midi(format!("{}: {m}", path.display())),
        other => other,
    })
}

/// Human-readable dump of every event, used for golden-file snapshots.
pub fn dump(bytes: &[u8]) -> Result<String> {
    let smf = Smf::parse(bytes).map_err(|e| Error::Midi(e.to_string()))?;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "format={:?} timing={:?} tracks={}",
        smf.header.format,
        smf.header.timing,
        smf.tracks.len()
    );
    for (i, track) in smf.tracks.iter().enumerate() {
        let _ = writeln!(s, "track {i}");
        let mut tick = 0u64;
        for ev in track {
            tick += u64::from(ev.delta.as_int());
            let desc = match ev.kind {
                TrackEventKind::Midi { channel, message } => match message {
                    MidiMessage::NoteOn { key, vel } => format!(
                        "ch{} note_on  {:<4} vel {}",
                        channel.as_int(),
                        crate::theory::note_name(key.as_int()),
                        vel.as_int()
                    ),
                    MidiMessage::NoteOff { key, .. } => format!(
                        "ch{} note_off {}",
                        channel.as_int(),
                        crate::theory::note_name(key.as_int())
                    ),
                    other => format!("ch{} {other:?}", channel.as_int()),
                },
                TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                    format!("track_name {:?}", String::from_utf8_lossy(n))
                }
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => {
                    format!("tempo {:.2} bpm", 60_000_000.0 / f64::from(t.as_int()))
                }
                TrackEventKind::Meta(m) => format!("{m:?}"),
                other => format!("{other:?}"),
            };
            let _ = writeln!(s, "  {tick:>6}  {desc}");
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sequence {
        let mut seq = Sequence::new("lead", 96.0);
        seq.key = Some(Key::new(2, Scale::Minor));
        seq.program = Some(40);
        for (i, (p, d)) in [(62u8, 1.0), (65, 0.5), (69, 0.5), (69, 2.0)]
            .iter()
            .enumerate()
        {
            seq.notes.push(SeqNote {
                pitch: *p,
                start_beats: i as f64,
                duration_beats: *d,
                velocity: 90,
            });
        }
        seq
    }

    #[test]
    fn round_trip() {
        let seq = sample();
        let back = read_sequence(&write_sequence(&seq)).unwrap();
        assert_eq!(back.name, "lead");
        assert!((back.tempo_bpm - 96.0).abs() < 1e-3);
        assert_eq!(back.key, seq.key);
        assert_eq!(back.program, Some(40));
        assert_eq!(back.notes, seq.notes);
    }

    #[test]
    fn key_signature_inverse() {
        for tonic in 0..12 {
            for scale in [Scale::Major, Scale::Minor] {
                let k = Key::new(tonic, scale);
                assert_eq!(key_from_signature(k.sharps(), scale.is_minor()), Some(k));
            }
        }
    }

    #[test]
    fn golden_bytes() {
        let bytes = write_sequence(&sample());
        let golden =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/simple.mid");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &bytes).unwrap();
        }
        let expected =
            std::fs::read(&golden).expect("golden file missing; run with UPDATE_GOLDEN=1");
        assert_eq!(
            bytes, expected,
            "MIDI output changed; inspect and rerun with UPDATE_GOLDEN=1 if intended"
        );
        insta::assert_snapshot!(dump(&bytes).unwrap());
    }

    #[test]
    fn song_round_trip_keeps_parts_and_instruments() {
        let lead = sample().notes;
        let bass: Vec<SeqNote> = lead
            .iter()
            .map(|n| SeqNote {
                pitch: n.pitch - 24,
                ..*n
            })
            .collect();
        let bytes = write_song(
            "demo",
            90.0,
            Some(Key::new(2, Scale::Minor)),
            &[
                SongTrack {
                    name: "lead",
                    program: 40,
                    notes: &lead,
                },
                SongTrack {
                    name: "bass",
                    program: 33,
                    notes: &bass,
                },
            ],
        );
        let song = read_song(&bytes).unwrap();
        assert!((song.tempo_bpm - 90.0).abs() < 1e-3);
        assert_eq!(song.key, Some(Key::new(2, Scale::Minor)));
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(song.tracks[0].name, "lead");
        assert_eq!(song.tracks[0].program, Some(40));
        assert_eq!(song.tracks[1].program, Some(33));
        assert_eq!(song.tracks[1].notes, bass);
        assert_ne!(song.tracks[0].channel, song.tracks[1].channel);
        // Merged reading still works on multi-track files.
        assert_eq!(read_sequence(&bytes).unwrap().notes.len(), 8);
        insta::assert_snapshot!(dump(&bytes).unwrap());
    }

    #[test]
    fn channels_skip_drums() {
        let chans: Vec<u8> = (0..16).map(channel_for).collect();
        assert!(!chans[..15].contains(&9));
        assert_eq!(chans[8], 8);
        assert_eq!(chans[9], 10);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(read_sequence(b"MThd nope"), Err(Error::Midi(_))));
    }
}
