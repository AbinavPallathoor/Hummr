//! Playback for the timeline: each track is rendered to stereo on a
//! background thread and handed to the existing [`Player`]. A track is only
//! re-rendered when something that changes its sound changes (volume, pan,
//! mute and solo are applied live), and inside a track only the clips that
//! changed are rendered again (see [`ClipCache`]), so editing never stops
//! playback or the UI.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use hummr_core::audio::{PlayTrack, Player};
use hummr_core::dsp::db_to_amp;
use hummr_core::project::render::{ClipCache, render_track_cached};
use hummr_core::project::{Clip, PROJECT_RATE, Song, SourceBank, Track};
use hummr_core::resample::resample;

/// What decides how a track sounds before the mixer.
#[derive(Clone, Debug, PartialEq)]
struct TrackKey {
    track: Track,
    bpm: f64,
}

impl TrackKey {
    fn of(song: &Song, i: usize) -> Self {
        let mut track = song.tracks[i].clone();
        track.volume_db = 0.0;
        track.pan = 0.0;
        track.mute = false;
        track.solo = false;
        Self {
            track,
            bpm: song.bpm,
        }
    }
}

struct Job {
    index: usize,
    key: TrackKey,
    song: Arc<Song>,
    bank: SourceBank,
    rate: u32,
}

struct Done {
    index: usize,
    key: TrackKey,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Rendered clips (for waveforms): clip, tempo, mono audio.
    clips: Vec<(Clip, f64, Arc<Vec<f32>>)>,
}

#[derive(Default)]
struct Slot {
    key: Option<TrackKey>,
    ready: bool,
}

/// Plays the song. Without a sound device (or in tests) it only keeps time.
pub struct Engine {
    player: Option<Player>,
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    slots: Vec<Slot>,
    rate: u32,
    /// Why there is no sound, if there isn't.
    pub problem: Option<String>,
    /// Position (samples at `rate`) and state when there is no player.
    sim_pos: f64,
    sim_playing: bool,
    sim_end: f64,
}

/// Work done on the render thread, with one clip cache per track id.
fn worker(jobs: Receiver<Job>, done: Sender<Done>) {
    let mut caches: HashMap<u64, ClipCache> = HashMap::new();
    while let Ok(mut job) = jobs.recv() {
        // Skip work that a newer job for the same track replaces.
        let mut later = Vec::new();
        while let Ok(next) = jobs.try_recv() {
            later.push(next);
        }
        let mut queue = Vec::new();
        for j in std::iter::once(job).chain(later) {
            queue.retain(|q: &Job| q.index != j.index);
            queue.push(j);
        }
        for j in queue {
            job = j;
            let song = &job.song;
            let id = song.tracks[job.index].id;
            let cache = caches.entry(id).or_default();
            let stereo = render_track_cached(song, job.index, &job.bank, cache);
            let clips = song.tracks[job.index]
                .clips
                .iter()
                .map(|c| (c.clone(), song.bpm, cache.get(song, c, &job.bank)))
                .collect();
            cache.sweep();
            let (left, right) = if job.rate == PROJECT_RATE {
                (stereo.left, stereo.right)
            } else {
                (
                    resample(&stereo.left, PROJECT_RATE, job.rate),
                    resample(&stereo.right, PROJECT_RATE, job.rate),
                )
            };
            let msg = Done {
                index: job.index,
                key: job.key,
                left,
                right,
                clips,
            };
            if done.send(msg).is_err() {
                return;
            }
        }
    }
}

impl Engine {
    /// Opens the sound device and starts the render thread.
    pub fn open() -> Self {
        match Player::open() {
            Ok(player) => {
                let rate = player.sample_rate();
                let (jtx, jrx) = channel();
                let (dtx, drx) = channel();
                thread::Builder::new()
                    .name("hummr-render".into())
                    .spawn(move || worker(jrx, dtx))
                    .ok();
                Self {
                    player: Some(player),
                    jobs: Some(jtx),
                    done: Some(drx),
                    ..Self::headless()
                }
                .with_rate(rate)
            }
            Err(e) => Self {
                problem: Some(format!("{e}. You can still edit; playback is off.")),
                ..Self::headless()
            },
        }
    }

    /// Renders in the background but plays nothing (for tests).
    pub fn render_only() -> Self {
        let (jtx, jrx) = channel();
        let (dtx, drx) = channel();
        thread::spawn(move || worker(jrx, dtx));
        Self {
            jobs: Some(jtx),
            done: Some(drx),
            ..Self::headless()
        }
    }

    /// No sound: keeps time only. Used by tests.
    pub fn headless() -> Self {
        Self {
            player: None,
            jobs: None,
            done: None,
            slots: Vec::new(),
            rate: PROJECT_RATE,
            problem: None,
            sim_pos: 0.0,
            sim_playing: false,
            sim_end: 0.0,
        }
    }

    fn with_rate(mut self, rate: u32) -> Self {
        self.rate = rate;
        self
    }

    pub fn has_sound(&self) -> bool {
        self.player.is_some()
    }

    /// Whether every track's sound is up to date.
    pub fn is_ready(&self) -> bool {
        self.jobs.is_none() || self.slots.iter().all(|s| s.ready)
    }

    fn params(song: &Song, i: usize) -> (f32, f32, bool) {
        let t = &song.tracks[i];
        (
            db_to_amp(t.volume_db as f32),
            t.pan as f32,
            !song.audible(i),
        )
    }

