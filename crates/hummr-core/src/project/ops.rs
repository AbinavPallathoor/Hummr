//! Clip operations that need analysis: detecting a clip's tempo and
//! matching it to the song.

use super::{Clip, ClipKind, Stretch};
use crate::AudioBuffer;
use crate::beat::{BeatEstimate, estimate};
use crate::project::render::region;
use crate::slice::slice_clip;

/// How a clip with its own tempo is fitted to the song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MatchMode {
    /// Cut at the hits and place each on the song's beats (no stretching);
    /// hits move `strength` of the way to the `grid` (beats).
    CutAndSnap { grid: f64, strength: f64 },
    /// Time-stretch the whole clip to the song tempo.
    Stretch,
    /// Leave the sound alone.
    Keep,
}

/// Tempo analysis of a clip's region. Times in the result are relative to
/// the region start.
pub fn analyze(clip: &Clip, audio: &AudioBuffer) -> BeatEstimate {
    let r = region(audio, clip.source_start, clip.source_len);
    estimate(&r, audio.sample_rate)
}

/// Whether a clip is made of hits (beatbox, taps, drums) rather than
/// sustained sound (humming, singing, pads): at least two clear hits per
/// second and a fairly steady pulse.
pub fn is_rhythmic(est: &BeatEstimate, seconds: f64) -> bool {
    seconds > 0.0 && est.onsets.len() as f64 / seconds >= 2.0 && est.confidence >= 0.3
}

/// The mode a new clip gets by default: rhythmic clips are cut and snapped
/// (at full strength to 1/8 notes), everything else keeps its sound.
pub fn default_mode(est: &BeatEstimate, seconds: f64) -> MatchMode {
    if is_rhythmic(est, seconds) {
        MatchMode::CutAndSnap {
            grid: 0.5,
            strength: 1.0,
        }
    } else {
        MatchMode::Keep
    }
}

/// Fits `clip` (whose region was analysed into `est`) to a song at
/// `song_bpm`.
pub fn match_tempo(
    clip: &Clip,
    audio: &AudioBuffer,
    est: &BeatEstimate,
    song_bpm: f64,
    mode: MatchMode,
) -> Clip {
    match mode {
        MatchMode::CutAndSnap { grid, strength } => {
            let onsets: Vec<f64> = est
                .onsets
                .iter()
                .map(|o| clip.source_start + o.time)
                .collect();
            slice_clip(
                clip,
                audio.sample_rate,
                &onsets,
                est.bpm,
                clip.source_start + est.phase,
                grid,
                strength,
                song_bpm,
            )
        }
        MatchMode::Stretch => {
            let mut c = clip.clone();
            if matches!(c.kind, ClipKind::Sliced { .. }) {
                c.kind = ClipKind::Audio;
            }
            c.stretch = Stretch::Follow { bpm: est.bpm };
            c
        }
        MatchMode::Keep => clip.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onset::detect_onsets;
    use crate::project::render::render_clip;
    use crate::project::{PROJECT_RATE, Song, SourceBank};
    use crate::synthetic::{HumParams, TWINKLE, drum_groove, hum, melody};
    use std::sync::Arc;

    fn setup(audio: Vec<f32>) -> (AudioBuffer, SourceBank, Clip) {
        let a = AudioBuffer::new(audio, PROJECT_RATE);
        let mut bank = SourceBank::default();
        bank.insert("s", Arc::new(a.clone()));
        let dur = a.duration_seconds();
        (a, bank, Clip::audio(1, "c", "s", 0.0, dur, 0.0))
    }

    #[test]
    fn beatbox_loop_is_rhythmic_and_cut_to_the_song() {
        let (loop_audio, _) = drum_groove(100.0, 4, 1, 0.1, 0.01, 2, PROJECT_RATE);
        let (audio, bank, clip) = setup(loop_audio);
        let est = analyze(&clip, &audio);
        assert!(is_rhythmic(&est, clip.source_len), "{}", est.describe());
        let mode = default_mode(&est, clip.source_len);
        assert!(matches!(mode, MatchMode::CutAndSnap { .. }));
        let matched = match_tempo(&clip, &audio, &est, 120.0, mode);
        let mut song = Song::new("t");
        song.bpm = 120.0;
        let out = render_clip(&song, &matched, &bank);
        // The loop's beats (16, plus about one beat of tail) stay beats.
        let beats = matched.len_beats(120.0);
        assert!((16.0..=18.0).contains(&beats), "{beats} beats");
        for h in detect_onsets(&out, PROJECT_RATE) {
            let d = (h.time / 0.25).rem_euclid(1.0);
            assert!(d.min(1.0 - d) * 0.25 < 0.005, "hit at {}", h.time);
        }
    }

    #[test]
    fn humming_keeps_its_sound_by_default_and_can_stretch() {
        let tones = melody(&TWINKLE, 90.0, 0.2, 0.0, 1);
        let (audio, bank, clip) = setup(hum(&tones, &HumParams::new(PROJECT_RATE), 1));
        let est = analyze(&clip, &audio);
        assert!(!is_rhythmic(&est, clip.source_len), "{}", est.describe());
        assert_eq!(default_mode(&est, clip.source_len), MatchMode::Keep);

        let mut fake = est.clone();
        fake.bpm = 90.0;
        let stretched = match_tempo(&clip, &audio, &fake, 120.0, MatchMode::Stretch);
        let mut song = Song::new("t");
        song.bpm = 120.0;
        let out = render_clip(&song, &stretched, &bank);
        let expect = audio.samples.len() as f64 * 90.0 / 120.0;
        assert!(
            (out.len() as f64 / expect - 1.0).abs() < 0.01,
            "{} vs {expect}",
            out.len()
        );
    }
}
