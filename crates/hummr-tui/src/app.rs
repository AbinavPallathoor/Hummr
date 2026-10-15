//! Editor state and actions. Nothing in here draws; see `ui.rs`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use hummr_core::audio::{PlayTrack, Player, Recorder};
use hummr_core::dsp::db_to_amp;
use hummr_core::io::{midi, wav};
use hummr_core::pitch::live::LivePitch;
use hummr_core::preview::{self, MixSource};
use hummr_core::rhythm::Grid;
use hummr_core::segment::DetectedNote;
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

/// A note in the editor, remembering what was actually sung (if anything).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditNote {
    pub note: SeqNote,
    pub sung: Option<DetectedNote>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub name: &'static str,
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    pub meter: Meter,
}

impl Strip {
    fn new(name: &'static str, gain_db: f32) -> Self {
        Self {
            name,
            gain_db,
            pan: 0.0,
            muted: false,
            solo: false,
            meter: Meter::default(),
        }
    }
}

pub const VOICE: usize = 0;
pub const LEAD: usize = 1;

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
}

pub struct App {
    pub mode: Mode,
    pub focus: Focus,
    pub lower: Lower,

    pub take: Option<AudioBuffer>,
    pub take_path: Option<PathBuf>,
    pub save_path: Option<PathBuf>,

    pub key_choice: KeyChoice,
    pub quantize: Option<Grid>,
    pub tempo_choice: TempoChoice,

    pub analysis: Option<Transcription>,
    pub notes: Vec<EditNote>,
    pub tempo_bpm: f64,
    pub key: Option<Key>,
    pub program: u8,
    /// Time in the take that corresponds to beat 0.
    pub origin_seconds: f64,
    pub selected: usize,
    undo: Vec<Vec<EditNote>>,
    pub dirty: bool,

    pub strips: Vec<Strip>,
    pub strip_sel: usize,
    pub master: Meter,
    pub mic: Meter,

    pub command: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    pub picker_query: String,
    pub picker_results: Vec<(u8, i32)>,
    pub picker_sel: usize,

    pub message: Option<(String, Level)>,

    recorder: Option<Recorder>,
    live: Option<LivePitch>,
    pub live_pitch: Ema,
    pub live_hz: Option<f32>,
    /// (seconds since record start, MIDI pitch) samples of the live contour.
    pub contour: Vec<(f64, Option<f32>)>,
    pub record_seconds: f64,
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

    job: Option<Receiver<Result<Transcription, String>>>,
    pub busy: Option<String>,
    pub quit: bool,
}

pub struct Options {
    pub file: Option<PathBuf>,
    pub animations: bool,
    pub mouse: bool,
}

impl App {
    pub fn new(opts: &Options) -> Self {
        Self {
            mode: Mode::Normal,
            focus: Focus::Roll,
            lower: Lower::Tracker,
            take: None,
            take_path: None,
            save_path: None,
            key_choice: KeyChoice::Auto,
            quantize: Some(Grid::EIGHTH),
            tempo_choice: TempoChoice::Auto,
            analysis: None,
            notes: Vec::new(),
            tempo_bpm: 100.0,
            key: None,
            program: 40,
            origin_seconds: 0.0,
            selected: 0,
            undo: Vec::new(),
            dirty: false,
            strips: vec![Strip::new("voice", -6.0), Strip::new("lead", 0.0)],
            strip_sel: LEAD,
            master: Meter::default(),
            mic: Meter::default(),
            command: String::new(),
            history: Vec::new(),
            history_pos: None,
            picker_query: String::new(),
            picker_results: Vec::new(),
            picker_sel: 0,
            message: Some((
                "Press r and hum a melody, or :open take.wav. Press ? for help.".into(),
                Level::Info,
            )),
            recorder: None,
            live: None,
            live_pitch: Ema::new(0.06),
            live_hz: None,
            contour: Vec::new(),
            record_seconds: 0.0,
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
            },
            view_follow: true,
            view_fit: false,
            clock: 0.0,
            job: None,
            busy: None,
            quit: false,
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
        self.recorder.is_some()
    }

    /// Whether the screen is changing on its own (so we redraw at ~60 fps).
    pub fn is_live(&self) -> bool {
        self.is_recording()
            || self.playing
            || self.busy.is_some()
            || self.effects.is_running()
            || !self.master.is_idle()
            || self.strips.iter().any(|s| !s.meter.is_idle())
            || !self.mic.is_idle()
    }

