//! Editor state and actions. Nothing in here draws; see `ui.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use hummr_core::audio::{Metronome, PlayTrack, Player, Recorder};
use hummr_core::dsp::db_to_amp;
use hummr_core::io::{midi, wav};
use hummr_core::pitch::live::LivePitch;
use hummr_core::preview::{self, MixSource};
use hummr_core::rhythm::Grid;
use hummr_core::sequence::{SeqNote, Sequence};
use hummr_core::theory::{Key, detect_key, hz_to_midi};
use hummr_core::transcribe::{
    KeyChoice, TempoChoice, TranscribeOptions, Transcription, transcribe,
};
use hummr_core::{AudioBuffer, instruments};
use ratatui::layout::Rect;
use tachyonfx::EffectManager;

use crate::command::{self, Cmd};
use crate::smooth::{Ema, Follower, Meter};
pub use crate::song::{Clip, EditNote, Placement, Track};
use crate::song::{next_instrument, place};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Command,
    Picker,
    Recording,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Roll,
    Tracker,
    Mixer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lower {
    Tracker,
    Why,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub meter: Meter,
}

/// Animation requests; turned into effects once the draw pass knows areas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxRequest {
    NotesIn,
    Lower,
    Popup,
    NoteFlash(usize),
    RecordStart,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FxKey {
    #[default]
    Roll,
    Lower,
    Popup,
    Note,
    Header,
}

/// Screen areas from the last draw, for mouse hit-testing and effects.
#[derive(Clone, Copy, Debug, Default)]
pub struct Areas {
    pub roll: Rect,
    pub roll_grid: Rect,
    /// Track tabs above the roll, for mouse clicks.
    pub tabs: [Option<(Rect, usize)>; 16],
    /// The bar ruler above the roll (click to move the playhead).
    pub ruler: Rect,
    pub lower: Rect,
    pub mixer: Rect,
    pub popup: Rect,
}

/// The visible window of the piano roll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollView {
    pub top_pitch: u8,
    pub left_beat: f64,
    /// Beats per terminal column.
    pub zoom: f64,
    /// Semitones per row (more than 1 when zoomed out to see everything).
    pub pitch_step: u8,
}

/// Where and how a recording will be placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// `None` for the first take of a song (free tempo, no count-in).
    pub placement: Option<Placement>,
    pub track: usize,
    /// Song beat where the take starts.
    pub at_beat: f64,
    pub count_in_beats: f64,
    /// Song beat the backing (and count-in) starts playing from.
    pub backing_from_beat: Option<f64>,
    /// Song beat where recording stopped (set when it stops).
    pub until_beat: Option<f64>,
}

struct Session {
    recorder: Recorder,
    live: LivePitch,
    plan: Plan,
    /// Recording time (s) at which the count-in/backing started playing.
    play_offset: Option<f64>,
}

struct Job {
    rx: Receiver<Result<Transcription, String>>,
    plan: Plan,
    audio: Arc<AudioBuffer>,
}

#[derive(Clone)]
struct Snapshot {
    tracks: Vec<Track>,
    clips: Vec<Clip>,
    cur: usize,
    selected: usize,
}

pub struct Options {
    pub file: Option<PathBuf>,
    pub animations: bool,
    pub mouse: bool,
}

pub struct App {
    pub mode: Mode,
    pub focus: Focus,
    pub lower: Lower,

    pub tracks: Vec<Track>,
    /// Current track.
    pub cur: usize,
    pub clips: Vec<Clip>,
    pub save_path: Option<PathBuf>,
    pub last_take: Option<PathBuf>,

    pub key_choice: KeyChoice,
    pub quantize: Option<Grid>,
    pub tempo_choice: TempoChoice,
    pub tempo_bpm: f64,
    pub key: Option<Key>,

    /// Transcription of the most recent take, for the "why" panel.
    pub analysis: Option<Transcription>,
    pub selected: usize,
    undo: Vec<Snapshot>,
    pub dirty: bool,

    /// One strip per track, plus `voice` for the recorded takes.
    pub strips: Vec<Strip>,
    pub voice: Strip,
    /// Selected mixer strip; `tracks.len()` means the voice strip.
    pub mixer_sel: usize,
    pub master: Meter,
    pub mic: Meter,

    pub command: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    pub picker_query: String,
    pub picker_results: Vec<(u8, i32)>,
    pub picker_sel: usize,
    /// The playhead when stopped: where playback and recording start.
    pub cursor: f64,

    pub message: Option<(String, Level)>,

    session: Option<Session>,
    pub live_pitch: Ema,
    pub live_hz: Option<f32>,
    /// (song beat, MIDI pitch) samples of the live contour.
    pub contour: Vec<(f64, Option<f32>)>,
    pub record_seconds: f64,
    /// Beats left in the count-in, while it is running.
    pub count_in_left: Option<u32>,
    /// Song beat under the record head.
    pub record_beat: Option<f64>,
    silence_warned: bool,

    player: Option<Player>,
    player_stale: bool,
    pub playhead: Follower,
    pub playing: bool,

    pub animations: bool,
    pub mouse: bool,
    pub mouse_changed: bool,
    pub effects: EffectManager<FxKey>,
    pub fx_requests: Vec<FxRequest>,
    pub areas: Areas,
    pub view: RollView,
    /// Scroll the roll to keep the selection visible (off after mouse scroll).
    pub view_follow: bool,
    /// Fit the roll to the notes on the next draw.
    pub view_fit: bool,
    /// Seconds since start, for blinking and spinners.
    pub clock: f64,

    job: Option<Job>,
    pub busy: Option<String>,
    pub quit: bool,
    /// Draw as if recording, without a microphone (for screenshots only).
    pub(crate) demo_recording: bool,
}

impl App {
    pub fn new(opts: &Options) -> Self {
        Self {
            mode: Mode::Normal,
            focus: Focus::Roll,
            lower: Lower::Tracker,
            tracks: Vec::new(),
            cur: 0,
            clips: Vec::new(),
            save_path: None,
            last_take: None,
            key_choice: KeyChoice::Auto,
            quantize: Some(Grid::EIGHTH),
            tempo_choice: TempoChoice::Auto,
            tempo_bpm: 100.0,
            key: None,
            analysis: None,
            selected: 0,
            undo: Vec::new(),
            dirty: false,
            strips: Vec::new(),
            voice: Strip {
                gain_db: -6.0,
                ..Strip::default()
            },
            mixer_sel: 0,
            master: Meter::default(),
            mic: Meter::default(),
            command: String::new(),
            history: Vec::new(),
            history_pos: None,
            picker_query: String::new(),
            picker_results: Vec::new(),
            picker_sel: 0,
            cursor: 0.0,
            message: Some((
                "Press r and hum a melody, or :open take.wav. Press ? for help.".into(),
                Level::Info,
            )),
            session: None,
            live_pitch: Ema::new(0.06),
            live_hz: None,
            contour: Vec::new(),
            record_seconds: 0.0,
            count_in_left: None,
            record_beat: None,
            silence_warned: false,
            player: None,
            player_stale: true,
            playhead: Follower::default(),
            playing: false,
            animations: opts.animations,
            mouse: opts.mouse,
            mouse_changed: false,
            effects: EffectManager::default(),
            fx_requests: Vec::new(),
            areas: Areas::default(),
            view: RollView {
                top_pitch: 76,
                left_beat: 0.0,
                zoom: 0.25,
                pitch_step: 1,
            },
            view_follow: true,
            view_fit: false,
            clock: 0.0,
            job: None,
            busy: None,
            quit: false,
            demo_recording: false,
        }
    }

    // ---- status -------------------------------------------------------

