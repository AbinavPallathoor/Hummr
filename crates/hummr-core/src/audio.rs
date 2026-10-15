//! Microphone capture and speaker playback via `cpal`.
//!
//! Errors are turned into messages that say what to do, including the
//! platform-specific microphone-permission steps. On macOS a terminal without
//! microphone permission receives silence rather than an error, so callers
//! should also check [`Recorder::heard_anything`].

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::preview::{pan_gains, soft_clip};
use crate::{AudioBuffer, Error, Result};

/// How to grant microphone access on this platform.
pub fn mic_permission_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "On macOS, allow your terminal app to use the microphone: System Settings → Privacy & Security → \
         Microphone, switch it on for Terminal (or iTerm, Ghostty, VS Code…), then restart the terminal."
    } else if cfg!(windows) {
        "On Windows, open Settings → Privacy & security → Microphone and turn on \
         'Let desktop apps access your microphone'."
    } else {
        "On Linux, check the microphone is not muted (alsamixer or pavucontrol) and that your user may \
         access audio devices (e.g. is in the 'audio' group)."
    }
}

fn map_err(context: &str, e: cpal::Error) -> Error {
    use cpal::ErrorKind as K;
    let detail = match e.kind() {
        K::PermissionDenied => format!("permission denied. {}", mic_permission_hint()),
        K::DeviceNotAvailable => "the device is not available (unplugged or disabled?)".to_string(),
        K::HostUnavailable => "the system audio service is not running".to_string(),
        K::DeviceBusy => {
            "the device is busy; close other apps that are using it and try again".to_string()
        }
        K::UnsupportedConfig => "the device does not support the requested format".to_string(),
        _ => e.to_string(),
    };
    Error::Audio(format!("{context}: {detail}"))
}

/// An `f32` stored in an `AtomicU32`, for lock-free meters.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    /// Stores the max of the current and new value.
    pub fn fetch_max(&self, v: f32) {
        let mut cur = self.0.load(Ordering::Relaxed);
        while f32::from_bits(cur) < v {
            match self.0.compare_exchange_weak(
                cur,
                v.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => cur = x,
            }
        }
    }
    /// Returns the value and resets it to zero.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

fn log_stream_error(err: cpal::Error, slot: &Mutex<Option<String>>) {
    use cpal::ErrorKind as K;
    if matches!(err.kind(), K::Xrun | K::RealtimeDenied | K::DeviceChanged) {
        return;
    }
    if let Ok(mut s) = slot.lock() {
        *s = Some(err.to_string());
    }
}

#[derive(Default)]
struct RecShared {
    samples: Mutex<Vec<f32>>,
    peak: AtomicF32,
    heard: AtomicBool,
    error: Mutex<Option<String>>,
}

