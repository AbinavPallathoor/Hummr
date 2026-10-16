//! Exporting a song: a limited stereo WAV mix, or the notes clips as MIDI.

use std::path::Path;

use super::render::{MixOptions, mixdown};
use super::{ClipKind, Song, SourceBank, Voice};
use crate::io::{midi, wav};
use crate::sequence::SeqNote;
use crate::{Error, Result};

/// What an export wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub seconds: f64,
    /// Notes written (MIDI only).
    pub notes: usize,
    /// Number of parts/tracks written.
    pub parts: usize,
}

/// Writes the limited stereo mix as 16-bit WAV at the project rate.
pub fn write_wav(song: &Song, bank: &SourceBank, path: &Path) -> Result<Exported> {
    if song.tracks.iter().all(|t| t.clips.is_empty()) {
        return Err(Error::Project(
            "the song is empty: record or import something first".into(),
        ));
    }
    let mix = mixdown(song, bank, MixOptions::default());
    wav::write(
        path,
        &mix.interleaved(),
        2,
        song.sample_rate,
        wav::WavFormat::Pcm16,
    )?;
    Ok(Exported {
        seconds: mix.len() as f64 / f64::from(song.sample_rate),
        notes: 0,
        parts: song.tracks.len(),
    })
}

/// One MIDI part per track that has notes clips: (name, program, drums,
/// notes in song beats).
pub fn midi_parts(song: &Song) -> Vec<(String, u8, bool, Vec<SeqNote>)> {
    let mut parts = Vec::new();
    for t in &song.tracks {
        let mut notes = Vec::new();
        let mut program = None;
        let mut drums = false;
        for c in &t.clips {
            let ClipKind::Notes {
                notes: ns, voice, ..
            } = &c.kind
            else {
                continue;
            };
            match voice {
                Voice::Instrument { program: p } => {
                    program.get_or_insert(*p);
                }
                Voice::Drums => drums = true,
                Voice::Sampler { .. } => {}
            }
            let unit = c.unit_beats(song.bpm);
            for k in 0..c.repeat.max(1) {
                let base = c.start_beat + unit * f64::from(k);
                notes.extend(ns.iter().map(|n| SeqNote {
                    pitch: n.pitch,
                    start_beats: base + n.beat,
                    duration_beats: n.len,
                    velocity: n.vel,
                }));
            }
        }
        if !notes.is_empty() {
            notes.sort_by(|a, b| a.start_beats.total_cmp(&b.start_beats));
            parts.push((t.name.clone(), program.unwrap_or(0), drums, notes));
        }
    }
    parts
}

/// Writes every notes clip as a multi-track MIDI file.
pub fn write_midi(song: &Song, path: &Path) -> Result<Exported> {
    let parts = midi_parts(song);
    if parts.is_empty() {
        return Err(Error::Project(
            "no notes to export: turn a clip into notes first (Enter on a clip, then 'Turn into'), \
             or export a .wav instead"
                .into(),
        ));
    }
    let tracks: Vec<midi::SongTrack<'_>> = parts
        .iter()
        .map(|(name, program, drums, notes)| midi::SongTrack {
            name,
            program: *program,
            notes,
            drums: *drums,
        })
        .collect();
    midi::write_song_file(path, &song.name, song.bpm, song.key, &tracks)?;
    Ok(Exported {
        seconds: song.beats_to_seconds(song.end_beats()),
        notes: parts.iter().map(|p| p.3.len()).sum(),
        parts: parts.len(),
    })
}

/// Exports by extension: `.wav` or `.mid`/`.midi`.
pub fn export(song: &Song, bank: &SourceBank, path: &Path) -> Result<Exported> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "wav" | "wave" => write_wav(song, bank, path),
        "mid" | "midi" => write_midi(song, path),
        _ => Err(Error::Project(format!(
            "can't export to {}: use a .wav (audio mix) or .mid (notes) file name",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioBuffer;
    use crate::project::{Clip, Note, PROJECT_RATE};
    use std::sync::Arc;

    #[test]
    fn exports_wav_and_midi() {
        let dir = std::env::temp_dir().join(format!("hummr-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bank = SourceBank::default();
        bank.insert(
            "s",
            Arc::new(AudioBuffer::new(vec![0.2; 48_000], PROJECT_RATE)),
        );
        let mut song = Song::new("x");
        song.bpm = 120.0;
        let t = song.add_track("Lead");
        let mut c = Clip::audio(0, "n", "s", 0.0, 1.0, 2.0);
        c.kind = ClipKind::Notes {
            length_beats: 2.0,
            voice: Voice::Instrument { program: 40 },
            notes: vec![Note {
                pitch: 60,
                beat: 0.5,
                len: 1.0,
                vel: 100,
            }],
        };
        c.repeat = 2;
        song.insert_clip(t, c);
        let a = song.add_track("Audio");
        song.insert_clip(a, Clip::audio(0, "a", "s", 0.0, 1.0, 0.0));

        let w = export(&song, &bank, &dir.join("song.wav")).unwrap();
        assert!(w.seconds >= 3.0, "{w:?}");
        let back = wav::read_mono(&dir.join("song.wav")).unwrap();
        assert_eq!(back.sample_rate, PROJECT_RATE);

        let m = export(&song, &bank, &dir.join("song.mid")).unwrap();
        assert_eq!(m.notes, 2);
        let read = midi::read_song_file(&dir.join("song.mid")).unwrap();
        assert_eq!(read.tracks.len(), 1);
        assert_eq!(read.tracks[0].program, Some(40));
        let starts: Vec<f64> = read.tracks[0].notes.iter().map(|n| n.start_beats).collect();
        assert_eq!(starts, vec![2.5, 4.5]);
        assert!(export(&song, &bank, &dir.join("song.mp3")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
