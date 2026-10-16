//! The song model: tracks of clips placed on a beat grid.
//!
//! Editing is non-destructive. Recordings and imported samples are
//! immutable *sources* (shared buffers, see [`SourceBank`]); a [`Clip`] is a
//! view of a region of a source plus settings (gain, pitch, effects, how it
//! is cut into slices or turned into notes). A [`Song`] holds no audio, so
//! cloning it for undo is cheap.
//!
//! Positions on the timeline are in beats (quarter notes). Lengths inside
//! sources are in seconds.

pub mod convert;
pub mod export;
pub mod file;
pub mod history;
pub mod ops;
pub mod render;

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::fx::FxChain;
use crate::rhythm::Grid;
use crate::theory::Key;
use crate::{AudioBuffer, Error, Result};

pub use file::Project;
pub use history::History;

/// Sample rate every source is converted to and every render runs at.
pub const PROJECT_RATE: u32 = 48_000;

/// Current project file format.
pub const FORMAT: u32 = 1;

const EPS: f64 = 1e-9;

/// A time signature. A "beat" is always a quarter note, so a bar of 6/8
/// lasts three beats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSig {
    pub top: u8,
    pub bottom: u8,
}

impl TimeSig {
    pub const FOUR_FOUR: TimeSig = TimeSig { top: 4, bottom: 4 };
    pub const THREE_FOUR: TimeSig = TimeSig { top: 3, bottom: 4 };

    /// Beats (quarter notes) per bar.
    pub fn bar_beats(&self) -> f64 {
        f64::from(self.top) * 4.0 / f64::from(self.bottom)
    }
}

impl Default for TimeSig {
    fn default() -> Self {
        Self::FOUR_FOUR
    }
}

impl fmt::Display for TimeSig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.top, self.bottom)
    }
}

impl FromStr for TimeSig {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let hint = "expected a time signature like 4/4, 3/4 or 6/8";
        let (a, b) = s
            .trim()
            .split_once('/')
            .ok_or_else(|| Error::parse("time signature", s, hint))?;
        let top: u8 = a
            .trim()
            .parse()
            .map_err(|_| Error::parse("time signature", s, hint))?;
        let bottom: u8 = b
            .trim()
            .parse()
            .map_err(|_| Error::parse("time signature", s, hint))?;
        if !(1..=16).contains(&top) || !matches!(bottom, 2 | 4 | 8 | 16) {
            return Err(Error::parse("time signature", s, hint));
        }
        Ok(TimeSig { top, bottom })
    }
}

/// Serde helpers that store a type through its `Display`/`FromStr` forms,
/// so the project file says `key = "A minor"` rather than a nested table.
mod as_string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<T: Display, S: Serializer>(v: &T, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let s = String::deserialize(d)?;
        s.parse().map_err(de::Error::custom)
    }
}

/// Like [`as_string`] for optional values, written as `"none"` when absent.
mod opt_string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<T: Display, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.collect_str(v),
            None => s.serialize_str("none"),
        }
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let s = String::deserialize(d)?;
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "" => Ok(None),
            _ => s.parse().map(Some).map_err(de::Error::custom),
        }
    }
}

fn default_snap() -> Option<Grid> {
    Some(Grid {
        numerator: 1,
        denominator: 4,
        triplet: false,
    })
}

fn default_rate() -> u32 {
    PROJECT_RATE
}

fn one() -> f64 {
    1.0
}

fn one_u32() -> u32 {
    1
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

fn is_one(x: &f64) -> bool {
    *x == 1.0
}

fn is_one_u32(x: &u32) -> bool {
    *x == 1
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Where an imported or recorded sound lives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub id: String,
    /// Path relative to the project file, e.g. `sources/s1-beatbox.wav`.
    pub file: String,
    pub name: String,
    /// Tempo of the count-in when this was recorded inside Hummr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_bpm: Option<f64>,
}

/// The whole song. Contains no audio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub format: u32,
    pub name: String,
    pub bpm: f64,
    #[serde(rename = "time_signature", with = "as_string", default)]
    pub time_sig: TimeSig,
    #[serde(with = "opt_string", default)]
    pub key: Option<Key>,
    #[serde(with = "opt_string", default = "default_snap")]
    pub snap: Option<Grid>,
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Next free id for clips, tracks and sources.
    #[serde(default)]
    pub next_id: u64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub master_fx: FxChain,
    #[serde(default)]
    pub sources: Vec<SourceInfo>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// One row of the timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub volume_db: f64,
    #[serde(default)]
    pub pan: f64,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fx: FxChain,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// How a whole-audio clip reacts when the song tempo changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Stretch {
    /// Keep the natural sound; only the start moves with the beat grid.
    #[default]
    Keep,
    /// Stretch to stay the same number of beats long. `bpm` is the song
    /// tempo the clip was locked at.
    Follow { bpm: f64 },
}

