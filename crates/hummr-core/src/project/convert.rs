//! "Turn into": converting a recorded clip into notes, an instrument made
//! of its own sound, or drums, and back. The source region is always
//! kept, so every conversion can be undone with "Turn into: none" and
//! redone with different settings.

use super::render::{cleaned_source, region};
use super::{Clip, ClipKind, Note, Song, Voice};
use crate::drums::classify;
use crate::onset::detect_onsets;
use crate::theory::{Key, note_name};
use crate::transcribe::{KeyChoice, TempoChoice, TranscribeOptions, transcribe};
use crate::voice::root_pitch;
use crate::{AudioBuffer, Error, Result};

/// What a clip can be turned into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The melody as notes, played by an instrument.
    Notes,
    /// An instrument that plays the clip's own sound at any pitch.
    OwnSound,
    /// Each hit replaced by a kick, snare or hat.
    Drums,
    /// Back to the recorded sound.
    None,
}

impl Target {
    pub const ALL: [Target; 4] = [Target::Notes, Target::OwnSound, Target::Drums, Target::None];

    pub fn label(self) -> &'static str {
        match self {
            Target::Notes => "notes",
            Target::OwnSound => "own sound",
            Target::Drums => "drums",
            Target::None => "none",
        }
    }

    /// What the clip currently is.
    pub fn of(clip: &Clip) -> Target {
        match &clip.kind {
            ClipKind::Notes {
                voice: Voice::Sampler { .. },
                ..
            } => Target::OwnSound,
            ClipKind::Notes {
                voice: Voice::Drums,
                ..
            } => Target::Drums,
            ClipKind::Notes { .. } => Target::Notes,
            _ => Target::None,
        }
    }
}

/// The result of a conversion.
#[derive(Clone, Debug)]
pub struct Converted {
    pub clip: Clip,
    /// One plain sentence about what happened.
    pub summary: String,
    /// The key the notes were found in, when the song has none yet.
    pub key: Option<Key>,
}

/// Beats per second of source audio: how the clip maps its region onto
/// the timeline (follows speed and tempo-locking).
fn beats_per_second(clip: &Clip, bpm: f64) -> f64 {
    clip.unit_beats(bpm) / clip.source_len.max(1e-6)
}

/// The clip's region as audio (noise-cleaned if the clip asks for it).
fn clip_audio(clip: &Clip, source: &AudioBuffer) -> AudioBuffer {
    let cleaned = cleaned_source(source, clip);
    let a = cleaned.as_ref().unwrap_or(source);
    AudioBuffer::new(region(a, clip.source_start, clip.source_len), a.sample_rate)
}

fn notes_clip(clip: &Clip, bpm: f64, voice: Voice, notes: Vec<Note>) -> Clip {
    let mut c = clip.clone();
    c.kind = ClipKind::Notes {
        length_beats: clip.unit_beats(bpm),
        voice,
        notes,
    };
    // The beats already include speed and tempo-locking.
    c.speed = 1.0;
    c.stretch = super::Stretch::Keep;
    c
}

/// Converts `clip` (whose sound lives in `source`) for a song.
pub fn convert(
    song: &Song,
    clip: &Clip,
    source: &AudioBuffer,
    target: Target,
) -> Result<Converted> {
    match target {
        Target::Notes => to_notes(song, clip, source),
        Target::OwnSound => to_own_sound(song, clip, source),
        Target::Drums => to_drums(song, clip, source),
        Target::None => {
            let mut c = clip.clone();
            if let ClipKind::Notes { .. } = c.kind {
                c.kind = ClipKind::Audio;
            }
            Ok(Converted {
                clip: c,
                summary: "Back to the recorded sound.".into(),
                key: None,
            })
        }
    }
}

fn to_notes(song: &Song, clip: &Clip, source: &AudioBuffer) -> Result<Converted> {
    let audio = clip_audio(clip, source);
    let bps = beats_per_second(clip, song.bpm);
    let opts = TranscribeOptions {
        key: song.key.map_or(KeyChoice::Auto, KeyChoice::Fixed),
        tempo: TempoChoice::Fixed((bps * 60.0).clamp(20.0, 400.0)),
        grid_origin: Some(0.0),
        ..TranscribeOptions::default()
    };
    let t = transcribe(&audio, &opts)?;
    if t.notes.is_empty() {
        return Err(Error::Project(
            "No notes found in this clip. Hum or sing each note clearly, or try \"Turn into: drums\" for beatbox."
                .into(),
        ));
    }
    let notes: Vec<Note> = t
        .notes
        .iter()
        .map(|n| Note {
            pitch: n.pitch,
            beat: n.start_beats,
            len: n.duration_beats,
            vel: n.velocity,
        })
        .collect();
    let program = match &clip.kind {
        ClipKind::Notes {
            voice: Voice::Instrument { program },
            ..
        } => *program,
        _ => 0,
    };
    let summary = format!(
        "Found {} note{}{}; playing them on {}.",
        notes.len(),
        if notes.len() == 1 { "" } else { "s" },
        t.key.map(|k| format!(" in {k}")).unwrap_or_default(),
        crate::instruments::name(program)
    );
    Ok(Converted {
        clip: notes_clip(clip, song.bpm, Voice::Instrument { program }, notes),
        summary,
        key: if song.key.is_none() { t.key } else { None },
    })
}

