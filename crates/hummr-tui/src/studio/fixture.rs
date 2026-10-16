//! A small, fully synthetic song shaped like the timeline mock-up in the
//! design brief: a beatbox loop, a bass line, a hummed lead with an
//! overlapping second take, and a couple of samples. Used by snapshot tests
//! and the screenshot example.

use hummr_core::AudioBuffer;
use hummr_core::kit::{Sound, drum_hit};
use hummr_core::project::{Clip, ClipKind, Note, PROJECT_RATE, Project, Voice};
use hummr_core::synthetic::{HumParams, TWINKLE, Tone, drum_groove, hum, melody};

fn notes(clip: &mut Clip, voice: Voice, notes: Vec<Note>, beats: f64) {
    clip.kind = ClipKind::Notes {
        length_beats: beats,
        voice,
        notes,
    };
}

pub fn mockup_project() -> Project {
    let sr = PROJECT_RATE;
    let mut p = Project::new("Beatbox Song");
    p.song.bpm = 96.0;
    p.song.key = "A minor".parse().ok();
    let spb = 60.0 / 96.0;

    let (beat, _) = drum_groove(96.0, 2, 0, 0.0, 0.0, 1, sr);
    let beat_id = p.add_audio("beatbox", AudioBuffer::new(beat, sr), Some(96.0));
    let hp = HumParams::new(sr);
    let mmm = hum(
        &[
            Tone::new(45.0, 0.0, 1.2),
            Tone::new(48.0, 1.25, 1.2),
            Tone::new(43.0, 2.5, 2.4),
        ],
        &hp,
        2,
    );
    let mmm_id = p.add_audio("mmm", AudioBuffer::new(mmm, sr), Some(96.0));
    let lead = hum(&melody(&TWINKLE, 96.0, 0.0, 0.0, 3), &hp, 3);
    let lead_id = p.add_audio("hum", AudioBuffer::new(lead.clone(), sr), Some(96.0));
    let take2 = hum(&melody(&TWINKLE[7..], 96.0, 0.0, 0.0, 4), &hp, 4);
    let take2_id = p.add_audio("hum take 2", AudioBuffer::new(take2, sr), Some(96.0));
    let chop = hum(
        &[
            Tone::new(64.0, 0.05, 0.3),
            Tone::new(67.0, 0.45, 0.3),
            Tone::new(62.0, 0.9, 0.6),
        ],
        &hp,
        5,
    );
    let chop_id = p.add_audio("vocal chop", AudioBuffer::new(chop, sr), None);
    let slam_id = p.add_audio(
        "slam",
        AudioBuffer::new(drum_hit(Sound::Kick, 1.0, sr, 1), sr),
        None,
    );

    let t = p.song.add_track("Beat");
    let a = Clip::audio(0, "beatbox", &beat_id, 0.0, 8.0 * spb, 0.0);
    let mut b = a.clone();
    b.name = "beatbox (copy)".into();
    b.start_beat = 8.0;
    p.song.insert_clip(t, a);
    p.song.insert_clip(t, b);

    let t = p.song.add_track("Bass");
    let bass_notes: Vec<Note> = [(45, 0.0, 2.0), (48, 2.0, 2.0), (43, 4.0, 4.0)]
        .iter()
        .map(|&(pitch, beat, len)| Note {
            pitch,
            beat,
            len,
            vel: 100,
        })
        .collect();
    let mut c = Clip::audio(0, "mmm", &mmm_id, 0.0, 8.0 * spb, 0.0);
    notes(
        &mut c,
        Voice::Instrument { program: 33 },
        bass_notes.clone(),
        8.0,
    );
    let mut d = c.clone();
    d.start_beat = 10.0;
    notes(
        &mut d,
        Voice::Instrument { program: 33 },
        bass_notes[..2].to_vec(),
        6.0,
    );
    p.song.insert_clip(t, c);
    p.song.insert_clip(t, d);

    let t = p.song.add_track("Lead");
    let lead_notes: Vec<Note> = TWINKLE
        .iter()
        .scan(0.0, |at, &(pitch, len)| {
            let n = Note {
                pitch,
                beat: *at,
                len: len * 0.9,
                vel: 96,
            };
            *at += len;
            Some(n)
        })
        .take(8)
        .collect();
    let mut e = Clip::audio(0, "hum", &lead_id, 0.0, 8.0 * spb, 4.0);
    notes(&mut e, Voice::Instrument { program: 0 }, lead_notes, 8.0);
    p.song.insert_clip(t, e);
    p.song.insert_clip(
        t,
        Clip::audio(0, "hum take 2", &take2_id, 0.0, 6.0 * spb, 10.0),
    );

    let t = p.song.add_track("Sample");
    p.song
        .insert_clip(t, Clip::audio(0, "vocal chop", &chop_id, 0.0, 1.5, 2.0));
    p.song
        .insert_clip(t, Clip::audio(0, "slam", &slam_id, 0.0, 0.45, 13.0));
    p
}