/// A piece of a source played at a beat position inside a sliced clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slice {
    /// Start inside the source (seconds).
    pub src: f64,
    /// Natural length (seconds).
    pub len: f64,
    /// Where it plays, in beats from the clip start.
    pub beat: f64,
    /// Seconds from `src` to the attack of the hit in this slice (none for
    /// a lead-in before the first hit). Snapping lines the attack up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<f64>,
    /// Where the slice was before snapping (beats from the clip start), so
    /// the snap strength can be changed later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural: Option<f64>,
}

impl Slice {
    pub fn new(src: f64, len: f64, beat: f64) -> Self {
        Self {
            src,
            len,
            beat,
            hit: None,
            natural: None,
        }
    }
}

/// A note in a notes clip, in beats from the clip start.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub beat: f64,
    pub len: f64,
    pub vel: u8,
}

/// What plays the notes of a notes clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Voice {
    /// A General MIDI instrument (preview synth, or a SoundFont if set up).
    Instrument { program: u8 },
    /// The clip's own sound, pitched to each note. `root` is the detected
    /// pitch of the sound (fractional MIDI note).
    Sampler { root: f64 },
    /// A drum kit; pitches follow General MIDI drums (36 kick, 38 snare,
    /// 42 closed hat, 46 open hat, 39 clap).
    Drums,
}

/// What a clip plays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClipKind {
    /// The source region as recorded.
    #[default]
    Audio,
    /// The region cut at its hits, each slice anchored to a beat.
    Sliced {
        length_beats: f64,
        slices: Vec<Slice>,
        /// Snap grid in beats (0: no snapping).
        #[serde(default)]
        grid: f64,
        /// How far hits move towards the grid, 0..=1.
        #[serde(default)]
        strength: f64,
    },
    /// Notes rendered by a voice. The source region is kept so the clip can
    /// be re-transcribed.
    Notes {
        length_beats: f64,
        voice: Voice,
        notes: Vec<Note>,
    },
}

/// A clip: a region of a source placed on the timeline, plus settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    pub source: String,
    /// Region start inside the source (seconds).
    pub source_start: f64,
    /// Region length (seconds).
    pub source_len: f64,
    pub start_beat: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gain_db: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pan: f64,
    /// Pitch shift in semitones (length unchanged).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pitch: f64,
    /// Playback speed (pitch unchanged): 2 plays twice as fast.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub speed: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_out: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// How many times the clip plays back to back.
    #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
    pub repeat: u32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub stretch: Stretch,
    /// Noise cleanup strength (0 = off, 1 = strongest); see `denoise`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub clean: f64,
    /// Where the background noise was learned from (source seconds:
    /// start, length); `None` = the first 300 ms of the clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_region: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fx: FxChain,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: ClipKind,
}

impl Clip {
    /// A plain audio clip of a whole source region.
    pub fn audio(id: u64, name: &str, source: &str, start: f64, len: f64, at_beat: f64) -> Self {
        Self {
            id,
            name: name.to_string(),
            source: source.to_string(),
            source_start: start,
            source_len: len,
            start_beat: at_beat,
            gain_db: 0.0,
            pan: 0.0,
            pitch: 0.0,
            speed: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            reverse: false,
            repeat: 1,
            stretch: Stretch::Keep,
            clean: 0.0,
            noise_region: None,
            fx: FxChain::default(),
            kind: ClipKind::Audio,
        }
    }