    pub fn info(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Level::Info));
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Level::Warn));
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Level::Error));
    }

    fn fx(&mut self, req: FxRequest) {
        if self.animations {
            self.fx_requests.push(req);
        }
    }

    pub fn is_recording(&self) -> bool {
        self.session.is_some() || self.demo_recording
    }

    /// Whether the screen is changing on its own (so we redraw at ~60 fps).
    pub fn is_live(&self) -> bool {
        self.is_recording()
            || self.playing
            || self.busy.is_some()
            || self.effects.is_running()
            || !self.master.is_idle()
            || !self.voice.meter.is_idle()
            || self.strips.iter().any(|s| !s.meter.is_idle())
            || !self.mic.is_idle()
    }

    pub fn grid_beats(&self) -> f64 {
        self.quantize.map_or(0.5, |g| g.beats())
    }

    pub fn seconds_per_beat(&self) -> f64 {
        60.0 / self.tempo_bpm
    }

    // ---- tracks ---------------------------------------------------------

    pub fn track(&self) -> Option<&Track> {
        self.tracks.get(self.cur)
    }

    /// Notes of the current track.
    pub fn notes(&self) -> &[EditNote] {
        self.track().map_or(&[], |t| &t.notes)
    }

    fn notes_mut(&mut self) -> Option<&mut Vec<EditNote>> {
        self.tracks.get_mut(self.cur).map(|t| &mut t.notes)
    }

    pub fn song_is_empty(&self) -> bool {
        self.clips.is_empty() && self.tracks.iter().all(|t| t.notes.is_empty())
    }

    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.notes.len()).sum()
    }

    fn sync_strips(&mut self) {
        self.strips.resize_with(self.tracks.len(), Strip::default);
        self.cur = self.cur.min(self.tracks.len().saturating_sub(1));
        self.mixer_sel = self.mixer_sel.min(self.tracks.len());
    }

    /// Switches to track `i`, keeping the selection near the same time.
    pub fn select_track(&mut self, i: usize) {
        if i >= self.tracks.len() {
            self.warn(format!(
                "there is no track {} ({} track{})",
                i + 1,
                self.tracks.len(),
                if self.tracks.len() == 1 { "" } else { "s" }
            ));
            return;
        }
        let t = self.notes().get(self.selected).map_or(0.0, EditNote::start);
        self.cur = i;
        self.mixer_sel = i;
        self.selected = self
            .notes()
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.start() - t).abs().total_cmp(&(b.1.start() - t).abs()))
            .map_or(0, |(j, _)| j);
        let name = self.tracks[i].display_name();
        self.info(format!("track {}: {name}", i + 1));
    }

    pub fn sequence_of(&self, i: usize) -> Sequence {
        let t = &self.tracks[i];
        let mut seq = Sequence::new(t.display_name(), self.tempo_bpm);
        seq.key = self.key;
        seq.program = Some(t.program);
        seq.notes = t.notes.iter().map(|n| n.note).collect();
        seq
    }

    pub fn title(&self) -> String {
        self.save_path
            .as_ref()
            .or(self.last_take.as_ref())
            .and_then(|p| p.file_name())
            .map_or_else(
                || "untitled".to_string(),
                |s| s.to_string_lossy().into_owned(),
            )
    }

    // ---- files ----------------------------------------------------------

    pub fn open(&mut self, path: &Path) {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "wav" | "wave" => match wav::read_mono(path) {
                Ok(audio) => {
                    self.checkpoint();
                    self.tracks = vec![Track::new(40)];
                    self.clips.clear();
                    self.sync_strips();
                    self.cur = 0;
                    self.last_take = Some(path.to_path_buf());
                    self.save_path = Some(path.with_extension("mid"));
                    self.start_job(Arc::new(audio), first_take_plan());
                }
                Err(e) => self.error(e.to_string()),
            },
            "mid" | "midi" => match midi::read_song_file(path) {
                Ok(song) => {
                    self.checkpoint();
                    self.clips.clear();
                    self.analysis = None;
                    self.tempo_bpm = song.tempo_bpm;
                    self.key = song.key;
                    self.tracks = song
                        .tracks
                        .iter()
                        .map(|s| Track {
                            name: (!s.name.is_empty()).then(|| s.name.clone()),
                            program: s.program.unwrap_or(0),
                            notes: s
                                .notes
                                .iter()
                                .map(|&note| EditNote { note, sung: None })
                                .collect(),
                        })
                        .collect();
                    self.sync_strips();
                    self.cur = 0;
                    self.selected = 0;
                    self.save_path = Some(path.to_path_buf());
                    self.player_stale = true;
                    self.view_fit = true;
                    self.fx(FxRequest::NotesIn);
                    self.dirty = false;
                    self.info(format!(
                        "opened {}: {} track{}, {} notes",
                        path.display(),
                        self.tracks.len(),
                        if self.tracks.len() == 1 { "" } else { "s" },
                        self.note_count()
                    ));
                }
                Err(e) => self.error(e.to_string()),
            },
            _ => self.error(format!(
                "can't open {}: expected a .wav recording or a .mid file",
                path.display()
            )),
        }
    }

    pub fn save_midi(&mut self, path: Option<&str>) -> bool {
        let path = path
            .map(PathBuf::from)
            .or_else(|| self.save_path.clone())
            .unwrap_or_else(|| PathBuf::from("song.mid"));
        if self.note_count() == 0 {
            self.warn("nothing to save yet: record or open something first");
            return false;
        }
        let seqs: Vec<(String, u8, Vec<SeqNote>)> = self
            .tracks
            .iter()
            .map(|t| {
                (
                    t.display_name(),
                    t.program,
                    t.notes.iter().map(|n| n.note).collect(),
                )
            })
            .collect();
        let parts: Vec<midi::SongTrack<'_>> = seqs
            .iter()
            .map(|(name, program, notes)| midi::SongTrack {
                name,
                program: *program,
                notes,
            })
            .collect();
        let name = path
            .file_stem()
            .map_or_else(|| "song".to_string(), |s| s.to_string_lossy().into_owned());
        match midi::write_song_file(&path, &name, self.tempo_bpm, self.key, &parts) {
            Ok(()) => {
                self.info(format!(
                    "saved {} ({} track{}, {} notes)",
                    path.display(),
                    self.tracks.len(),
                    if self.tracks.len() == 1 { "" } else { "s" },
                    self.note_count()
                ));
                self.save_path = Some(path);
                self.dirty = false;
                true
            }
            Err(e) => {
                self.error(e.to_string());
                false
            }
        }
    }

    /// Stereo mix of every track plus the recorded voice.
    pub fn render_mix(&self, sample_rate: u32) -> Vec<f32> {
        let sources: Vec<MixSource> = self
            .play_tracks(sample_rate)
            .into_iter()
            .map(|t| MixSource {
                samples: t.samples,
                gain: t.gain,
                pan: t.pan,
                muted: t.muted,
            })
            .collect();
        preview::mix_stereo(&sources)
    }

    fn export(&mut self, path: &str) {
        let p = PathBuf::from(path);
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "mid" | "midi" => {
                self.save_midi(Some(path));
            }
            "wav" => {
                if self.song_is_empty() {
                    self.warn("nothing to export yet");
                    return;
                }
                let mix = self.render_mix(44_100);
                match wav::write(&p, &mix, 2, 44_100, wav::WavFormat::Pcm16) {
                    Ok(()) => self.info(format!(
                        "exported {} ({:.1} s, preview synth: SoundFont rendering comes later)",
                        p.display(),
                        mix.len() as f64 / 2.0 / 44_100.0
                    )),
                    Err(e) => self.error(e.to_string()),
                }
            }
            _ => self.error("export to a .wav (audio mix) or .mid (notes) file"),
        }
    }

    // ---- transcription jobs ------------------------------------------------

    fn start_job(&mut self, audio: Arc<AudioBuffer>, plan: Plan) {
        let opts = match plan.placement {
            None => TranscribeOptions {
                key: self.key_choice,
                quantize: self.quantize,
                tempo: self.tempo_choice,
                ..TranscribeOptions::default()
            },
            // Later takes follow the song: same tempo, key and grid, with the
            // grid anchored at the end of the count-in.
            Some(_) => TranscribeOptions {
                key: self.key.map_or(KeyChoice::Auto, KeyChoice::Fixed),
                quantize: self.quantize,
                tempo: TempoChoice::Fixed(self.tempo_bpm),
                grid_origin: None,
                ..TranscribeOptions::default()
            },
        };
        self.start_job_with(audio, plan, opts);
    }

    fn start_job_with(&mut self, audio: Arc<AudioBuffer>, plan: Plan, opts: TranscribeOptions) {
        let (tx, rx) = std::sync::mpsc::channel();
        let a = Arc::clone(&audio);
        std::thread::spawn(move || {
            let _ = tx.send(transcribe(&a, &opts).map_err(|e| e.to_string()));
        });
        self.job = Some(Job { rx, plan, audio });
        self.busy = Some("transcribing".into());
    }

    fn finish_job(&mut self, plan: Plan, audio: Arc<AudioBuffer>, t: Transcription) {
        let take: Vec<EditNote> = t
            .notes
            .iter()
            .map(|n| EditNote {
                note: SeqNote {
                    pitch: n.pitch,
                    start_beats: n.start_beats,
                    duration_beats: n.duration_beats,
                    velocity: n.velocity,
                },
                sung: Some(n.detected),
            })
            .collect();
        let count = take.len();
        let changed = t.changes.len();

        let Some(placement) = plan.placement else {
            // First take (or re-transcription): it defines the song.
            self.tempo_bpm = t.tempo_bpm;
            self.key = t.key;
            if self.tracks.is_empty() {
                self.tracks.push(Track::new(40));
                self.sync_strips();
            }
            let tr = plan.track.min(self.tracks.len() - 1);
            self.tracks[tr].notes = take;
            self.cur = tr;
            self.selected = 0;
            self.clips = vec![Clip {
                audio,
                at_beat: 0.0,
                origin_seconds: t.origin_seconds,
            }];
            self.analysis = Some(t);
            self.after_change(true);
            let key = self.key.map_or("no key".to_string(), |k| k.to_string());
            if count == 0 {
                self.warn("no sung notes found. Hum louder and closer to the mic, with clear 'doo' or 'la' sounds");
            } else {
                self.info(format!(
                    "{count} notes · {key} · {:.0} BPM · {changed} adjustments (t shows why). Space plays · r adds a layer · R redoes this track.",
                    self.tempo_bpm
                ));
            }
            return;
        };

        if take.is_empty() {
            self.analysis = Some(t);
            self.warn("no sung notes found in that take, so nothing changed. Press r to try again");
            return;
        }

        // Both placements are sung in time with the song, so the take's
        // beat 0 is exactly the beat recording started at.
        self.checkpoint();
        let tr = if placement == Placement::Overlay {
            let used: Vec<u8> = self.tracks.iter().map(|t| t.program).collect();
            self.tracks.push(Track::new(next_instrument(&used)));
            self.sync_strips();
            self.tracks.len() - 1
        } else {
            plan.track.min(self.tracks.len() - 1)
        };
        let until = plan.until_beat.unwrap_or(plan.at_beat);
        let placed = place(
            &mut self.tracks[tr].notes,
            &take,
            placement,
            plan.at_beat,
            until,
        );
        self.clips.push(Clip {
            audio,
            at_beat: plan.at_beat,
            origin_seconds: t.origin_seconds,
        });
        self.cur = tr;
        self.mixer_sel = tr;
        let first = take[0].note;
        self.selected = self
            .notes()
            .iter()
            .position(|n| {
                n.note.pitch == first.pitch
                    && (n.start() - plan.at_beat - first.start_beats).abs() < 1e-6
            })
            .unwrap_or(0);
        self.analysis = Some(t);
        self.after_change(false);

        let at = position(plan.at_beat);
        let name = self.tracks[tr].display_name();
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let msg = match placement {
            Placement::Overlay => format!(
                "new layer {}: {name}, {count} note{} from {at}. Press i to change its instrument",
                tr + 1,
                plural(count)
            ),
            Placement::Overwrite => format!(
                "re-recorded {name} from {at}: {} old note{} replaced by {count}",
                placed.removed,
                plural(placed.removed)
            ),
        };
        self.info(msg);
    }

    fn after_change(&mut self, fit: bool) {
        self.dirty = true;
        self.player_stale = true;
        self.view_follow = true;
        if fit {
            self.view_fit = true;
            self.view.left_beat = 0.0;
        }
        self.fx(FxRequest::NotesIn);
    }

    /// Re-runs transcription of a single-take song with new settings.
    fn retranscribe(&mut self) -> bool {
        if self.clips.len() == 1 && self.tracks.len() <= 1 {
            let audio = Arc::clone(&self.clips[0].audio);
            if !self.undo.is_empty() {
                self.warn("re-transcribing the take; note edits were replaced (u to undo)");
            }
            self.checkpoint();
            self.start_job(audio, first_take_plan());
            true
        } else {
            false
        }
    }

    // ---- per-frame update ----------------------------------------------

    pub fn tick(&mut self, dt: Duration) {
        let dts = dt.as_secs_f32();
        self.clock += dt.as_secs_f64();

        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(result) => {
                    let job = self.job.take().expect("job present");
                    self.busy = None;
                    match result {
                        Ok(t) => self.finish_job(job.plan, job.audio, t),
                        Err(e) => self.error(e),
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.busy = None;
                    self.error("transcription crashed; please report this with the recording");
                }
                Err(TryRecvError::Empty) => {}
            }
        }

        let bps = self.tempo_bpm / 60.0;
        if let Some(s) = &mut self.session {
            let peak = s.recorder.take_peak();
            let secs = s.recorder.seconds();
            let tail = s.recorder.tail(s.live.window_len());
            let heard = s.recorder.heard_anything();
            let err = s.recorder.error();
            let hz = s.live.estimate(&tail);
            let plan = s.plan;
            let play_offset = s.play_offset;

            self.record_seconds = secs;
            self.mic.update(peak, dts);
            self.live_hz = hz;
            self.live_pitch.update(hz.map(hz_to_midi), dts);
            let beat = match play_offset {
                Some(off) => {
                    let into = (secs - off).max(0.0) * bps;
                    self.count_in_left = (into < plan.count_in_beats)
                        .then(|| (plan.count_in_beats - into).ceil() as u32);
                    plan.backing_from_beat
                        .unwrap_or(plan.at_beat - plan.count_in_beats)
                        + into
                }
                None => plan.at_beat + secs * bps,
            };
            self.record_beat = Some(beat);
            if self.count_in_left.is_none() {
                self.contour.push((beat, self.live_pitch.value));
            }
            if !heard && secs > 1.5 && !self.silence_warned {
                self.silence_warned = true;
                self.error(format!(
                    "the microphone is sending pure silence. {}",
                    hummr_core::audio::mic_permission_hint()
                ));
            }
            if let Some(e) = err {
                self.error(format!("microphone error: {e}"));
            }
        } else {
            self.mic.update(0.0, dts);
        }

        let mut peaks = (Vec::new(), 0.0);
        if let Some(p) = &self.player {
            let was_playing = self.playing;
            self.playing = p.is_playing();
            if was_playing && !self.playing && self.session.is_none() {
                self.cursor = self.playhead.value.max(0.0);
            }
            if self.session.is_none() {
                let beat = p.position() as f64 / f64::from(p.sample_rate()) * bps;
                self.playhead
                    .update(beat, bps, dt.as_secs_f64(), self.playing);
            }
            peaks = p.take_peaks();
            if let Some(e) = p.error() {
                self.error(format!("playback error: {e}"));
            }
        }
        let recording = self.session.is_some();
        for (i, s) in self.strips.iter_mut().enumerate() {
            let p = if recording {
                0.0
            } else {
                peaks.0.get(i).copied().unwrap_or(0.0)
            };
            s.meter.update(p, dts);
        }
        let vp = if recording {
            0.0
        } else {
            peaks.0.get(self.tracks.len()).copied().unwrap_or(0.0)
        };
        self.voice.meter.update(vp, dts);
        self.master.update(peaks.1, dts);
    }

    // ---- recording ------------------------------------------------------

    /// `r`: on an empty song, record the first take freely; afterwards,
    /// record a new instrument layer while the song plays along.
    pub fn press_record(&mut self) {
        self.record(Placement::Overlay);
    }

    /// `R`: re-record the current track from the playhead.
    pub fn press_record_overwrite(&mut self) {
        self.record(Placement::Overwrite);
    }

    fn record(&mut self, placement: Placement) {
        if self.is_recording() {
            self.stop_recording(true);
        } else if self.busy.is_some() {
            self.warn("still transcribing the last take; one moment");
        } else if self.song_is_empty() || self.tracks.is_empty() {
            self.begin_recording(first_take_plan());
        } else {
            let plan = self.plan_for(placement);
            self.begin_recording(plan);
        }
    }

    /// Recording starts on the beat at (or just before) the playhead, after
    /// a one-bar count-in during which the song is already playing.
    pub fn plan_for(&self, placement: Placement) -> Plan {
        let at = self.cursor.max(0.0).floor();
        Plan {
            placement: Some(placement),
            track: self.cur,
            at_beat: at,
            count_in_beats: 4.0,
            backing_from_beat: Some(at - 4.0),
            until_beat: None,
        }
    }

    /// The plan of the recording in progress.
    pub fn recording_plan(&self) -> Option<Plan> {
        self.session.as_ref().map(|s| s.plan)
    }

    fn ensure_player(&mut self) -> bool {
        if self.player.is_none() {
            match Player::open() {
                Ok(p) => self.player = Some(p),
                Err(e) => {
                    self.error(e.to_string());
                    return false;
                }
            }
        }
        true
    }

    fn begin_recording(&mut self, mut plan: Plan) {
        self.stop();
        let recorder = match Recorder::start() {
            Ok(r) => r,
            Err(e) => return self.error(e.to_string()),
        };
        let live = LivePitch::new(recorder.sample_rate());
        let mut play_offset = None;
        if plan.count_in_beats > 0.0 {
            if self.ensure_player() {
                let sr = self.player.as_ref().map_or(48_000, Player::sample_rate);
                let from = plan
                    .backing_from_beat
                    .unwrap_or(plan.at_beat - plan.count_in_beats);
                // When re-recording a track, you hear everything except it.
                let skip = (plan.placement == Some(Placement::Overwrite)).then_some(plan.track);
                let backing = self.backing_tracks(sr, from, skip);
                let bpm = self.tempo_bpm;
                if let Some(p) = &self.player {
                    p.set_tracks(backing);
                    p.set_metronome(Some(Metronome::new(bpm, sr).starting_at_beat(from)));
                    p.play_from(0);
                }
                self.player_stale = true;
                play_offset = Some(recorder.seconds());
            } else {
                // No speakers: record without a count-in; the grid is fitted.
                plan.count_in_beats = 0.0;
            }
        }
        let how = match plan.placement {
            None => "hum your melody!".to_string(),
            Some(Placement::Overlay) => format!(
                "new layer from {} after the count-in (the song plays along; headphones help)",
                position(plan.at_beat)
            ),
            Some(Placement::Overwrite) => format!(
                "re-recording {} from {} after the count-in",
                self.track()
                    .map_or("this track".into(), Track::display_name),
                position(plan.at_beat)
            ),
        };
        self.info(format!(
            "recording from {}: {how}. r / space / enter to stop, esc to cancel",
            recorder.device_name()
        ));
        self.session = Some(Session {
            recorder,
            live,
            plan,
            play_offset,
        });
        self.contour.clear();
        self.record_seconds = 0.0;
        self.count_in_left = None;
        self.silence_warned = false;
        self.mode = Mode::Recording;
        self.fx(FxRequest::RecordStart);
    }

    /// Existing tracks rendered from song beat `from` (may be negative,
    /// meaning silence first), for playing along while recording.
    fn backing_tracks(&self, sr: u32, from: f64, skip: Option<usize>) -> Vec<PlayTrack> {
        let offset = (from * self.seconds_per_beat() * f64::from(sr)).round() as i64;
        let mut tracks = self.play_tracks(sr);
        tracks.truncate(self.tracks.len()); // leave the voice out
        if let Some(i) = skip {
            if let Some(t) = tracks.get_mut(i) {
                t.muted = true;
            }
        }
        for t in &mut tracks {
            t.samples = if offset >= 0 {
                t.samples
                    .get(offset as usize..)
                    .unwrap_or_default()
                    .to_vec()
            } else {
                let mut v = vec![0.0; (-offset) as usize];
                v.extend_from_slice(&t.samples);
                v
            };
        }
        tracks
    }

    pub fn stop_recording(&mut self, keep: bool) {
        let Some(mut s) = self.session.take() else {
            return;
        };
        self.mode = Mode::Normal;
        self.live_pitch.value = None;
        self.count_in_left = None;
        self.record_beat = None;
        if let Some(p) = &self.player {
            p.stop();
            p.set_metronome(None);
        }
        self.player_stale = true;
        if !keep {
            self.info("recording cancelled");
            return;
        }
        let spb = self.seconds_per_beat();
        let origin = s.play_offset.map(|o| o + s.plan.count_in_beats * spb);
        match s.recorder.stop() {
            Ok(audio) => {
                if audio.duration_seconds() < origin.unwrap_or(0.0) + 0.3 {
                    self.warn("stopped before the count-in finished; press r to try again");
                    return;
                }
                let path = next_take_path();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = wav::write_mono(&path, &audio, wav::WavFormat::Pcm16) {
                    self.error(format!("could not save the take: {e}"));
                }
                if s.plan.placement.is_none() {
                    self.save_path
                        .get_or_insert_with(|| path.with_extension("mid"));
                }
                if let Some(o) = origin {
                    s.plan.until_beat = Some(s.plan.at_beat + (audio.duration_seconds() - o) / spb);
                }
                self.last_take = Some(path);
                let audio = Arc::new(audio);
                match s.plan.placement {
                    None => self.start_job(audio, s.plan),
                    Some(_) => {
                        let opts = TranscribeOptions {
                            key: self.key.map_or(KeyChoice::Auto, KeyChoice::Fixed),
                            quantize: self.quantize,
                            tempo: TempoChoice::Fixed(self.tempo_bpm),
                            grid_origin: origin,
                            ..TranscribeOptions::default()
                        };
                        self.start_job_with(audio, s.plan, opts);
                    }
                }
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    // ---- playback -------------------------------------------------------

    fn strip_audible(&self, s: &Strip) -> bool {
        let any_solo = self
            .strips
            .iter()
            .chain(std::iter::once(&self.voice))
            .any(|s| s.solo);
        !(s.muted || (any_solo && !s.solo))
    }

    /// The recorded voice clips placed on the song timeline.
    fn render_voice(&self, sr: u32) -> Vec<f32> {
        let spb = self.seconds_per_beat();
        let mut out: Vec<f32> = Vec::new();
        for c in &self.clips {
            let a = preview::resample_linear(&c.audio.samples, c.audio.sample_rate, sr);
            let start = ((c.at_beat * spb - c.origin_seconds) * f64::from(sr)).round() as i64;
            let end = start + a.len() as i64;
            if end > out.len() as i64 {
                out.resize(end.max(0) as usize, 0.0);
            }
            for (i, x) in a.iter().enumerate() {
                let j = start + i as i64;
                if j >= 0 {
                    out[j as usize] += x;
                }
            }
        }
        out
    }

    /// Every track (instrument preview) followed by the voice.
    fn play_tracks(&self, sr: u32) -> Vec<PlayTrack> {
        let mut out: Vec<PlayTrack> = (0..self.tracks.len())
            .map(|i| {
                let s = &self.strips[i];
                PlayTrack {
                    samples: preview::render(&self.sequence_of(i), sr, 0.0),
                    gain: db_to_amp(s.gain_db),
                    pan: s.pan,
                    muted: !self.strip_audible(s),
                }
            })
            .collect();
        out.push(PlayTrack {
            samples: self.render_voice(sr),
            gain: db_to_amp(self.voice.gain_db),
            pan: self.voice.pan,
            muted: !self.strip_audible(&self.voice),
        });
        out
    }

    fn sync_mixer(&mut self) {
        if self.session.is_some() {
            return;
        }
        if let Some(p) = &self.player {
            for (i, s) in self
                .strips
                .iter()
                .chain(std::iter::once(&self.voice))
                .enumerate()
            {
                p.set_track_params(i, db_to_amp(s.gain_db), s.pan, !self.strip_audible(s));
            }
        }
    }

    /// Space: play from the playhead, or stop and leave the playhead there.
    pub fn toggle_play(&mut self) {
        if self.playing {
            self.stop();
        } else {
            let end = self.tracks.iter().map(Track::end_beats).fold(0.0, f64::max);
            if self.cursor >= end {
                self.cursor = 0.0;
            }
            self.play_from_beat(Some(self.cursor));
        }
    }

    /// Moves the playhead by `beats`, snapping to multiples of that step.
    pub fn scrub(&mut self, beats: f64) {
        let from = if self.playing {
            self.playhead.value
        } else {
            self.cursor
        };
        let step = beats.abs();
        self.cursor = (((from + beats) / step).round() * step).max(0.0);
        if self.playing {
            self.play_from_beat(Some(self.cursor));
        }
    }

    pub fn scrub_to(&mut self, beat: f64) {
        self.cursor = beat.max(0.0);
        if self.playing {
            self.play_from_beat(Some(self.cursor));
        }
    }

    /// Plays the song from a beat (None = from the start).
    pub fn play_from_beat(&mut self, beat: Option<f64>) {
        if self.song_is_empty() {
            self.warn("nothing to play: press r to record, or :open a .wav or .mid");
            return;
        }
        if !self.ensure_player() {
            return;
        }
        let Some(sr) = self.player.as_ref().map(Player::sample_rate) else {
            return;
        };
        if self.player_stale {
            let tracks = self.play_tracks(sr);
            if let Some(p) = &self.player {
                p.set_metronome(None);
                p.set_tracks(tracks);
            }
            self.player_stale = false;
        }
        let beat = beat.unwrap_or(0.0).max(0.0);
        self.cursor = beat;
        self.playhead.value = beat;
        let start = beat * self.seconds_per_beat();
        if let Some(p) = &self.player {
            p.play_from((start * f64::from(sr)) as usize);
        }
        self.playing = true;
    }

    pub fn stop(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
        if self.playing {
            self.cursor = self.playhead.value.max(0.0);
        }
        self.playing = false;
    }

    // ---- editing --------------------------------------------------------

    fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            tracks: self.tracks.clone(),
            clips: self.clips.clone(),
            cur: self.cur,
            selected: self.selected,
        });
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.dirty = true;
        self.player_stale = true;
    }

    pub fn select(&mut self, delta: i32) {
        let len = self.notes().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        if !self.playing {
            self.cursor = self.notes()[self.selected].start();
        }
    }

    pub fn transpose(&mut self, semitones: i32) {
        let Some(n) = self.notes().get(self.selected).copied() else {
            return;
        };
        let p = i32::from(n.note.pitch) + semitones;
        if !(0..=127).contains(&p) {
            return;
        }
        self.checkpoint();
        let sel = self.selected;
        if let Some(notes) = self.notes_mut() {
            notes[sel].note.pitch = p as u8;
        }
        let key = self.key;
        let name = key.map_or_else(
            || hummr_core::theory::note_name(p as u8),
            |k| k.note_name(p as u8),
        );
        match key.filter(|k| !k.contains(p as u8)) {
            Some(k) => self.warn(format!("{name} is outside {k}")),
            None => self.info(name),
        }
        self.fx(FxRequest::NoteFlash(sel));
    }

    pub fn change_length(&mut self, steps: i32) {
        let g = self.grid_beats();
        let Some(n) = self.notes().get(self.selected).copied() else {
            return;
        };
        let d = (n.note.duration_beats + f64::from(steps) * g).max(g);
        if (d - n.note.duration_beats).abs() < 1e-9 {
            return;
        }
        self.checkpoint();
        let sel = self.selected;
        if let Some(notes) = self.notes_mut() {
            notes[sel].note.duration_beats = d;
        }
        self.info(format!("length {}", hummr_core::notation::format_length(d)));
        self.fx(FxRequest::NoteFlash(sel));
    }

    pub fn shift_time(&mut self, steps: i32) {
        let g = self.grid_beats();
        let Some(n) = self.notes().get(self.selected).copied() else {
            return;
        };
        let s = (n.note.start_beats + f64::from(steps) * g).max(0.0);
        if (s - n.note.start_beats).abs() < 1e-9 {
            return;
        }
        self.checkpoint();
        let sel = self.selected;
        let cur = self.cur;
        self.tracks[cur].notes[sel].note.start_beats = s;
        let moved = self.tracks[cur].notes[sel];
        self.tracks[cur].sort();
        self.selected = self.notes().iter().position(|n| *n == moved).unwrap_or(0);
        self.fx(FxRequest::NoteFlash(self.selected));
    }

    pub fn delete(&mut self) {
        if self.notes().is_empty() {
            return;
        }
        self.checkpoint();
        let sel = self.selected;
        let removed = self.notes_mut().map(|n| n.remove(sel));
        self.selected = self.selected.min(self.notes().len().saturating_sub(1));
        if let Some(n) = removed {
            self.info(format!(
                "deleted {}",
                hummr_core::theory::note_name(n.note.pitch)
            ));
        }
    }

    /// Types notes into the current track at the playhead (see `:notes`).
    pub(crate) fn type_notes_at(&mut self, beat: f64, text: &str) {
        self.cursor = beat;
        self.type_notes(text);
    }

    /// Waits for a background transcription to finish (screenshots, tests).
    pub(crate) fn wait_for_job(&mut self) {
        while self.busy.is_some() {
            std::thread::sleep(Duration::from_millis(5));
            self.tick(Duration::from_millis(5));
        }
    }

    /// Makes sure there is a track to put notes on.
    fn ensure_track(&mut self) {
        if self.tracks.is_empty() {
            self.tracks.push(Track::new(40));
            self.sync_strips();
            self.cur = 0;
        }
    }

    /// `a`: adds a note at the playhead on the current track. It takes the
    /// pitch of the note before it (so you can then nudge it with j/k) and
    /// moves the playhead to its end, so pressing `a` again writes the next.
    pub fn add_note(&mut self) {
        self.ensure_track();
        let g = self.grid_beats();
        let start = (self.cursor / g + 1e-9).floor() * g;
        let pitch = self
            .notes()
            .iter()
            .rev()
            .find(|n| n.start() < start + 1e-9)
            .or_else(|| self.notes().get(self.selected))
            .map_or(60, |n| n.note.pitch);
        let len = g.max(1.0);
        self.add_note_at(pitch, start, len);
        self.cursor = start + len;
        let name = self.key.map_or_else(
            || hummr_core::theory::note_name(pitch),
            |k| k.note_name(pitch),
        );
        self.info(format!(
            "added {name} at {}: j/k pitch, < > length, a for the next note",
            position(start)
        ));
    }

    /// Adds one note to the current track and selects it.
    pub fn add_note_at(&mut self, pitch: u8, start: f64, beats: f64) {
        self.ensure_track();
        self.checkpoint();
        let note = EditNote {
            note: SeqNote {
                pitch,
                start_beats: start.max(0.0),
                duration_beats: beats,
                velocity: 96,
            },
            sung: None,
        };
        let cur = self.cur;
        self.tracks[cur].notes.push(note);
        self.tracks[cur].sort();
        self.selected = self.notes().iter().position(|n| *n == note).unwrap_or(0);
        self.fx(FxRequest::NoteFlash(self.selected));
    }

    /// `:notes C4 1/4 E4 1/8 r 1/8`: types a phrase in at the playhead.
    fn type_notes(&mut self, text: &str) {
        let items = match hummr_core::notation::parse(text) {
            Ok(items) => items,
            Err(e) => return self.error(e.to_string()),
        };
        self.ensure_track();
        self.checkpoint();
        let start = self.cursor;
        let mut at = start;
        let mut added = 0;
        let cur = self.cur;
        for item in items {
            match item {
                hummr_core::notation::Item::Note { pitch, beats } => {
                    self.tracks[cur].notes.push(EditNote {
                        note: SeqNote {
                            pitch,
                            start_beats: at,
                            duration_beats: beats,
                            velocity: 96,
                        },
                        sung: None,
                    });
                    added += 1;
                    at += beats;
                }
                hummr_core::notation::Item::Rest { beats } => at += beats,
            }
        }
        self.tracks[cur].sort();
        self.selected = self
            .notes()
            .iter()
            .position(|n| (n.start() - start).abs() < 1e-9)
            .unwrap_or(0);
        self.cursor = at;
        self.view_follow = true;
        self.info(format!(
            "typed {added} note{} into track {} from {}",
            if added == 1 { "" } else { "s" },
            cur + 1,
            position(start)
        ));
    }

    /// `0`: zooms the roll out so every note of every track is in view.
    pub fn fit_view(&mut self) {
        self.view_fit = true;
        self.view_follow = false;
        if self.note_count() == 0 {
            self.info("no notes to show yet");
        }
    }

    /// `M`: joins the selected note with the next one into one held note,
    /// e.g. when a voice crack split a long note in two.
    pub fn join_next(&mut self) {
        let sel = self.selected;
        let Some((a, b)) = self
            .notes()
            .get(sel)
            .zip(self.notes().get(sel + 1))
            .map(|(a, b)| (*a, *b))
        else {
            return self.warn("no next note to join with");
        };
        self.checkpoint();
        if let Some(notes) = self.notes_mut() {
            notes[sel].note.duration_beats = (b.end() - a.start()).max(a.note.duration_beats);
            notes.remove(sel + 1);
        }
        let len = hummr_core::notation::format_length(self.notes()[sel].note.duration_beats);
        self.info(format!("joined into one {len} note"));
        self.fx(FxRequest::NoteFlash(sel));
    }

    /// `D` (or `x` on a mixer strip): deletes the current track. Undoable.
    pub fn delete_track(&mut self) {
        if self.tracks.is_empty() {
            return self.warn("no track to delete");
        }
        self.checkpoint();
        let i = self.cur;
        let t = self.tracks.remove(i);
        if i < self.strips.len() {
            self.strips.remove(i);
        }
        self.sync_strips();
        self.selected = 0;
        self.info(format!(
            "deleted track {}: {} ({} notes). Press u to undo",
            i + 1,
            t.display_name(),
            t.notes.len()
        ));
    }

    pub fn undo(&mut self) {
        match self.undo.pop() {
            Some(s) => {
                self.tracks = s.tracks;
                self.clips = s.clips;
                self.cur = s.cur;
                self.selected = s.selected;
                self.sync_strips();
                self.selected = self.selected.min(self.notes().len().saturating_sub(1));
                self.player_stale = true;
                self.info("undone");
            }
            None => self.info("nothing to undo"),
        }
    }

    fn track_command(&mut self, arg: &str) {
        let (sub, rest) = match arg.split_once(char::is_whitespace) {
            Some((a, b)) => (a, b.trim()),
            None => (arg, ""),
        };
        if let Ok(n) = sub.parse::<usize>() {
            return self.select_track(n.saturating_sub(1));
        }
        match sub {
            "" | "list" => {
                let list: Vec<String> = self
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{} {}", i + 1, t.display_name()))
                    .collect();
                self.info(if list.is_empty() { "no tracks yet".into() } else { list.join(" · ") });
            }
            "new" | "add" => {
                let program = if rest.is_empty() {
                    let used: Vec<u8> = self.tracks.iter().map(|t| t.program).collect();
                    next_instrument(&used)
                } else {
                    match instruments::find(rest) {
                        Ok(p) => p,
                        Err(e) => return self.error(e.to_string()),
                    }
                };
                self.checkpoint();
                self.tracks.push(Track::new(program));
                self.sync_strips();
                let i = self.tracks.len() - 1;
                self.select_track(i);
                self.info(format!(
                    "new empty track {}: {}. Press r and choose append to fill it",
                    i + 1,
                    instruments::name(program)
                ));
            }
            "del" | "delete" | "rm" | "remove" => self.delete_track(),
            "name" | "rename" => {
                if rest.is_empty() {
                    return self.error("usage: :track name Bass");
                }
                if let Some(t) = self.tracks.get_mut(self.cur) {
                    t.name = Some(rest.to_string());
                    self.dirty = true;
                }
            }
            other => self.error(format!(
                "unknown :track {other}; try :track 2, :track new cello, :track del, :track name Bass"
            )),
        }
    }

    // ---- mixer ------------------------------------------------------------

    fn strip_mut(&mut self) -> &mut Strip {
        if self.mixer_sel < self.strips.len() {
            &mut self.strips[self.mixer_sel]
        } else {
            &mut self.voice
        }
    }

    pub fn mixer_select(&mut self, delta: i32) {
        let max = self.tracks.len() as i32;
        self.mixer_sel = (self.mixer_sel as i32 + delta).clamp(0, max) as usize;
        if self.mixer_sel < self.tracks.len() {
            self.cur = self.mixer_sel;
            self.selected = self.selected.min(self.notes().len().saturating_sub(1));
        }
    }

    pub fn mixer_gain(&mut self, db: f32) {
        let s = self.strip_mut();
        s.gain_db = (s.gain_db + db).clamp(-48.0, 12.0);
        self.sync_mixer();
    }

    pub fn mixer_pan(&mut self, delta: f32) {
        let s = self.strip_mut();
        s.pan = (s.pan + delta).clamp(-1.0, 1.0);
        self.sync_mixer();
    }

    pub fn mixer_mute(&mut self) {
        let s = self.strip_mut();
        s.muted = !s.muted;
        self.sync_mixer();
    }

    pub fn mixer_solo(&mut self) {
        let s = self.strip_mut();
        s.solo = !s.solo;
        self.sync_mixer();
    }

    // ---- panels and popups ---------------------------------------------------

    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Roll => Focus::Tracker,
            Focus::Tracker => Focus::Mixer,
            Focus::Mixer => Focus::Roll,
        };
        if self.focus == Focus::Mixer {
            self.mixer_sel = self.cur.min(self.tracks.len());
        }
    }

    pub fn toggle_lower(&mut self) {
        self.lower = match self.lower {
            Lower::Tracker => Lower::Why,
            Lower::Why => Lower::Tracker,
        };
        self.fx(FxRequest::Lower);
    }

    pub fn open_picker(&mut self) {
        if self.tracks.is_empty() {
            self.tracks.push(Track::new(40));
            self.sync_strips();
        }
        self.mode = Mode::Picker;
        self.picker_query.clear();
        self.refresh_picker();
        let program = self.track().map_or(40, |t| t.program);
        self.picker_sel = self
            .picker_results
            .iter()
            .position(|(p, _)| *p == program)
            .unwrap_or(0);
        self.fx(FxRequest::Popup);
    }

    pub fn refresh_picker(&mut self) {
        self.picker_results = instruments::search(&self.picker_query);
        self.picker_sel = 0;
    }

    pub fn picker_move(&mut self, delta: i32) {
        if self.picker_results.is_empty() {
            return;
        }
        let max = self.picker_results.len() as i32 - 1;
        self.picker_sel = (self.picker_sel as i32 + delta).clamp(0, max) as usize;
    }

    pub fn picker_choose(&mut self) {
        if let Some(&(p, _)) = self.picker_results.get(self.picker_sel) {
            self.set_instrument(p);
        }
        self.mode = Mode::Normal;
    }

    pub fn set_instrument(&mut self, program: u8) {
        if self.tracks.is_empty() {
            self.tracks.push(Track::new(program));
            self.sync_strips();
        }
        let cur = self.cur;
        self.tracks[cur].program = program;
        self.player_stale = true;
        self.dirty = true;
        self.info(format!(
            "track {}: {} (saved into the MIDI file; playback uses a preview synth for now)",
            cur + 1,
            instruments::name(program)
        ));
    }

    pub fn open_command(&mut self) {
        self.mode = Mode::Command;
        self.command.clear();
        self.history_pos = None;
    }

    pub fn command_history(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len();
        let pos = match (self.history_pos, up) {
            (None, true) => len - 1,
            (Some(p), true) => p.saturating_sub(1),
            (Some(p), false) if p + 1 < len => p + 1,
            _ => {
                self.history_pos = None;
                self.command.clear();
                return;
            }
        };
        self.history_pos = Some(pos);
        self.command = self.history[pos].clone();
    }

    pub fn run_command_line(&mut self) {
        let line = std::mem::take(&mut self.command);
        self.mode = Mode::Normal;
        if line.trim().is_empty() {
            return;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        match command::parse(&line) {
            Ok(cmd) => self.execute(cmd),
            Err(e) => self.error(e),
        }
    }

    pub fn execute(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit { force } => {
                if self.dirty && !force {
                    self.warn("unsaved changes: :w to save, :q! to quit anyway");
                } else {
                    self.quit = true;
                }
            }
            Cmd::Write(path) => {
                self.save_midi(path.as_deref());
            }
            Cmd::WriteQuit => {
                if self.save_midi(None) || self.note_count() == 0 {
                    self.quit = true;
                }
            }
            Cmd::Export(path) => self.export(&path),
            Cmd::Open(path) => self.open(Path::new(&path)),
            Cmd::Key(arg) => match arg.parse::<KeyChoice>() {
                Ok(choice) => self.apply_key(choice),
                Err(e) => self.error(e.to_string()),
            },
            Cmd::Tempo(arg) => match arg.parse::<TempoChoice>() {
                Ok(choice) => self.apply_tempo(choice),
                Err(e) => self.error(e.to_string()),
            },
            Cmd::Quantize(arg) => match Grid::parse_optional(&arg) {
                Ok(grid) => self.apply_quantize(grid),
                Err(e) => self.error(e.to_string()),
            },
            Cmd::Instrument(name) => match instruments::find(&name) {
                Ok(p) => self.set_instrument(p),
                Err(e) => self.error(e.to_string()),
            },
            Cmd::Track(arg) => self.track_command(&arg),
            Cmd::Transpose(n) => self.transpose_track(n),
            Cmd::Notes(text) => self.type_notes(&text),
            Cmd::Record(Placement::Overlay) => self.press_record(),
            Cmd::Record(Placement::Overwrite) => self.press_record_overwrite(),
            Cmd::Play => self.play_from_beat(None),
            Cmd::Stop => self.stop(),
            Cmd::Animations(on) => {
                self.animations = on;
                if !on {
                    self.effects = EffectManager::default();
                    self.fx_requests.clear();
                }
                self.info(format!("animations {}", if on { "on" } else { "off" }));
            }
            Cmd::Mouse(on) => {
                self.mouse = on;
                self.mouse_changed = true;
                self.info(format!("mouse {}", if on { "on" } else { "off" }));
            }
            Cmd::Retranscribe => {
                if !self.retranscribe() {
                    self.warn("re-transcribing only works while the song is a single take");
                }
            }
            Cmd::Undo => self.undo(),
            Cmd::Help => self.mode = Mode::Help,
        }
    }

    fn transpose_track(&mut self, semitones: i32) {
        if self.notes().is_empty() {
            return self.warn("this track has no notes to transpose");
        }
        let out_of_range = self.notes().iter().any(|n| {
            let p = i32::from(n.note.pitch) + semitones;
            !(0..=127).contains(&p)
        });
        if out_of_range {
            return self.error("that would push notes outside the MIDI range");
        }
        self.checkpoint();
        if let Some(notes) = self.notes_mut() {
            for n in notes {
                n.note.pitch = (i32::from(n.note.pitch) + semitones) as u8;
            }
        }
        self.info(format!(
            "transposed track {} by {semitones:+} semitones",
            self.cur + 1
        ));
    }

    fn apply_key(&mut self, choice: KeyChoice) {
        self.key_choice = choice;
        if self.retranscribe() {
            return;
        }
        match choice {
            KeyChoice::Off => {
                self.key = None;
                self.info("key off: notes keep their pitches");
            }
            KeyChoice::Fixed(k) => {
                self.checkpoint();
                let mut moved = 0;
                for t in &mut self.tracks {
                    for n in &mut t.notes {
                        let p = k.snap(f32::from(n.note.pitch)).pitch;
                        moved += usize::from(p != n.note.pitch);
                        n.note.pitch = p;
                    }
                }
                self.key = Some(k);
                self.info(format!("key {k}: {moved} notes snapped"));
            }
            KeyChoice::Auto => {
                let weighted: Vec<(u8, f64)> = self
                    .tracks
                    .iter()
                    .flat_map(|t| {
                        t.notes
                            .iter()
                            .map(|n| (n.note.pitch, n.note.duration_beats))
                    })
                    .collect();
                match detect_key(&weighted) {
                    Some(est) => {
                        self.key = Some(est.key);
                        self.info(format!("detected key {}", est.key));
                    }
                    None => self.warn("no notes to detect a key from"),
                }
            }
        }
        self.player_stale = true;
    }

    fn apply_tempo(&mut self, choice: TempoChoice) {
        self.tempo_choice = choice;
        if self.retranscribe() {
            return;
        }
        match choice {
            TempoChoice::Fixed(bpm) => {
                self.tempo_bpm = bpm;
                self.dirty = true;
                self.player_stale = true;
                self.info(format!(
                    "tempo {bpm} BPM{}",
                    if self.clips.is_empty() {
                        ""
                    } else {
                        " (your recorded voice keeps its own speed)"
                    }
                ));
            }
            TempoChoice::Auto => {
                self.warn("tempo detection needs a single take; give a BPM like :tempo 100")
            }
        }
    }

    fn apply_quantize(&mut self, grid: Option<Grid>) {
        self.quantize = grid;
        if self.retranscribe() {
            return;
        }
        if let Some(g) = grid {
            self.checkpoint();
            let step = g.beats();
            for t in &mut self.tracks {
                for n in &mut t.notes {
                    let s = (n.note.start_beats / step).round() * step;
                    let e = ((n.note.start_beats + n.note.duration_beats) / step).round() * step;
                    n.note.start_beats = s;
                    n.note.duration_beats = (e - s).max(step);
                }
            }
            self.info(format!("quantized to {g}"));
        }
    }
}

