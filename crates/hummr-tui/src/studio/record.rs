//! Recording onto the timeline: a one-bar count-in with the song playing
//! along, then everything the microphone hears becomes a new clip at the
//! playhead.

use hummr_core::AudioBuffer;
use hummr_core::audio::{Metronome, Recorder, mic_permission_hint};
use hummr_core::project::Song;

use super::{Mode, Studio};
use crate::app::Level;

/// A recording in progress.
pub struct Recording {
    recorder: Recorder,
    pub track: usize,
    /// Song beat where the take starts (after the count-in).
    pub at_beat: f64,
    /// Count-in length in beats (0 without speakers).
    pub count_in: f64,
    /// Recorder time (s) when the count-in started playing.
    play_offset: Option<f64>,
    /// Song beat under the record head.
    pub beat: f64,
    /// Beats of count-in left, while counting in.
    pub count_left: Option<u32>,
    pub level: f32,
    silence_warned: bool,
}

/// Where a take starts inside the raw recording: after the count-in.
pub fn take_origin(play_offset: Option<f64>, count_in_beats: f64, song: &Song) -> f64 {
    play_offset.map_or(0.0, |o| o + count_in_beats * song.seconds_per_beat())
}

/// The part of a raw recording after `origin` seconds.
pub fn trim_take(raw: &AudioBuffer, origin: f64) -> AudioBuffer {
    let skip = ((origin * f64::from(raw.sample_rate)).round() as usize).min(raw.samples.len());
    AudioBuffer::new(raw.samples[skip..].to_vec(), raw.sample_rate)
}

/// A microphone error as a plain sentence with what to do next.
pub fn mic_error(e: &str) -> String {
    let hint = mic_permission_hint();
    let has_hint = e.contains(hint);
    let mut text = e.trim().trim_end_matches('.').to_string();
    if let Some(first) = text.get(..1) {
        text = first.to_uppercase() + &text[1..];
    }
    if has_hint {
        format!("Can't record: {text}.")
    } else {
        format!("Can't record: {text}. {hint}")
    }
}

impl Studio {
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// `r`: starts recording on the selected track at the playhead.
    pub fn start_recording(&mut self) {
        if self.playing {
            self.engine.stop();
            self.playing = false;
        }
        let recorder = match Recorder::start() {
            Ok(r) => r,
            Err(e) => {
                self.say(mic_error(&e.to_string()), Level::Error);
                return;
            }
        };
        let track = match self.current_track() {
            Some(t) => t,
            None => self.new_track(None),
        };
        let song = self.project.song.clone();
        let at = song.snap_beat(self.playhead).max(0.0);
        let bar = song.bar_beats();
        let mut count_in = 0.0;
        let mut play_offset = None;
        if let Some(p) = self.engine.player() {
            let rate = p.sample_rate();
            let from = at - bar;
            let mut m = Metronome::new(song.bpm, rate).starting_at_beat(from);
            m.beats_per_bar = u32::from(song.time_sig.top);
            p.set_metronome(Some(m));
            let samples =
                |beats: f64| (song.beats_to_seconds(beats) * f64::from(rate)).round() as usize;
            if from >= 0.0 {
                p.play_after(samples(from), 0);
            } else {
                p.play_after(0, samples(-from));
            }
            count_in = bar;
            play_offset = Some(recorder.seconds());
        }
        let who = recorder.device_name().to_string();
        self.recording = Some(Recording {
            recorder,
            track,
            at_beat: at,
            count_in,
            play_offset,
            beat: at - count_in,
            count_left: (count_in > 0.0).then_some(count_in as u32),
            level: 0.0,
            silence_warned: false,
        });
        self.mode = Mode::Recording;
        let how = if count_in > 0.0 {
            "after the one-bar count-in. Use headphones so the song doesn't leak into the mic."
        } else {
            "now (no speakers found, so there is no count-in)."
        };
        self.say(
            format!("Recording from {who} {how} r or space to stop, Esc to cancel."),
            Level::Info,
        );
    }

    /// Per-frame recording updates (record head, count-in, mic level).
    pub(super) fn tick_recording(&mut self) {
        let bps = self.song().bpm / 60.0;
        let Some(r) = &mut self.recording else { return };
        let secs = r.recorder.seconds();
        r.level = r.recorder.take_peak().max(r.level * 0.8);
        match r.play_offset {
            Some(off) => {
                let into = (secs - off).max(0.0) * bps;
                r.count_left = (into < r.count_in).then(|| (r.count_in - into).ceil() as u32);
                r.beat = r.at_beat - r.count_in + into;
            }
            None => r.beat = r.at_beat + secs * bps,
        }
        let silent = !r.recorder.heard_anything() && secs > 1.5 && !r.silence_warned;
        if silent {
            r.silence_warned = true;
        }
        let err = r.recorder.error();
        if silent {
            self.say(
                format!(
                    "The microphone is sending pure silence. {}",
                    mic_permission_hint()
                ),
                Level::Error,
            );
        }
        if let Some(e) = err {
            self.say(format!("Microphone problem: {e}"), Level::Error);
        }
    }

    /// Stops recording; `keep` adds the take as a clip.
    pub fn stop_recording(&mut self, keep: bool) {
        let Some(r) = self.recording.take() else {
            return;
        };
        self.mode = Mode::Normal;
        if let Some(p) = self.engine.player() {
            p.stop();
            p.set_metronome(None);
        }
        if !keep {
            self.say("Recording cancelled.", Level::Info);
            return;
        }
        let origin = take_origin(r.play_offset, r.count_in, self.song());
        match r.recorder.stop() {
            Ok(raw) => {
                if raw.duration_seconds() < origin + 0.3 {
                    self.say(
                        "Stopped before the count-in finished. Press r to try again.",
                        Level::Warn,
                    );
                    return;
                }
                let take = trim_take(&raw, origin);
                self.add_take(take, r.track, r.at_beat);
            }
            Err(e) => self.say(format!("{e}"), Level::Error),
        }
    }

    /// Adds a recorded take as a clip. It was recorded against the song's
    /// click, so its tempo is the song's: nothing to detect.
    pub fn add_take(&mut self, take: AudioBuffer, track: usize, at_beat: f64) -> u64 {
        let n = self
            .song()
            .sources
            .iter()
            .filter(|s| s.name.starts_with("take"))
            .count()
            + 1;
        let name = format!("take {n}");
        let bpm = self.song().bpm;
        let id = self.add_clip_from_audio(&name, take, Some(bpm), Some(track), at_beat);
        self.say(
            format!("Added \"{name}\". Press Enter on it to turn it into an instrument or drums."),
            Level::Info,
        );
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_in_is_trimmed_from_the_take() {
        let mut song = Song::new("t");
        song.bpm = 120.0;
        // Count-in started 0.2 s into the recording; one 4-beat bar = 2 s.
        let origin = take_origin(Some(0.2), 4.0, &song);
        assert!((origin - 2.2).abs() < 1e-12);
        let raw = AudioBuffer::new((0..48_000 * 3).map(|i| i as f32).collect(), 48_000);
        let take = trim_take(&raw, origin);
        assert_eq!(take.samples.len(), 48_000 * 3 - 105_600);
        assert_eq!(take.samples[0], 105_600.0);
        assert_eq!(take_origin(None, 4.0, &song), 0.0);
    }
}