    /// Length of one repeat, in beats, at song tempo `bpm`.
    pub fn unit_beats(&self, bpm: f64) -> f64 {
        match &self.kind {
            ClipKind::Audio => {
                let secs = self.source_len / self.speed.max(1e-3);
                match self.stretch {
                    Stretch::Keep => secs * bpm / 60.0,
                    Stretch::Follow { bpm: locked } => secs * locked / 60.0,
                }
            }
            ClipKind::Sliced { length_beats, .. } | ClipKind::Notes { length_beats, .. } => {
                *length_beats
            }
        }
    }

    /// Length on the timeline, in beats.
    pub fn len_beats(&self, bpm: f64) -> f64 {
        self.unit_beats(bpm) * f64::from(self.repeat.max(1))
    }

    pub fn end_beat(&self, bpm: f64) -> f64 {
        self.start_beat + self.len_beats(bpm)
    }

    /// A short label for what the clip has been turned into.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            ClipKind::Audio => "audio",
            ClipKind::Sliced { .. } => "sliced",
            ClipKind::Notes { voice, .. } => match voice {
                Voice::Instrument { .. } => "notes",
                Voice::Sampler { .. } => "own sound",
                Voice::Drums => "drums",
            },
        }
    }
}

/// Decoded audio for every source, at [`PROJECT_RATE`]. Sources are only ever
/// added, so undo never needs to touch this.
#[derive(Clone, Debug, Default)]
pub struct SourceBank {
    audio: HashMap<String, Arc<AudioBuffer>>,
}

impl SourceBank {
    pub fn get(&self, id: &str) -> Option<&Arc<AudioBuffer>> {
        self.audio.get(id)
    }

    pub fn insert(&mut self, id: impl Into<String>, audio: Arc<AudioBuffer>) {
        self.audio.insert(id.into(), audio);
    }

    pub fn contains(&self, id: &str) -> bool {
        self.audio.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.audio.len()
    }

    pub fn is_empty(&self) -> bool {
        self.audio.is_empty()
    }
}