/// Captures mono audio from the default input device until dropped/stopped.
pub struct Recorder {
    stream: cpal::Stream,
    shared: Arc<RecShared>,
    sample_rate: u32,
    device_name: String,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| {
            Error::Audio(format!(
                "no microphone found. Plug one in or choose a default input device in your system sound \
                 settings. {}",
                mic_permission_hint()
            ))
        })?;
        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "default input".to_string());
        let supported = device
            .default_input_config()
            .map_err(|e| map_err("could not open the microphone", e))?;
        let sample_rate = supported.sample_rate();
        let channels = usize::from(supported.channels()).max(1);
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let shared = Arc::new(RecShared::default());

        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&device, config, channels, &shared),
            SampleFormat::I16 => build_input::<i16>(&device, config, channels, &shared),
            SampleFormat::I32 => build_input::<i32>(&device, config, channels, &shared),
            SampleFormat::U16 => build_input::<u16>(&device, config, channels, &shared),
            SampleFormat::I8 => build_input::<i8>(&device, config, channels, &shared),
            SampleFormat::U8 => build_input::<u8>(&device, config, channels, &shared),
            SampleFormat::F64 => build_input::<f64>(&device, config, channels, &shared),
            other => {
                return Err(Error::Audio(format!(
                    "the microphone uses sample format {other}, which is not supported yet"
                )));
            }
        }?;
        stream
            .play()
            .map_err(|e| map_err("could not start recording", e))?;
        Ok(Self {
            stream,
            shared,
            sample_rate,
            device_name,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn seconds(&self) -> f64 {
        self.shared.samples.lock().map_or(0, |s| s.len()) as f64 / f64::from(self.sample_rate)
    }

    /// Peak level since the last call (linear).
    pub fn take_peak(&self) -> f32 {
        self.shared.peak.take()
    }

    /// Whether any non-zero sample has arrived. All-zero input usually means
    /// the OS is withholding microphone access.
    pub fn heard_anything(&self) -> bool {
        self.shared.heard.load(Ordering::Relaxed)
    }

    /// A stream error reported by the device, if any.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }

    /// The most recent `n` samples (fewer at the start).
    pub fn tail(&self, n: usize) -> Vec<f32> {
        self.shared
            .samples
            .lock()
            .map(|s| s[s.len().saturating_sub(n)..].to_vec())
            .unwrap_or_default()
    }

    /// Stops capture and returns the take. Fails with a permission hint if
    /// the take is pure digital silence.
    pub fn stop(self) -> Result<AudioBuffer> {
        let _ = self.stream.pause();
        drop(self.stream);
        let samples = std::mem::take(
            &mut *self
                .shared
                .samples
                .lock()
                .map_err(|_| Error::Audio("recorder state poisoned".into()))?,
        );
        if samples.is_empty() {
            return Err(Error::Audio("nothing was recorded".into()));
        }
        if !self.shared.heard.load(Ordering::Relaxed) {
            return Err(Error::Audio(format!(
                "the microphone delivered only silence. {}",
                mic_permission_hint()
            )));
        }
        Ok(AudioBuffer::new(samples, self.sample_rate))
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    shared: &Arc<RecShared>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let data_shared = Arc::clone(shared);
    let err_shared = Arc::clone(shared);
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut peak = 0.0f32;
                let mut nonzero = false;
                let mono = data.chunks(channels).map(|frame| {
                    let v =
                        frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32;
                    peak = peak.max(v.abs());
                    nonzero |= v != 0.0;
                    v
                });
                if let Ok(mut buf) = data_shared.samples.lock() {
                    buf.extend(mono);
                }
                data_shared.peak.fetch_max(peak);
                if nonzero {
                    data_shared.heard.store(true, Ordering::Relaxed);
                }
            },
            move |e| log_stream_error(e, &err_shared.error),
            None,
        )
        .map_err(|e| map_err("could not open the microphone", e))
}

/// One mono track in the playback mix.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayTrack {
    pub samples: Vec<f32>,
    pub gain: f32,
    pub pan: f32,
    pub muted: bool,
}

#[derive(Default)]
struct Mix {
    tracks: Vec<PlayTrack>,
    pos: usize,
    playing: bool,
    peaks: Vec<f32>,
    master_peak: f32,
}

struct PlayShared {
    mix: Mutex<Mix>,
    error: Mutex<Option<String>>,
}

/// Plays a set of mono tracks, mixed to stereo, on the default output.
pub struct Player {
    _stream: cpal::Stream,
    shared: Arc<PlayShared>,
    sample_rate: u32,
}

