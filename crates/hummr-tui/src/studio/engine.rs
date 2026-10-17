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

/// A clip to loop on its own (the inspector's preview).
struct PreviewJob {
    generation: u64,
    song: Arc<Song>,
    bank: SourceBank,
    rate: u32,
}

enum Work {
    Track(Job),
    Preview(PreviewJob),
}

struct PreviewDone {
    generation: u64,
    left: Vec<f32>,
    right: Vec<f32>,
}

enum Out {
    Track(Done),
    Preview(PreviewDone),
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
    jobs: Option<Sender<Work>>,
    done: Option<Receiver<Out>>,
    slots: Vec<Slot>,
    rate: u32,
    /// Why there is no sound, if there isn't.
    pub problem: Option<String>,
    /// Position (samples at `rate`) and state when there is no player.
    sim_pos: f64,
    sim_playing: bool,
    sim_end: f64,
    /// While previewing a clip, the song's tracks are put aside.
    previewing: bool,
    preview_generation: u64,
    preview_ready: bool,
}

fn to_rate(stereo: hummr_core::project::render::Stereo, rate: u32) -> (Vec<f32>, Vec<f32>) {
    if rate == PROJECT_RATE {
        (stereo.left, stereo.right)
    } else {
        (
            resample(&stereo.left, PROJECT_RATE, rate),
            resample(&stereo.right, PROJECT_RATE, rate),
        )
    }
}

/// Work done on the render thread, with one clip cache per track id.
fn worker(jobs: Receiver<Work>, done: Sender<Out>) {
    let mut caches: HashMap<u64, ClipCache> = HashMap::new();
    let mut preview_cache = ClipCache::default();
    while let Ok(first) = jobs.recv() {
        // Skip work that newer work replaces (same track, or any preview).
        let mut queue: Vec<Work> = Vec::new();
        for w in std::iter::once(first).chain(std::iter::from_fn(|| jobs.try_recv().ok())) {
            queue.retain(|q| match (q, &w) {
                (Work::Track(a), Work::Track(b)) => a.index != b.index,
                (Work::Preview(_), Work::Preview(_)) => false,
                _ => true,
            });
            queue.push(w);
        }
        for w in queue {
            let msg = match w {
                Work::Track(job) => {
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
                    let (left, right) = to_rate(stereo, job.rate);
                    Out::Track(Done {
                        index: job.index,
                        key: job.key,
                        left,
                        right,
                        clips,
                    })
                }
                Work::Preview(job) => {
                    let stereo = render_track_cached(&job.song, 0, &job.bank, &mut preview_cache);
                    preview_cache.sweep();
                    let (left, right) = to_rate(stereo, job.rate);
                    Out::Preview(PreviewDone {
                        generation: job.generation,
                        left,
                        right,
                    })
                }
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
            previewing: false,
            preview_generation: 0,
            preview_ready: false,
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
        if self.previewing {
            // Caught up when the preview ends.
            return;
        }
        let Some(jobs) = &self.jobs else {
            return;
        };
        let mut shared: Option<Arc<Song>> = None;
        for i in 0..n {
            let key = TrackKey::of(song, i);
            if self.slots[i].key.as_ref() != Some(&key) {
                let song_arc = shared.get_or_insert_with(|| Arc::new(song.clone()));
                let _ = jobs.send(Work::Track(Job {
                    index: i,
                    key: key.clone(),
                    song: Arc::clone(song_arc),
                    bank: bank.clone(),
                    rate: self.rate,
                }));
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
        while let Ok(out) = done.try_recv() {
            let d = match out {
                Out::Track(d) => d,
                Out::Preview(p) => {
                    if self.previewing && p.generation == self.preview_generation {
                        self.preview_ready = true;
                        if let Some(player) = &self.player {
                            let len = p.left.len().max(1);
                            let pos = if player.is_playing() {
                                player.position() % len
                            } else {
                                0
                            };
                            player.set_tracks(vec![PlayTrack {
                                samples: p.left,
                                gain: 1.0,
                                pan: 0.0,
                                muted: false,
                                right: Some(p.right),
                            }]);
                            player.set_looping(true);
                            player.play_from(pos);
                        }
                    }
                    continue;
                }
            };
            if self.previewing {
                // The song's tracks are rendered again after the preview.
                continue;
            }
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

    /// Loops `clip` on its own (rendered in the background). Changes to
    /// the clip call this again; the loop carries on from where it was.
    pub fn preview(&mut self, song: &Song, clip: &Clip, bank: &SourceBank) {
        if self.jobs.is_none() {
            return;
        }
        let Some((t, _)) = song.find_clip(clip.id) else {
            return;
        };
        let mut solo = song.clone();
        let mut track = song.tracks[t].clone();
        track.fx = Default::default();
        let mut c = clip.clone();
        c.start_beat = 0.0;
        track.clips = vec![c];
        solo.tracks = vec![track];
        if !self.previewing {
            self.stop();
        }
        self.previewing = true;
        self.preview_ready = false;
        self.preview_generation += 1;
        let Some(jobs) = &self.jobs else { return };
        let _ = jobs.send(Work::Preview(PreviewJob {
            generation: self.preview_generation,
            song: Arc::new(solo),
            bank: bank.clone(),
            rate: self.rate,
        }));
    }

    /// Stops the preview; the song's tracks are rendered again at the next
    /// [`Engine::sync`].
    pub fn end_preview(&mut self) {
        if !self.previewing {
            return;
        }
        self.previewing = false;
        self.preview_ready = false;
        if let Some(p) = &self.player {
            p.stop();
            p.set_looping(false);
            p.truncate_tracks(0);
        }
        for slot in &mut self.slots {
            *slot = Slot::default();
        }
    }

    pub fn is_previewing(&self) -> bool {
        self.previewing
    }

    /// Whether the preview loop has been rendered.
    pub fn preview_ready(&self) -> bool {
        self.preview_ready
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

    #[test]
    fn preview_loops_one_clip_and_hands_back_to_the_song() {
        let p = crate::studio::fixture::mockup_project();
        let song = p.song.clone();
        let mut e = Engine::render_only();
        e.sync(&song, &p.sources);
        let clip = song.tracks[2].clips[0].clone();
        e.preview(&song, &clip, &p.sources);
        assert!(e.is_previewing());
        let t0 = Instant::now();
        while !e.preview_ready() && t0.elapsed() < Duration::from_secs(120) {
            // Track renders finishing now are dropped, not played.
            assert!(e.poll(&song).is_empty());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(e.preview_ready());
        // A newer change replaces the older preview.
        let mut louder = clip.clone();
        louder.gain_db = 6.0;
        e.preview(&song, &louder, &p.sources);
        assert!(!e.preview_ready());
        e.end_preview();
        assert!(!e.is_previewing());
        e.sync(&song, &p.sources);
        assert!(!e.is_ready(), "every track renders again after a preview");
        let t0 = Instant::now();
        while !e.is_ready() && t0.elapsed() < Duration::from_secs(120) {
            e.poll(&song);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(e.is_ready());
    }
}