impl Song {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: FORMAT,
            name: name.into(),
            bpm: 100.0,
            time_sig: TimeSig::FOUR_FOUR,
            key: None,
            snap: default_snap(),
            sample_rate: PROJECT_RATE,
            next_id: 1,
            master_fx: FxChain::default(),
            sources: Vec::new(),
            tracks: Vec::new(),
        }
    }

    pub fn alloc_id(&mut self) -> u64 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn seconds_per_beat(&self) -> f64 {
        60.0 / self.bpm
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        beats * self.seconds_per_beat()
    }

    pub fn seconds_to_beats(&self, secs: f64) -> f64 {
        secs / self.seconds_per_beat()
    }

    pub fn bar_beats(&self) -> f64 {
        self.time_sig.bar_beats()
    }

    /// Snap step in beats (one beat when snapping is off, for movement).
    pub fn snap_beats(&self) -> f64 {
        self.snap.map_or(1.0, |g| g.beats())
    }

    /// Rounds a beat position to the snap grid (unchanged when snap is off).
    pub fn snap_beat(&self, beat: f64) -> f64 {
        match self.snap {
            Some(g) => {
                let s = g.beats();
                (beat / s).round() * s
            }
            None => beat,
        }
    }

    pub fn add_track(&mut self, name: impl Into<String>) -> usize {
        let id = self.alloc_id();
        self.tracks.push(Track {
            id,
            name: name.into(),
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            fx: FxChain::default(),
            clips: Vec::new(),
        });
        self.tracks.len() - 1
    }

    pub fn source(&self, id: &str) -> Option<&SourceInfo> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// End of the last clip, in beats.
    pub fn end_beats(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.end_beat(self.bpm))
            .fold(0.0, f64::max)
    }

    /// (track index, clip index) of the clip with `id`.
    pub fn find_clip(&self, id: u64) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(t, tr)| tr.clips.iter().position(|c| c.id == id).map(|i| (t, i)))
    }

    pub fn clip(&self, id: u64) -> Option<&Clip> {
        self.find_clip(id).map(|(t, i)| &self.tracks[t].clips[i])
    }

    pub fn clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.find_clip(id)
            .map(|(t, i)| &mut self.tracks[t].clips[i])
    }

    /// Whether a track is heard, honouring mute and solo.
    pub fn audible(&self, track: usize) -> bool {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        self.tracks
            .get(track)
            .is_some_and(|t| !t.mute && (!any_solo || t.solo))
    }

    /// Overlap lane of each clip on a track (same order as `clips`), by
    /// greedy interval packing: clips sorted by start each take the first
    /// lane whose last clip has ended.
    pub fn lanes(&self, track: usize) -> Vec<usize> {
        let Some(t) = self.tracks.get(track) else {
            return Vec::new();
        };
        let spans: Vec<(f64, f64)> = t
            .clips
            .iter()
            .map(|c| (c.start_beat, c.end_beat(self.bpm)))
            .collect();
        pack_lanes(&spans)
    }

    /// Number of lanes a track needs (at least 1).
    pub fn lane_count(&self, track: usize) -> usize {
        self.lanes(track).into_iter().max().map_or(1, |m| m + 1)
    }

    /// Moves a clip to `track` at `start_beat` (clamped at 0). Returns false
    /// if either does not exist.
    pub fn move_clip(&mut self, id: u64, track: usize, start_beat: f64) -> bool {
        let Some((t, i)) = self.find_clip(id) else {
            return false;
        };
        if track >= self.tracks.len() {
            return false;
        }
        let mut clip = self.tracks[t].clips.remove(i);
        clip.start_beat = start_beat.max(0.0);
        let dest = &mut self.tracks[track].clips;
        dest.push(clip);
        sort_clips(dest);
        true
    }

    /// Splits a clip at song beat `at`. Returns the ids of the left and
    /// right parts, or `None` if `at` is not strictly inside the clip.
    pub fn split_clip(&mut self, id: u64, at: f64) -> Option<(u64, u64)> {
        let (t, i) = self.find_clip(id)?;
        let bpm = self.bpm;
        let sr = f64::from(self.sample_rate);
        // A repeated clip is first unrolled into one clip per repeat.
        if self.tracks[t].clips[i].repeat > 1 {
            let clip = self.tracks[t].clips.remove(i);
            let unit = clip.unit_beats(bpm);
            for k in 0..clip.repeat {
                let mut c = clip.clone();
                c.repeat = 1;
                c.start_beat = clip.start_beat + unit * f64::from(k);
                if k > 0 {
                    c.id = self.alloc_id();
                }
                self.tracks[t].clips.push(c);
            }
            sort_clips(&mut self.tracks[t].clips);
            let under = self.tracks[t]
                .clips
                .iter()
                .find(|c| c.start_beat < at - EPS && at < c.end_beat(bpm) - EPS)?
                .id;
            return self.split_clip(under, at);
        }

        let clip = self.tracks[t].clips[i].clone();
        let rel = at - clip.start_beat;
        if rel <= EPS || at >= clip.end_beat(bpm) - EPS {
            return None;
        }
        let right_id = self.alloc_id();
        let mut left = clip.clone();
        let mut right = clip.clone();
        right.id = right_id;
        right.fade_in = 0.0;
        left.fade_out = 0.0;
        match &clip.kind {
            ClipKind::Audio => {
                // Seconds of source per beat on the timeline.
                let src_per_beat = match clip.stretch {
                    Stretch::Keep => 60.0 / bpm,
                    Stretch::Follow { bpm: locked } => 60.0 / locked,
                } * clip.speed;
                // Cut on a whole output sample so both halves line up exactly.
                let samples = (rel * 60.0 / bpm * sr).round();
                let rel = samples / sr / (60.0 / bpm);
                let src_cut = if clip.speed == 1.0 && matches!(clip.stretch, Stretch::Keep) {
                    samples / sr
                } else {
                    rel * src_per_beat
                };
                if clip.reverse {
                    left.source_start = clip.source_start + clip.source_len - src_cut;
                    left.source_len = src_cut;
                    right.source_len = clip.source_len - src_cut;
                } else {
                    left.source_len = src_cut;
                    right.source_start = clip.source_start + src_cut;
                    right.source_len = clip.source_len - src_cut;
                }
                right.start_beat = clip.start_beat + rel;
            }
            ClipKind::Sliced {
                length_beats,
                slices,
                grid,
                strength,
            } => {
                let spb = 60.0 / bpm;
                let mut ls = Vec::new();
                let mut rs = Vec::new();
                for s in slices {
                    if s.beat < rel - EPS {
                        let mut s = *s;
                        // Trim a slice that would ring past the cut.
                        s.len = s.len.min((rel - s.beat) * spb);
                        ls.push(s);
                    } else {
                        let mut s = *s;
                        s.beat -= rel;
                        s.natural = s.natural.map(|n| n - rel);
                        rs.push(s);
                    }
                }
                left.kind = ClipKind::Sliced {
                    length_beats: rel,
                    slices: ls,
                    grid: *grid,
                    strength: *strength,
                };
                right.kind = ClipKind::Sliced {
                    length_beats: length_beats - rel,
                    slices: rs,
                    grid: *grid,
                    strength: *strength,
                };
                let cut = rel * spb;
                left.source_len = cut.min(clip.source_len);
                right.source_start = clip.source_start + left.source_len;
                right.source_len = (clip.source_len - left.source_len).max(0.0);
                right.start_beat = at;
            }
            ClipKind::Notes {
                length_beats,
                voice,
                notes,
            } => {
                let mut ln = Vec::new();
                let mut rn = Vec::new();
                for n in notes {
                    if n.beat < rel - EPS {
                        let mut n = *n;
                        n.len = n.len.min(rel - n.beat);
                        ln.push(n);
                    } else {
                        let mut n = *n;
                        n.beat -= rel;
                        rn.push(n);
                    }
                }
                left.kind = ClipKind::Notes {
                    length_beats: rel,
                    voice: voice.clone(),
                    notes: ln,
                };
                right.kind = ClipKind::Notes {
                    length_beats: length_beats - rel,
                    voice: voice.clone(),
                    notes: rn,
                };
                let cut = (rel * 60.0 / bpm).min(clip.source_len);
                left.source_len = cut;
                right.source_start = clip.source_start + cut;
                right.source_len = clip.source_len - cut;
                right.start_beat = at;
            }
        }
        self.tracks[t].clips[i] = left;
        self.tracks[t].clips.push(right);
        sort_clips(&mut self.tracks[t].clips);
        Some((id, right_id))
    }

    /// Removes a clip, returning it.
    pub fn remove_clip(&mut self, id: u64) -> Option<Clip> {
        let (t, i) = self.find_clip(id)?;
        Some(self.tracks[t].clips.remove(i))
    }

    /// Adds a clip to a track (giving it a fresh id), returning the id.
    pub fn insert_clip(&mut self, track: usize, mut clip: Clip) -> Option<u64> {
        if track >= self.tracks.len() {
            return None;
        }
        clip.id = self.alloc_id();
        let id = clip.id;
        self.tracks[track].clips.push(clip);
        sort_clips(&mut self.tracks[track].clips);
        Some(id)
    }

    /// Changes the tempo. Sliced and notes clips keep their beat positions
    /// (and so re-time for free); whole-audio clips keep their sound unless
    /// they follow the tempo.
    pub fn set_bpm(&mut self, bpm: f64) {
        self.bpm = bpm.clamp(20.0, 400.0);
    }
}