    /// Brings the player up to date with the song: re-renders changed
    /// tracks in the background, applies mixer settings at once.
    pub fn sync(&mut self, song: &Song, bank: &SourceBank) {
        let n = song.tracks.len();
        if self.slots.len() > n {
            self.slots.truncate(n);
            if let Some(p) = &self.player {
                p.truncate_tracks(n);
            }
        }
        self.slots.resize_with(n, Slot::default);
        let Some(jobs) = &self.jobs else {
            return;
        };
        let mut shared: Option<Arc<Song>> = None;
        for i in 0..n {
            let key = TrackKey::of(song, i);
            if self.slots[i].key.as_ref() != Some(&key) {
                let song_arc = shared.get_or_insert_with(|| Arc::new(song.clone()));
                let _ = jobs.send(Job {
                    index: i,
                    key: key.clone(),
                    song: Arc::clone(song_arc),
                    bank: bank.clone(),
                    rate: self.rate,
                });
                self.slots[i] = Slot {
                    key: Some(key),
                    ready: false,
                };
            }
            if let Some(p) = &self.player {
                let (g, pan, muted) = Self::params(song, i);
                p.set_track_params(i, g, pan, muted);
            }
        }
    }

    /// Takes finished renders; returns rendered clips for the waveforms.
    pub fn poll(&mut self, song: &Song) -> Vec<(Clip, f64, Arc<Vec<f32>>)> {
        let mut clips = Vec::new();
        let Some(done) = &self.done else {
            return clips;
        };
        while let Ok(d) = done.try_recv() {
            let Some(slot) = self.slots.get_mut(d.index) else {
                continue;
            };
            if slot.key.as_ref() != Some(&d.key) {
                continue;
            }
            slot.ready = true;
            let (gain, pan, muted) = Self::params(song, d.index);
            if let Some(p) = &self.player {
                p.replace_track(
                    d.index,
                    PlayTrack {
                        samples: d.left,
                        gain,
                        pan,
                        muted,
                        right: Some(d.right),
                    },
                );
            }
            clips.extend(d.clips);
        }
        clips
    }

    /// Starts playing at `beat`.
    pub fn play(&mut self, song: &Song, beat: f64) {
        let pos = song.beats_to_seconds(beat.max(0.0)) * f64::from(self.rate);
        match &self.player {
            Some(p) => p.play_from(pos as usize),
            None => {
                self.sim_pos = pos;
                self.sim_playing = true;
                self.sim_end = song.beats_to_seconds(song.end_beats()) * f64::from(self.rate);
            }
        }
    }

    pub fn stop(&mut self) {
        match &self.player {
            Some(p) => p.stop(),
            None => self.sim_playing = false,
        }
    }

    pub fn is_playing(&self) -> bool {
        match &self.player {
            Some(p) => p.is_playing(),
            None => self.sim_playing,
        }
    }

    /// Advances the simulated clock (no-op with a real player).
    pub fn advance(&mut self, seconds: f64) {
        if self.player.is_none() && self.sim_playing {
            self.sim_pos += seconds * f64::from(self.rate);
            if self.sim_pos >= self.sim_end {
                self.sim_pos = self.sim_end;
                self.sim_playing = false;
            }
        }
    }

    /// The play position in beats.
    pub fn position_beats(&self, song: &Song) -> f64 {
        let pos = match &self.player {
            Some(p) => p.position() as f64,
            None => self.sim_pos,
        };
        song.seconds_to_beats(pos / f64::from(self.rate))
    }

    /// Per-track and master peak levels since the last call.
    pub fn take_peaks(&self) -> (Vec<f32>, f32) {
        self.player
            .as_ref()
            .map(Player::take_peaks)
            .unwrap_or_default()
    }

    /// The player, for recording (metronome, count-in).
    pub fn player(&self) -> Option<&Player> {
        self.player.as_ref()
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hummr_core::project::render::render_clip;
    use std::time::{Duration, Instant};

    #[test]
    fn renders_changed_tracks_in_the_background() {
        let p = crate::studio::fixture::mockup_project();
        let mut song = p.song.clone();
        let mut e = Engine::render_only();
        e.sync(&song, &p.sources);
        assert!(!e.is_ready());
        let mut got = Vec::new();
        let t0 = Instant::now();
        while !e.is_ready() && t0.elapsed() < Duration::from_secs(120) {
            got.extend(e.poll(&song));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(e.is_ready());
        let n: usize = song.tracks.iter().map(|t| t.clips.len()).sum();
        assert_eq!(got.len(), n);
        for (clip, _, audio) in &got {
            assert_eq!(
                **audio,
                render_clip(&song, clip, &p.sources),
                "{}",
                clip.name
            );
        }
        // Volume, mute and solo don't re-render; moving a clip re-renders
        // only its track.
        song.tracks[0].volume_db = -6.0;
        song.tracks[1].mute = true;
        e.sync(&song, &p.sources);
        assert!(e.is_ready());
        let id = song.tracks[3].clips[0].id;
        song.move_clip(id, 3, 5.0);
        e.sync(&song, &p.sources);
        assert!(!e.is_ready());
        assert_eq!(e.slots.iter().filter(|s| !s.ready).count(), 1);
    }
}