impl Player {
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            Error::Audio("no speakers or headphones found; choose a default output device in your sound settings".into())
        })?;
        let supported = device
            .default_output_config()
            .map_err(|e| map_err("could not open the speakers", e))?;
        let sample_rate = supported.sample_rate();
        let channels = usize::from(supported.channels()).max(1);
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let shared = Arc::new(PlayShared {
            mix: Mutex::new(Mix::default()),
            error: Mutex::new(None),
        });
        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&device, config, channels, &shared),
            SampleFormat::I16 => build_output::<i16>(&device, config, channels, &shared),
            SampleFormat::I32 => build_output::<i32>(&device, config, channels, &shared),
            SampleFormat::U16 => build_output::<u16>(&device, config, channels, &shared),
            SampleFormat::F64 => build_output::<f64>(&device, config, channels, &shared),
            other => {
                return Err(Error::Audio(format!(
                    "the output device uses sample format {other}, which is not supported yet"
                )));
            }
        }?;
        stream
            .play()
            .map_err(|e| map_err("could not start playback", e))?;
        Ok(Self {
            _stream: stream,
            shared,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn with_mix<R>(&self, f: impl FnOnce(&mut Mix) -> R) -> Option<R> {
        self.shared.mix.lock().ok().map(|mut m| f(&mut m))
    }

    /// Replaces the tracks (samples must be at [`Player::sample_rate`]).
    pub fn set_tracks(&self, tracks: Vec<PlayTrack>) {
        self.with_mix(|m| {
            m.peaks = vec![0.0; tracks.len()];
            m.tracks = tracks;
        });
    }

    /// Updates gain/pan/mute without touching the audio.
    pub fn set_track_params(&self, index: usize, gain: f32, pan: f32, muted: bool) {
        self.with_mix(|m| {
            if let Some(t) = m.tracks.get_mut(index) {
                t.gain = gain;
                t.pan = pan;
                t.muted = muted;
            }
        });
    }

    pub fn play_from(&self, sample: usize) {
        self.with_mix(|m| {
            m.pos = sample;
            m.playing = true;
        });
    }

    pub fn stop(&self) {
        self.with_mix(|m| m.playing = false);
    }

    pub fn is_playing(&self) -> bool {
        self.with_mix(|m| m.playing).unwrap_or(false)
    }

    pub fn position(&self) -> usize {
        self.with_mix(|m| m.pos).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.with_mix(|m| m.tracks.iter().map(|t| t.samples.len()).max().unwrap_or(0))
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Per-track and master peak levels since the last call.
    pub fn take_peaks(&self) -> (Vec<f32>, f32) {
        self.with_mix(|m| {
            let p = std::mem::replace(&mut m.peaks, vec![0.0; m.tracks.len()]);
            (p, std::mem::take(&mut m.master_peak))
        })
        .unwrap_or_default()
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }
}

fn build_output<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    shared: &Arc<PlayShared>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let data_shared = Arc::clone(shared);
    let err_shared = Arc::clone(shared);
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                let silence = T::from_sample(0.0f32);
                // Never block the audio thread: if the UI holds the lock, output
                // silence for this one buffer.
                let Ok(mut mix) = data_shared.mix.try_lock() else {
                    out.fill(silence);
                    return;
                };
                let mix = &mut *mix;
                if !mix.playing {
                    out.fill(silence);
                    return;
                }
                let len = mix
                    .tracks
                    .iter()
                    .map(|t| t.samples.len())
                    .max()
                    .unwrap_or(0);
                let pans: Vec<(f32, f32)> = mix.tracks.iter().map(|t| pan_gains(t.pan)).collect();
                for frame in out.chunks_mut(channels) {
                    let (mut l, mut r) = (0.0f32, 0.0f32);
                    if mix.pos < len {
                        for (i, t) in mix.tracks.iter().enumerate() {
                            if t.muted {
                                continue;
                            }
                            let x = t.samples.get(mix.pos).copied().unwrap_or(0.0) * t.gain;
                            mix.peaks[i] = mix.peaks[i].max(x.abs());
                            l += x * pans[i].0;
                            r += x * pans[i].1;
                        }
                        mix.pos += 1;
                    }
                    let (l, r) = (soft_clip(l), soft_clip(r));
                    mix.master_peak = mix.master_peak.max(l.abs()).max(r.abs());
                    match frame.len() {
                        1 => frame[0] = T::from_sample((l + r) * 0.5),
                        _ => {
                            frame[0] = T::from_sample(l);
                            frame[1] = T::from_sample(r);
                            for s in &mut frame[2..] {
                                *s = silence;
                            }
                        }
                    }
                }
                if mix.pos >= len {
                    mix.playing = false;
                }
            },
            move |e| log_stream_error(e, &err_shared.error),
            None,
        )
        .map_err(|e| map_err("could not open the speakers", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_f32() {
        let a = AtomicF32::default();
        a.fetch_max(0.5);
        a.fetch_max(0.2);
        assert_eq!(a.load(), 0.5);
        assert_eq!(a.take(), 0.5);
        assert_eq!(a.load(), 0.0);
    }

    #[test]
    fn hint_mentions_microphone() {
        assert!(mic_permission_hint().to_lowercase().contains("microphone"));
    }
}