/// Keeps clips in start order (ties broken by id) so lanes are stable.
pub fn sort_clips(clips: &mut [Clip]) {
    clips.sort_by(|a, b| a.start_beat.total_cmp(&b.start_beat).then(a.id.cmp(&b.id)));
}

/// Greedy interval packing: each span `(start, end)`, taken in start order,
/// goes into the first lane that is free by its start. Returns the lane of
/// each span in input order.
pub fn pack_lanes(spans: &[(f64, f64)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a].0.total_cmp(&spans[b].0).then(a.cmp(&b)));
    let mut lane_end: Vec<f64> = Vec::new();
    let mut out = vec![0; spans.len()];
    for i in order {
        let (s, e) = spans[i];
        match lane_end.iter().position(|&end| end <= s + EPS) {
            Some(l) => {
                lane_end[l] = e;
                out[i] = l;
            }
            None => {
                lane_end.push(e);
                out[i] = lane_end.len() - 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song_with_clips(spans: &[(f64, f64)]) -> Song {
        let mut s = Song::new("t");
        s.bpm = 120.0;
        let t = s.add_track("a");
        for &(start, len_beats) in spans {
            let id = s.alloc_id();
            s.tracks[t]
                .clips
                .push(Clip::audio(id, "c", "s1", 0.0, len_beats * 0.5, start));
        }
        sort_clips(&mut s.tracks[t].clips);
        s
    }

    #[test]
    fn time_signatures() {
        assert_eq!("3/4".parse::<TimeSig>().unwrap().bar_beats(), 3.0);
        assert_eq!("6/8".parse::<TimeSig>().unwrap().bar_beats(), 3.0);
        assert_eq!(TimeSig::FOUR_FOUR.to_string(), "4/4");
        assert!("4/5".parse::<TimeSig>().is_err());
        assert!("four".parse::<TimeSig>().is_err());
    }

    #[test]
    fn lanes_pack_greedily_and_collapse() {
        assert_eq!(pack_lanes(&[(0.0, 4.0), (4.0, 8.0)]), vec![0, 0]);
        assert_eq!(pack_lanes(&[(0.0, 4.0), (2.0, 6.0)]), vec![0, 1]);
        assert_eq!(
            pack_lanes(&[(0.0, 4.0), (2.0, 6.0), (4.0, 5.0), (3.0, 9.0)]),
            vec![0, 1, 0, 2]
        );
        let mut s = song_with_clips(&[(0.0, 4.0), (2.0, 4.0)]);
        assert_eq!(s.lane_count(0), 2);
        let id = s.tracks[0].clips[1].id;
        s.move_clip(id, 0, 4.0);
        assert_eq!(s.lane_count(0), 1);
    }

    #[test]
    fn clip_lengths_follow_tempo_only_when_asked() {
        let mut c = Clip::audio(1, "c", "s", 0.0, 2.0, 0.0);
        assert_eq!(c.len_beats(120.0), 4.0);
        assert_eq!(c.len_beats(60.0), 2.0);
        c.stretch = Stretch::Follow { bpm: 120.0 };
        assert_eq!(c.len_beats(60.0), 4.0);
        c.repeat = 3;
        assert_eq!(c.len_beats(60.0), 12.0);
    }

    #[test]
    fn split_audio_clip() {
        let mut s = song_with_clips(&[(1.0, 4.0)]);
        let id = s.tracks[0].clips[0].id;
        assert!(s.split_clip(id, 1.0).is_none());
        assert!(s.split_clip(id, 5.0).is_none());
        let (l, r) = s.split_clip(id, 2.5).unwrap();
        let (l, r) = (s.clip(l).unwrap(), s.clip(r).unwrap());
        assert_eq!(l.start_beat, 1.0);
        assert!((l.len_beats(120.0) - 1.5).abs() < 1e-9);
        assert!((r.start_beat - 2.5).abs() < 1e-9);
        assert!((r.source_start - 0.75).abs() < 1e-9);
        assert!((r.len_beats(120.0) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn split_repeated_clip_unrolls_it() {
        let mut s = song_with_clips(&[(0.0, 2.0)]);
        s.tracks[0].clips[0].repeat = 3;
        let id = s.tracks[0].clips[0].id;
        s.split_clip(id, 3.0).unwrap();
        let starts: Vec<f64> = s.tracks[0].clips.iter().map(|c| c.start_beat).collect();
        assert_eq!(starts, vec![0.0, 2.0, 3.0, 4.0]);
        assert_eq!(s.end_beats(), 6.0);
    }

    #[test]
    fn split_notes_and_slices() {
        let mut s = Song::new("t");
        s.bpm = 60.0;
        let t = s.add_track("a");
        let mut c = Clip::audio(0, "n", "s", 0.0, 4.0, 0.0);
        c.kind = ClipKind::Notes {
            length_beats: 4.0,
            voice: Voice::Instrument { program: 0 },
            notes: vec![
                Note {
                    pitch: 60,
                    beat: 0.0,
                    len: 2.0,
                    vel: 100,
                },
                Note {
                    pitch: 62,
                    beat: 2.0,
                    len: 1.0,
                    vel: 100,
                },
            ],
        };
        let id = s.insert_clip(t, c).unwrap();
        let (l, r) = s.split_clip(id, 1.0).unwrap();
        match (&s.clip(l).unwrap().kind, &s.clip(r).unwrap().kind) {
            (ClipKind::Notes { notes: a, .. }, ClipKind::Notes { notes: b, .. }) => {
                assert_eq!(a.len(), 1);
                assert_eq!(a[0].len, 1.0);
                assert_eq!(b.len(), 1);
                assert_eq!(b[0].beat, 1.0);
            }
            _ => panic!("kinds changed"),
        }
    }
}