    pub fn grid_beats(&self) -> f64 {
        self.quantize.map_or(0.5, |g| g.beats())
    }

    pub fn sequence(&self) -> Sequence {
        let name = self
            .take_path
            .as_ref()
            .or(self.save_path.as_ref())
            .and_then(|p| p.file_stem())
            .map_or_else(|| "lead".to_string(), |s| s.to_string_lossy().into_owned());
        let mut seq = Sequence::new(name, self.tempo_bpm);
        seq.key = self.key;
        seq.program = Some(self.program);
        seq.notes = self.notes.iter().map(|n| n.note).collect();
        seq
    }

    pub fn title(&self) -> String {
        self.take_path
            .as_ref()
            .or(self.save_path.as_ref())
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
                    self.take = Some(audio);
                    self.take_path = Some(path.to_path_buf());
                    self.save_path = Some(path.with_extension("mid"));
                    self.player_stale = true;
                    self.start_transcription();
                }
                Err(e) => self.error(e.to_string()),
            },
            "mid" | "midi" => match midi::read_file(path) {
                Ok(seq) => {
                    self.take = None;
                    self.take_path = None;
                    self.analysis = None;
                    self.save_path = Some(path.to_path_buf());
                    self.tempo_bpm = seq.tempo_bpm;
                    self.key = seq.key;
                    if let Some(p) = seq.program {
                        self.program = p;
                    }
                    self.origin_seconds = 0.0;
                    self.set_notes(
                        seq.notes
                            .iter()
                            .map(|&note| EditNote { note, sung: None })
                            .collect(),
                    );
                    self.dirty = false;
                    self.info(format!(
                        "opened {} ({} notes)",
                        path.display(),
                        self.notes.len()
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

    fn set_notes(&mut self, mut notes: Vec<EditNote>) {
        notes.sort_by(|a, b| a.note.start_beats.total_cmp(&b.note.start_beats));
        self.notes = notes;
        self.selected = 0;
        self.undo.clear();
        self.player_stale = true;
        self.view.left_beat = 0.0;
        self.view_fit = true;
        self.view_follow = true;
        self.fx(FxRequest::NotesIn);
    }

    fn options(&self) -> TranscribeOptions {
        TranscribeOptions {
            key: self.key_choice,
            quantize: self.quantize,
            tempo: self.tempo_choice,
            ..TranscribeOptions::default()
        }
    }

    pub fn start_transcription(&mut self) {
        let Some(audio) = self.take.clone() else {
            self.warn("no recording to transcribe: press r to record or :open a .wav");
            return;
        };
        if !self.undo.is_empty() {
            self.warn("re-transcribing from the recording; your manual note edits were replaced");
        }
        let opts = self.options();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(transcribe(&audio, &opts).map_err(|e| e.to_string()));
        });
        self.job = Some(rx);
        self.busy = Some("transcribing".into());
    }

    fn finish_transcription(&mut self, t: Transcription) {
        self.tempo_bpm = t.tempo_bpm;
        self.key = t.key;
        self.origin_seconds = t.origin_seconds;
        let notes = t
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
        let count = t.notes.len();
        let key = t.key.map_or("no key".to_string(), |k| k.to_string());
        let changed = t.changes.len();
        self.analysis = Some(t);
        self.set_notes(notes);
        self.dirty = true;
        if count == 0 {
            self.warn("no sung notes found. Try humming louder and closer to the mic, with clear 'doo' or 'la' sounds");
        } else {
            self.info(format!(
                "{count} notes · {key} · {:.0} BPM · {changed} adjustments (t shows why). Space plays, :w saves.",
                self.tempo_bpm
            ));
        }
    }

    pub fn save_midi(&mut self, path: Option<&str>) -> bool {
        let path = match path.map(PathBuf::from).or_else(|| self.save_path.clone()) {
            Some(p) => p,
            None => PathBuf::from("lead.mid"),
        };
        if self.notes.is_empty() {
            self.warn("nothing to save yet: record or open something first");
            return false;
        }
        match midi::write_file(&path, &self.sequence()) {
            Ok(()) => {
                self.info(format!(
                    "saved {} ({} notes)",
                    path.display(),
                    self.notes.len()
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

    /// Stereo mix of the voice and the lead preview.
    pub fn render_mix(&self, sample_rate: u32) -> Vec<f32> {
        let tracks = self.tracks(sample_rate);
        let sources: Vec<MixSource> = tracks
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
                if self.notes.is_empty() && self.take.is_none() {
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

    // ---- per-frame update ----------------------------------------------

    pub fn tick(&mut self, dt: Duration) {
        let dts = dt.as_secs_f32();
        self.clock += dt.as_secs_f64();
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(result) => {
                    self.job = None;
                    self.busy = None;
                    match result {
                        Ok(t) => self.finish_transcription(t),
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

        if let Some(rec) = &self.recorder {
            let peak = rec.take_peak();
            self.record_seconds = rec.seconds();
            let tail = self
                .live
                .as_ref()
                .map(|l| rec.tail(l.window_len()))
                .unwrap_or_default();
            let heard = rec.heard_anything();
            let err = rec.error();
            self.mic.update(peak, dts);
            self.live_hz = self.live.as_mut().and_then(|l| l.estimate(&tail));
            self.live_pitch.update(self.live_hz.map(hz_to_midi), dts);
            self.contour
                .push((self.record_seconds, self.live_pitch.value));
            if !heard && self.record_seconds > 1.5 && !self.silence_warned {
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
            let secs = p.position() as f64 / f64::from(p.sample_rate());
            let beat = (secs - self.origin_seconds) * self.tempo_bpm / 60.0;
            self.playhead
                .update(beat, self.tempo_bpm / 60.0, dt.as_secs_f64(), self.playing);
            peaks = p.take_peaks();
            if was_playing && !self.playing {
                self.playhead.value = beat;
            }
            if let Some(e) = p.error() {
                self.error(format!("playback error: {e}"));
            }
        }
        for (i, s) in self.strips.iter_mut().enumerate() {
            s.meter.update(peaks.0.get(i).copied().unwrap_or(0.0), dts);
        }
        self.master.update(peaks.1, dts);
    }

    // ---- recording ------------------------------------------------------

    pub fn start_recording(&mut self) {
        if self.recorder.is_some() {
            return;
        }
        self.stop();
        match Recorder::start() {
            Ok(rec) => {
                self.info(format!(
                    "recording from {}: hum your melody! r / space / enter to stop, esc to cancel",
                    rec.device_name()
                ));
                self.live = Some(LivePitch::new(rec.sample_rate()));
                self.recorder = Some(rec);
                self.contour.clear();
                self.record_seconds = 0.0;
                self.silence_warned = false;
                self.mode = Mode::Recording;
                self.fx(FxRequest::RecordStart);
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn stop_recording(&mut self, keep: bool) {
        let Some(rec) = self.recorder.take() else {
            return;
        };
        self.mode = Mode::Normal;
        self.live = None;
        self.live_pitch.value = None;
        if !keep {
            self.info("recording cancelled");
            return;
        }
        match rec.stop() {
            Ok(audio) => {
                if audio.duration_seconds() < 0.3 {
                    self.warn("that take was too short; press r to try again");
                    return;
                }
                let path = next_take_path();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = wav::write_mono(&path, &audio, wav::WavFormat::Pcm16) {
                    self.error(format!("could not save the take: {e}"));
                }
                self.take = Some(audio);
                self.take_path = Some(path.clone());
                self.save_path = Some(path.with_extension("mid"));
                self.player_stale = true;
                self.undo.clear();
                self.start_transcription();
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    // ---- playback -------------------------------------------------------

    fn tracks(&self, sample_rate: u32) -> Vec<PlayTrack> {
        let any_solo = self.strips.iter().any(|s| s.solo);
        let strip_params = |i: usize| {
            let s = &self.strips[i];
            (
                db_to_amp(s.gain_db),
                s.pan,
                s.muted || (any_solo && !s.solo),
            )
        };
        let voice = self
            .take
            .as_ref()
            .map(|t| preview::resample_linear(&t.samples, t.sample_rate, sample_rate))
            .unwrap_or_default();
        let lead_offset = if self.take.is_some() {
            self.origin_seconds
        } else {
            0.0
        };
        let lead = preview::render(&self.sequence(), sample_rate, lead_offset.max(0.0));
        let (vg, vp, vm) = strip_params(VOICE);
        let (lg, lp, lm) = strip_params(LEAD);
        vec![
            PlayTrack {
                samples: voice,
                gain: vg,
                pan: vp,
                muted: vm,
            },
            PlayTrack {
                samples: lead,
                gain: lg,
                pan: lp,
                muted: lm,
            },
        ]
    }

    fn sync_mixer(&mut self) {
        if let Some(p) = &self.player {
            let any_solo = self.strips.iter().any(|s| s.solo);
            for (i, s) in self.strips.iter().enumerate() {
                p.set_track_params(
                    i,
                    db_to_amp(s.gain_db),
                    s.pan,
                    s.muted || (any_solo && !s.solo),
                );
            }
        }
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.stop();
        } else {
            self.play_from_beat(None);
        }
    }

    /// Plays from a beat (None = from the start of the take).
    pub fn play_from_beat(&mut self, beat: Option<f64>) {
        if self.notes.is_empty() && self.take.is_none() {
            self.warn("nothing to play: press r to record, or :open a .wav or .mid");
            return;
        }
        if self.player.is_none() {
            match Player::open() {
                Ok(p) => self.player = Some(p),
                Err(e) => {
                    self.error(e.to_string());
                    return;
                }
            }
        }
        let Some(sr) = self.player.as_ref().map(Player::sample_rate) else {
            return;
        };
        if self.player_stale {
            let tracks = self.tracks(sr);
            if let Some(p) = &self.player {
                p.set_tracks(tracks);
            }
            self.player_stale = false;
        }
        let start_secs = beat.map_or(0.0, |b| {
            (self.origin_seconds + b * 60.0 / self.tempo_bpm).max(0.0)
        });
        if let Some(p) = &self.player {
            p.play_from((start_secs * f64::from(sr)) as usize);
        }
        self.playing = true;
    }

    pub fn stop(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
        self.playing = false;
    }

    // ---- editing --------------------------------------------------------

    fn checkpoint(&mut self) {
        self.undo.push(self.notes.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.dirty = true;
        self.player_stale = true;
    }

    pub fn select(&mut self, delta: i32) {
        if self.notes.is_empty() {
            return;
        }
        let max = self.notes.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, max) as usize;
    }

    pub fn transpose(&mut self, semitones: i32) {
        let Some(n) = self.notes.get(self.selected) else {
            return;
        };
        let p = i32::from(n.note.pitch) + semitones;
        if !(0..=127).contains(&p) {
            return;
        }
        self.checkpoint();
        self.notes[self.selected].note.pitch = p as u8;
        let key = self.key;
        let name = key.map_or_else(
            || hummr_core::theory::note_name(p as u8),
            |k| k.note_name(p as u8),
        );
        let warn = key.is_some_and(|k| !k.contains(p as u8));
        if warn {
            self.warn(format!(
                "{name} is outside {}",
                key.map(|k| k.to_string()).unwrap_or_default()
            ));
        } else {
            self.info(name);
        }
        self.fx(FxRequest::NoteFlash(self.selected));
    }

    pub fn change_length(&mut self, steps: i32) {
        let g = self.grid_beats();
        let Some(n) = self.notes.get(self.selected).copied() else {
            return;
        };
        let d = (n.note.duration_beats + f64::from(steps) * g).max(g);
        if (d - n.note.duration_beats).abs() < 1e-9 {
            return;
        }
        self.checkpoint();
        self.notes[self.selected].note.duration_beats = d;
        self.info(format!("length {}", hummr_core::notation::format_length(d)));
        self.fx(FxRequest::NoteFlash(self.selected));
    }

    pub fn shift_time(&mut self, steps: i32) {
        let g = self.grid_beats();
        let Some(n) = self.notes.get(self.selected).copied() else {
            return;
        };
        let s = (n.note.start_beats + f64::from(steps) * g).max(0.0);
        if (s - n.note.start_beats).abs() < 1e-9 {
            return;
        }
        self.checkpoint();
        self.notes[self.selected].note.start_beats = s;
        let moved = self.notes[self.selected];
        self.notes
            .sort_by(|a, b| a.note.start_beats.total_cmp(&b.note.start_beats));
        self.selected = self.notes.iter().position(|n| *n == moved).unwrap_or(0);
        self.fx(FxRequest::NoteFlash(self.selected));
    }

    pub fn delete(&mut self) {
        if self.notes.is_empty() {
            return;
        }
        self.checkpoint();
        let n = self.notes.remove(self.selected);
        self.selected = self.selected.min(self.notes.len().saturating_sub(1));
        self.info(format!(
            "deleted {}",
            hummr_core::theory::note_name(n.note.pitch)
        ));
    }

    pub fn undo(&mut self) {
        match self.undo.pop() {
            Some(prev) => {
                self.notes = prev;
                self.selected = self.selected.min(self.notes.len().saturating_sub(1));
                self.player_stale = true;
                self.info("undone");
            }
            None => self.info("nothing to undo"),
        }
    }

    // ---- mixer ------------------------------------------------------------

    pub fn mixer_gain(&mut self, db: f32) {
        let s = &mut self.strips[self.strip_sel];
        s.gain_db = (s.gain_db + db).clamp(-48.0, 12.0);
        self.sync_mixer();
    }

    pub fn mixer_pan(&mut self, delta: f32) {
        let s = &mut self.strips[self.strip_sel];
        s.pan = (s.pan + delta).clamp(-1.0, 1.0);
        self.sync_mixer();
    }

    pub fn mixer_mute(&mut self) {
        let s = &mut self.strips[self.strip_sel];
        s.muted = !s.muted;
        self.sync_mixer();
    }

    pub fn mixer_solo(&mut self) {
        let s = &mut self.strips[self.strip_sel];
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
    }

    pub fn toggle_lower(&mut self) {
        self.lower = match self.lower {
            Lower::Tracker => Lower::Why,
            Lower::Why => Lower::Tracker,
        };
        self.fx(FxRequest::Lower);
    }

    pub fn open_picker(&mut self) {
        self.mode = Mode::Picker;
        self.picker_query.clear();
        self.refresh_picker();
        self.picker_sel = self
            .picker_results
            .iter()
            .position(|(p, _)| *p == self.program)
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
        self.program = program;
        self.player_stale = true;
        self.dirty = true;
        self.info(format!(
            "instrument: {} (saved into the MIDI file; playback uses a preview synth until SoundFont rendering lands)",
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
                if self.save_midi(None) || self.notes.is_empty() {
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
            Cmd::Record => self.start_recording(),
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
            Cmd::Retranscribe => self.start_transcription(),
            Cmd::Undo => self.undo(),
            Cmd::Help => self.mode = Mode::Help,
        }
    }

    fn apply_key(&mut self, choice: KeyChoice) {
        self.key_choice = choice;
        if self.take.is_some() {
            self.start_transcription();
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
                for n in &mut self.notes {
                    let p = k.snap(f32::from(n.note.pitch)).pitch;
                    moved += usize::from(p != n.note.pitch);
                    n.note.pitch = p;
                }
                self.key = Some(k);
                self.info(format!("key {k}: {moved} notes snapped"));
            }
            KeyChoice::Auto => {
                let weighted: Vec<(u8, f64)> = self
                    .notes
                    .iter()
                    .map(|n| (n.note.pitch, n.note.duration_beats))
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
        if self.take.is_some() {
            self.start_transcription();
            return;
        }
        match choice {
            TempoChoice::Fixed(bpm) => {
                self.tempo_bpm = bpm;
                self.dirty = true;
                self.player_stale = true;
                self.info(format!("tempo {bpm} BPM"));
            }
            TempoChoice::Auto => {
                self.warn("tempo detection needs a recording; give a BPM like :tempo 100")
            }
        }
    }

    fn apply_quantize(&mut self, grid: Option<Grid>) {
        self.quantize = grid;
        if self.take.is_some() {
            self.start_transcription();
            return;
        }
        if let Some(g) = grid {
            self.checkpoint();
            let step = g.beats();
            for n in &mut self.notes {
                let s = (n.note.start_beats / step).round() * step;
                let e = ((n.note.start_beats + n.note.duration_beats) / step).round() * step;
                n.note.start_beats = s;
                n.note.duration_beats = (e - s).max(step);
            }
            self.info(format!("quantized to {g}"));
        }
    }
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
    use hummr_core::theory::Scale;

    fn app_with_notes() -> App {
        let mut app = App::new(&Options {
            file: None,
            animations: true,
            mouse: false,
        });
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
        app.set_notes(notes);
        app
    }

    #[test]
    fn editing_and_undo() {
        let mut app = app_with_notes();
        app.select(1);
        app.transpose(1);
        assert_eq!(app.notes[1].note.pitch, 67);
        app.change_length(1);
        assert_eq!(app.notes[1].note.duration_beats, 1.5);
        app.delete();
        assert_eq!(app.notes.len(), 2);
        app.undo();
        app.undo();
        app.undo();
        assert_eq!(app.notes[1].note.pitch, 66);
        assert_eq!(app.notes[1].note.duration_beats, 1.0);
        assert!(app.dirty);
    }

    #[test]
    fn shift_time_keeps_selection_on_moved_note() {
        let mut app = app_with_notes();
        app.select(2);
        app.shift_time(-5);
        assert_eq!(app.notes[app.selected].note.pitch, 67);
        assert_eq!(app.notes[app.selected].note.start_beats, 0.0);
    }

    #[test]
    fn key_command_snaps_without_a_take() {
        let mut app = app_with_notes();
        app.execute(command::parse(":key C major").unwrap());
        assert_eq!(app.key, Some(Key::new(0, Scale::Major)));
        let pitches: Vec<u8> = app.notes.iter().map(|n| n.note.pitch).collect();
        assert_eq!(pitches, vec![60, 65, 67]);
    }

    #[test]
    fn tempo_and_quit_guard() {
        let mut app = app_with_notes();
        app.execute(Cmd::Tempo("90".into()));
        assert_eq!(app.tempo_bpm, 90.0);
        app.execute(Cmd::Quit { force: false });
        assert!(!app.quit, "dirty buffer must not quit");
        app.execute(Cmd::Quit { force: true });
        assert!(app.quit);
    }

    #[test]
    fn picker_and_instrument() {
        let mut app = app_with_notes();
        app.open_picker();
        app.picker_query = "flute".into();
        app.refresh_picker();
        app.picker_choose();
        assert_eq!(app.program, 73);
        app.execute(Cmd::Instrument("cello".into()));
        assert_eq!(app.program, 42);
    }

    #[test]
    fn save_and_reopen_midi() {
        let mut app = app_with_notes();
        let path = std::env::temp_dir().join(format!("hummr-tui-{}.mid", std::process::id()));
        app.execute(Cmd::Write(Some(path.to_string_lossy().into_owned())));
        assert!(!app.dirty);
        let mut other = App::new(&Options {
            file: None,
            animations: false,
            mouse: false,
        });
        other.open(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(other.notes.len(), 3);
        assert_eq!(other.program, 40);
    }

    #[test]
    fn wav_export_renders_preview() {
        let mut app = app_with_notes();
        let path = std::env::temp_dir().join(format!("hummr-tui-{}.wav", std::process::id()));
        app.execute(Cmd::Export(path.to_string_lossy().into_owned()));
        let audio = wav::read_mono(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(audio.duration_seconds() > 1.5);
        assert!(audio.peak() > 0.05);
    }

    #[test]
    fn transcribes_an_opened_take() {
        use hummr_core::synthetic::{self, HumParams, TWINKLE};
        let tones = synthetic::melody(&TWINKLE[..7], 100.0, 0.3, 0.0, 1);
        let audio = AudioBuffer::new(synthetic::hum(&tones, &HumParams::new(44_100), 1), 44_100);
        let path = std::env::temp_dir().join(format!("hummr-tui-take-{}.wav", std::process::id()));
        wav::write_mono(&path, &audio, wav::WavFormat::Pcm16).unwrap();
        let mut app = App::new(&Options {
            file: None,
            animations: true,
            mouse: false,
        });
        app.open(&path);
        std::fs::remove_file(&path).ok();
        for _ in 0..500 {
            app.tick(Duration::from_millis(10));
            if app.busy.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let pitches: Vec<u8> = app.notes.iter().map(|n| n.note.pitch).collect();
        assert_eq!(pitches, vec![60, 60, 67, 67, 69, 69, 67]);
        assert!(app.origin_seconds > 0.2);
        assert!(app.fx_requests.contains(&FxRequest::NotesIn));
    }
}