fn first_take_plan() -> Plan {
    Plan {
        placement: None,
        track: 0,
        at_beat: 0.0,
        count_in_beats: 0.0,
        backing_from_beat: None,
        until_beat: None,
    }
}

/// "bar 2 beat 3" for a beat offset.
pub fn position(beat: f64) -> String {
    hummr_core::transcribe::Position(beat).to_string()
}

fn next_take_path() -> PathBuf {
    let dir = PathBuf::from("takes");
    (1..10_000)
        .map(|i| dir.join(format!("take-{i:03}.wav")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join("take.wav"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hummr_core::synthetic::{self, HumParams, TWINKLE};
    use hummr_core::theory::Scale;

    fn new_app() -> App {
        App::new(&Options {
            file: None,
            animations: true,
            mouse: false,
        })
    }

    fn app_with_notes() -> App {
        let mut app = new_app();
        let notes = [(60u8, 0.0), (66, 1.0), (67, 2.0)]
            .iter()
            .map(|&(p, s)| EditNote {
                note: SeqNote {
                    pitch: p,
                    start_beats: s,
                    duration_beats: 1.0,
                    velocity: 100,
                },
                sung: None,
            })
            .collect();
        app.tracks = vec![Track {
            notes,
            ..Track::new(40)
        }];
        app.sync_strips();
        app
    }

    fn wait(app: &mut App) {
        for _ in 0..1000 {
            app.tick(Duration::from_millis(5));
            if app.busy.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("transcription did not finish");
    }

    fn take(notes: &[(u8, f64)], bpm: f64, lead_in: f64) -> Arc<AudioBuffer> {
        let tones = synthetic::melody(notes, bpm, lead_in, 0.0, 3);
        Arc::new(AudioBuffer::new(
            synthetic::hum(&tones, &HumParams::new(44_100), 3),
            44_100,
        ))
    }

    /// Simulates a recorded take arriving for `plan` (as if sung after a
    /// 4-beat count-in that started at t = 0).
    fn feed(app: &mut App, plan: Plan, notes: &[(u8, f64)]) {
        let bpm = app.tempo_bpm;
        let count_in = 4.0 * 60.0 / bpm;
        let audio = take(notes, bpm, count_in);
        let opts = TranscribeOptions {
            key: app.key.map_or(KeyChoice::Auto, KeyChoice::Fixed),
            quantize: app.quantize,
            tempo: TempoChoice::Fixed(bpm),
            grid_origin: Some(count_in),
            ..TranscribeOptions::default()
        };
        app.start_job_with(audio, plan, opts);
        wait(app);
    }

    fn pitches(app: &App, track: usize) -> Vec<(u8, f64)> {
        app.tracks[track]
            .notes
            .iter()
            .map(|n| (n.note.pitch, n.note.start_beats))
            .collect()
    }

    fn first_song() -> App {
        let mut app = new_app();
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        app.open(&{
            // Tests run in parallel: give each its own file.
            let p =
                std::env::temp_dir().join(format!("hummr-first-{}-{n}.wav", std::process::id()));
            wav::write_mono(&p, &take(&TWINKLE[..4], 100.0, 0.3), wav::WavFormat::Pcm16).unwrap();
            p
        });
        wait(&mut app);
        app
    }

    #[test]
    fn first_take_defines_the_song() {
        let app = first_song();
        assert_eq!(app.tracks.len(), 1);
        assert_eq!(
            pitches(&app, 0),
            vec![(60, 0.0), (60, 1.0), (67, 2.0), (67, 3.0)]
        );
        assert_eq!(app.clips.len(), 1);
        assert_eq!(app.key, Some(Key::new(0, Scale::Major)));
    }

    #[test]
    fn overwrite_replaces_from_the_playhead_for_as_long_as_you_sang() {
        let mut app = first_song(); // C4 C4 G4 G4 on beats 0..3
        app.cursor = 1.0;
        let mut plan = app.plan_for(Placement::Overwrite);
        assert_eq!(plan.at_beat, 1.0);
        assert_eq!(plan.backing_from_beat, Some(-3.0));
        plan.until_beat = Some(3.0);
        feed(&mut app, plan, &[(64, 1.0), (65, 1.0)]);
        assert_eq!(
            pitches(&app, 0),
            vec![(60, 0.0), (64, 1.0), (65, 2.0), (67, 3.0)]
        );
        assert_eq!(app.tracks.len(), 1, "overwrite never adds a track");
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .0
                .contains("2 old notes replaced by 2")
        );
    }

    #[test]
    fn overlay_adds_a_new_instrument_track_in_time() {
        let mut app = first_song();
        let plan = app.plan_for(Placement::Overlay);
        assert_eq!(plan.at_beat, 0.0);
        assert_eq!(plan.backing_from_beat, Some(-4.0));
        feed(&mut app, plan, &[(48, 1.0), (55, 1.0), (48, 2.0)]);
        assert_eq!(app.tracks.len(), 2);
        assert_ne!(app.tracks[1].program, app.tracks[0].program);
        assert_eq!(pitches(&app, 1), vec![(48, 0.0), (55, 1.0), (48, 2.0)]);
        assert_eq!(pitches(&app, 0).len(), 4, "the original track is untouched");
        assert_eq!(app.cur, 1);
        assert_eq!(app.strips.len(), 2);
        app.undo();
        assert_eq!(app.tracks.len(), 1);
    }

    #[test]
    fn overlay_starts_at_the_playhead() {
        let mut app = first_song();
        app.cursor = 2.6; // snaps back to beat 2
        let plan = app.plan_for(Placement::Overlay);
        assert_eq!(plan.at_beat, 2.0);
        feed(&mut app, plan, &[(72, 2.0)]);
        assert_eq!(pitches(&app, 1), vec![(72, 2.0)]);
    }

    #[test]
    fn playhead_follows_selection_and_scrubbing() {
        let mut app = app_with_notes();
        app.select(2);
        assert_eq!(app.cursor, 2.0);
        app.scrub(1.0);
        assert_eq!(app.cursor, 3.0);
        app.scrub(-4.0);
        assert_eq!(app.cursor, 0.0, "never before the start");
        app.scrub_to(7.5);
        assert_eq!(app.cursor, 7.5);
    }

    #[test]
    fn editing_and_undo() {
        let mut app = app_with_notes();
        app.select(1);
        app.transpose(1);
        assert_eq!(app.notes()[1].note.pitch, 67);
        app.change_length(1);
        assert_eq!(app.notes()[1].note.duration_beats, 1.5);
        app.delete();
        assert_eq!(app.notes().len(), 2);
        app.undo();
        app.undo();
        app.undo();
        assert_eq!(app.notes()[1].note.pitch, 66);
        assert_eq!(app.notes()[1].note.duration_beats, 1.0);
        assert!(app.dirty);
    }

    #[test]
    fn shift_time_keeps_selection_on_moved_note() {
        let mut app = app_with_notes();
        app.select(2);
        app.shift_time(-5);
        assert_eq!(app.notes()[app.selected].note.pitch, 67);
        assert_eq!(app.notes()[app.selected].note.start_beats, 0.0);
    }

    #[test]
    fn key_command_snaps_every_track() {
        let mut app = app_with_notes();
        app.execute(command::parse(":key C major").unwrap());
        assert_eq!(app.key, Some(Key::new(0, Scale::Major)));
        let p: Vec<u8> = app.notes().iter().map(|n| n.note.pitch).collect();
        assert_eq!(p, vec![60, 65, 67]);
    }

    #[test]
    fn track_commands() {
        let mut app = app_with_notes();
        app.execute(command::parse(":track new cello").unwrap());
        assert_eq!(app.tracks.len(), 2);
        assert_eq!(app.tracks[1].program, 42);
        assert_eq!(app.cur, 1);
        app.execute(command::parse(":track name Bass").unwrap());
        assert_eq!(app.tracks[1].display_name(), "Bass");
        app.execute(command::parse(":track 1").unwrap());
        assert_eq!(app.cur, 0);
        app.execute(command::parse(":transpose -12").unwrap());
        assert_eq!(app.notes()[0].note.pitch, 48);
        app.execute(command::parse(":track 2").unwrap());
        app.execute(command::parse(":track del").unwrap());
        assert_eq!(app.tracks.len(), 1);
        assert_eq!(app.strips.len(), 1);
    }

    #[test]
    fn adding_notes_by_hand() {
        let mut app = new_app();
        app.add_note();
        assert_eq!(app.tracks.len(), 1, "a track is created on demand");
        assert_eq!(pitches(&app, 0), vec![(60, 0.0)]);
        app.transpose(4);
        app.add_note();
        app.add_note();
        assert_eq!(pitches(&app, 0), vec![(64, 0.0), (64, 1.0), (64, 2.0)]);
        assert_eq!(app.cursor, 3.0);
        assert_eq!(app.selected, 2);
        app.execute(command::parse(":notes G4 1/8 r 1/8 C5 1/2").unwrap());
        assert_eq!(&pitches(&app, 0)[3..], &[(67, 3.0), (72, 4.0)]);
        assert_eq!(app.notes()[4].note.duration_beats, 2.0);
        assert_eq!(app.cursor, 6.0);
        app.undo();
        assert_eq!(app.notes().len(), 3);
        app.execute(command::parse(":notes Q9 1/4").unwrap());
        assert_eq!(app.message.as_ref().unwrap().1, Level::Error);
    }

    #[test]
    fn join_and_delete_track() {
        let mut app = app_with_notes();
        app.select(1);
        app.join_next();
        assert_eq!(app.notes().len(), 2);
        assert_eq!(app.notes()[1].note.duration_beats, 2.0);
        app.execute(command::parse(":track new cello").unwrap());
        assert_eq!(app.tracks.len(), 2);
        app.delete_track();
        assert_eq!(app.tracks.len(), 1);
        assert!(app.message.as_ref().unwrap().0.contains("u to undo"));
        app.undo();
        assert_eq!(app.tracks.len(), 2);
    }

    #[test]
    fn tempo_and_quit_guard() {
        let mut app = app_with_notes();
        app.execute(Cmd::Tempo("90".into()));
        assert_eq!(app.tempo_bpm, 90.0);
        app.execute(Cmd::Quit { force: false });
        assert!(!app.quit, "dirty song must not quit");
        app.execute(Cmd::Quit { force: true });
        assert!(app.quit);
    }

    #[test]
    fn picker_sets_the_current_tracks_instrument() {
        let mut app = app_with_notes();
        app.open_picker();
        app.picker_query = "flute".into();
        app.refresh_picker();
        app.picker_choose();
        assert_eq!(app.tracks[0].program, 73);
    }

    #[test]
    fn multitrack_save_and_reopen() {
        let mut app = app_with_notes();
        app.execute(command::parse(":track new bass").unwrap());
        let mut bass = app.tracks[0].notes.clone();
        for n in &mut bass {
            n.note.pitch -= 24;
        }
        app.tracks[1].notes = bass;
        let path = std::env::temp_dir().join(format!("hummr-tui-{}.mid", std::process::id()));
        app.execute(Cmd::Write(Some(path.to_string_lossy().into_owned())));
        assert!(!app.dirty);
        let mut other = new_app();
        other.open(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(other.tracks.len(), 2);
        assert_eq!(other.tracks[0].program, 40);
        assert_eq!(other.tracks[1].program, 33);
        assert_eq!(other.tracks[1].notes[0].note.pitch, 36);
    }

    #[test]
    fn wav_export_mixes_all_tracks() {
        let mut app = app_with_notes();
        app.execute(command::parse(":track new piano").unwrap());
        app.tracks[1].notes = app.tracks[0].notes.clone();
        let path = std::env::temp_dir().join(format!("hummr-tui-{}.wav", std::process::id()));
        app.execute(Cmd::Export(path.to_string_lossy().into_owned()));
        let audio = wav::read_mono(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(audio.duration_seconds() > 1.5);
        assert!(audio.peak() > 0.05);
    }
}