fn to_own_sound(song: &Song, clip: &Clip, source: &AudioBuffer) -> Result<Converted> {
    let audio = clip_audio(clip, source);
    let root = root_pitch(&audio.samples, audio.sample_rate).ok_or_else(|| {
        Error::Project(
            "This clip has no clear pitch to play notes with. Record one steady hummed or sung note, or try \"Turn into: drums\"."
                .into(),
        )
    })?;
    let pitch = root.round().clamp(0.0, 127.0) as u8;
    let beats = clip.unit_beats(song.bpm);
    let notes = vec![Note {
        pitch,
        beat: 0.0,
        len: beats,
        vel: 100,
    }];
    Ok(Converted {
        clip: notes_clip(clip, song.bpm, Voice::Sampler { root }, notes),
        summary: format!(
            "Your sound is now an instrument (it's a {}); open the notes editor with e to play other notes.",
            note_name(pitch)
        ),
        key: None,
    })
}

fn to_drums(song: &Song, clip: &Clip, source: &AudioBuffer) -> Result<Converted> {
    let cleaned = cleaned_source(source, clip);
    let src = cleaned.as_ref().unwrap_or(source);
    let sr = src.sample_rate;
    // (beat, attack time in the source) for every hit.
    let hits: Vec<(f64, f64)> = match &clip.kind {
        // Snapped slices already know where each hit plays.
        ClipKind::Sliced { slices, .. } => slices
            .iter()
            .filter_map(|s| {
                s.hit
                    .map(|h| (s.beat + h / song.seconds_per_beat(), s.src + h))
            })
            .collect(),
        _ => {
            let r = region(src, clip.source_start, clip.source_len);
            let bps = beats_per_second(clip, song.bpm);
            detect_onsets(&r, sr)
                .iter()
                .map(|o| (o.time * bps, clip.source_start + o.time))
                .collect()
        }
    };
    if hits.is_empty() {
        return Err(Error::Project(
            "No hits found in this clip. Beatbox or tap with short, clear sounds, then try again."
                .into(),
        ));
    }
    let peaks: Vec<f32> = hits
        .iter()
        .map(|&(_, t)| {
            region(src, t, 0.03)
                .iter()
                .fold(0.0f32, |m, s| m.max(s.abs()))
        })
        .collect();
    let loudest = peaks.iter().copied().fold(1e-9f32, f32::max);
    let mut counts = [0usize; 3];
    let notes: Vec<Note> = hits
        .iter()
        .zip(&peaks)
        .map(|(&(beat, t), &p)| {
            let class = classify(&region(src, t, 0.06), sr);
            counts[class as usize] += 1;
            Note {
                pitch: class.gm_note(),
                beat: beat.max(0.0),
                len: 0.25,
                vel: (40.0 + 87.0 * (p / loudest)).round().clamp(1.0, 127.0) as u8,
            }
        })
        .collect();
    let summary = format!(
        "Replaced {} hits with drums: {} kick, {} snare, {} hat.",
        notes.len(),
        counts[0],
        counts[1],
        counts[2]
    );
    Ok(Converted {
        clip: notes_clip(clip, song.bpm, Voice::Drums, notes),
        summary,
        key: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::render::render_clip;
    use crate::project::{PROJECT_RATE, SourceBank};
    use crate::synthetic::{Drum, HumParams, TWINKLE, Tone, beatbox, hum, melody};
    use std::sync::Arc;

    fn setup(samples: Vec<f32>) -> (Song, AudioBuffer, SourceBank, Clip) {
        let a = AudioBuffer::new(samples, PROJECT_RATE);
        let mut bank = SourceBank::default();
        bank.insert("s", Arc::new(a.clone()));
        let mut song = Song::new("t");
        song.bpm = 90.0;
        let clip = Clip::audio(1, "take", "s", 0.0, a.duration_seconds(), 4.0);
        (song, a, bank, clip)
    }

    #[test]
    fn hummed_melody_turns_into_notes_and_back() {
        let tones = melody(&TWINKLE, 90.0, 0.2, 0.0, 1);
        let (song, a, bank, clip) = setup(hum(&tones, &HumParams::new(PROJECT_RATE), 1));
        let c = convert(&song, &clip, &a, Target::Notes).unwrap();
        assert_eq!(Target::of(&c.clip), Target::Notes);
        let ClipKind::Notes { notes, .. } = &c.clip.kind else {
            panic!()
        };
        let want: Vec<u8> = tones.iter().map(|t| t.midi.round() as u8).collect();
        let got: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
        assert_eq!(got, want, "{}", c.summary);
        // Linked: same place, same length, same source region.
        assert_eq!(c.clip.start_beat, clip.start_beat);
        assert!((c.clip.len_beats(90.0) - clip.len_beats(90.0)).abs() < 1e-9);
        assert_eq!(c.clip.source_start, clip.source_start);
        assert!(!render_clip(&song, &c.clip, &bank).is_empty());
        let back = convert(&song, &c.clip, &a, Target::None).unwrap();
        assert_eq!(back.clip.kind, ClipKind::Audio);
    }

    #[test]
    fn a_sung_note_becomes_an_instrument_of_itself() {
        let (song, a, bank, clip) = setup(hum(
            &[Tone::new(55.0, 0.1, 1.2)],
            &HumParams::new(PROJECT_RATE),
            2,
        ));
        let c = convert(&song, &clip, &a, Target::OwnSound).unwrap();
        let ClipKind::Notes {
            voice: Voice::Sampler { root },
            notes,
            ..
        } = &c.clip.kind
        else {
            panic!()
        };
        assert!((root - 55.0).abs() < 0.25, "{root}");
        assert_eq!(notes[0].pitch, 55);
        // Played a fifth up, it comes out a fifth up.
        let mut up = c.clip.clone();
        if let ClipKind::Notes { notes, .. } = &mut up.kind {
            notes[0].pitch = 62;
        }
        let y = render_clip(&song, &up, &bank);
        let got = root_pitch(&y, PROJECT_RATE).unwrap();
        assert!((got - root - 7.0).abs() < 0.25, "{got}");
        // Noise has no pitch.
        let (song, a, _, clip) = setup(crate::synthetic::white_noise(48_000, 0.2, 3));
        let e = convert(&song, &clip, &a, Target::OwnSound)
            .unwrap_err()
            .to_string();
        assert!(e.contains("no clear pitch"), "{e}");
    }

    #[test]
    fn beatbox_turns_into_the_right_drums() {
        let pattern: Vec<(f64, Drum)> = (0..16)
            .map(|i| {
                let d = match i % 4 {
                    0 => Drum::Kick,
                    2 => Drum::Snare,
                    _ => Drum::Hat,
                };
                (0.3 + i as f64 * 60.0 / 90.0 / 2.0, d)
            })
            .collect();
        let (song, a, bank, clip) = setup(beatbox(&pattern, 4, PROJECT_RATE));
        let c = convert(&song, &clip, &a, Target::Drums).unwrap();
        let ClipKind::Notes { notes, .. } = &c.clip.kind else {
            panic!()
        };
        assert_eq!(notes.len(), pattern.len(), "{}", c.summary);
        let mut right = 0;
        for (n, (t, d)) in notes.iter().zip(&pattern) {
            let beat = t * 90.0 / 60.0;
            assert!((n.beat - beat).abs() < 0.01, "{} vs {beat}", n.beat);
            let want = match d {
                Drum::Kick => 36,
                Drum::Snare => 38,
                Drum::Hat => 42,
            };
            right += usize::from(n.pitch == want);
        }
        assert!(right * 10 >= notes.len() * 9, "{right}/{}", notes.len());
        // The replacement plays a hit at every note.
        let y = render_clip(&song, &c.clip, &bank);
        let found = detect_onsets(&y, PROJECT_RATE);
        assert_eq!(found.len(), notes.len());
        assert_eq!(Target::of(&c.clip), Target::Drums);
    }

    #[test]
    fn silence_gives_a_plain_error() {
        let (song, a, _, clip) = setup(vec![0.0; 48_000]);
        for t in [Target::Notes, Target::Drums, Target::OwnSound] {
            let e = convert(&song, &clip, &a, t).unwrap_err().to_string();
            assert!(e.contains("No ") || e.contains("no "), "{e}");
        }
    }
}
