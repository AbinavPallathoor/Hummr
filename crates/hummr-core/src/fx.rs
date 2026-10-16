//! Effects: gain, pan, fades, reverse, EQ, filters, distortion, bitcrush,
//! echo, reverb, compressor and limiter; effect chains for clips and tracks;
//! and named presets with a 0..1 "amount" macro.
//!
//! Everything is written from scratch, runs offline on mono buffers and is
//! deterministic (no randomness, no time-dependence).

use std::f32::consts::FRAC_PI_2;

use serde::{Deserialize, Serialize};

use crate::dsp::{Biquad, db_to_amp};

/// The effects on one clip or track: an optional named preset with an
/// "amount" macro, four friendly sliders, and raw effects one level down.
///
/// The processing order is preset, then the sliders (drive, filter, echo,
/// reverb), then the raw effects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxChain {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// How strongly the preset is applied, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub amount: f64,
    /// Distortion, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub drive: f64,
    /// Reverb, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub reverb: f64,
    /// Echo, 0..=1.
    #[serde(skip_serializing_if = "is_zero")]
    pub echo: f64,
    /// Filter sweep: below 0 darkens (low-pass), above 0 thins (high-pass).
    #[serde(skip_serializing_if = "is_zero")]
    pub filter: f64,
    /// Raw effects, applied last.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<Effect>,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

impl FxChain {
    pub fn is_empty(&self) -> bool {
        (self.preset.is_none() || self.amount <= 0.0)
            && self.drive <= 0.0
            && self.reverb <= 0.0
            && self.echo <= 0.0
            && self.filter == 0.0
            && self.raw.is_empty()
    }

    /// A chain with just a preset at `amount`.
    pub fn preset(p: Preset, amount: f64) -> Self {
        Self {
            preset: Some(p.name().to_string()),
            amount,
            ..Self::default()
        }
    }

    /// The raw effects this chain runs, in order.
    pub fn effects(&self) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.amount > 0.0 {
            if let Some(p) = self.preset.as_deref().and_then(Preset::from_name) {
                out.extend(p.effects(self.amount));
            }
        }
        if self.drive > 0.0 {
            let d = self.drive.clamp(0.0, 1.0);
            out.push(Effect::Distortion {
                drive: 1.0 + 24.0 * d * d,
                shape: Shape::Soft,
                mix: 0.3 + 0.7 * d,
            });
        }
        if self.filter < 0.0 {
            // Exponential sweep from 20 kHz down to 200 Hz.
            let f = (-self.filter).clamp(0.0, 1.0);
            out.push(Effect::Filter {
                kind: FilterKind::Lowpass,
                hz: 20_000.0 * (0.01f64).powf(f),
                q: 0.8,
            });
        } else if self.filter > 0.0 {
            // From 20 Hz up to 5 kHz.
            let f = self.filter.clamp(0.0, 1.0);
            out.push(Effect::Filter {
                kind: FilterKind::Highpass,
                hz: 20.0 * 250f64.powf(f),
                q: 0.8,
            });
        }
        if self.echo > 0.0 {
            let e = self.echo.clamp(0.0, 1.0);
            out.push(Effect::Echo {
                beats: 0.75,
                feedback: 0.2 + 0.45 * e,
                mix: 0.45 * e,
            });
        }
        if self.reverb > 0.0 {
            let r = self.reverb.clamp(0.0, 1.0);
            out.push(Effect::Reverb {
                size: 0.5 + 0.45 * r,
                damping: 0.4,
                mix: 0.5 * r,
            });
        }
        out.extend(self.raw.iter().cloned());
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    Lowpass,
    Highpass,
    Bandpass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Smooth tanh saturation.
    Soft,
    /// Wave folding: loud parts fold back, which sounds harsh and buzzy.
    Fold,
}

/// One raw effect with all of its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Effect {
    Gain {
        db: f64,
    },
    Filter {
        kind: FilterKind,
        hz: f64,
        q: f64,
    },
    Eq {
        low_db: f64,
        mid_db: f64,
        mid_hz: f64,
        high_db: f64,
    },
    Distortion {
        drive: f64,
        shape: Shape,
        mix: f64,
    },
    Bitcrush {
        bits: f64,
        /// Keep one sample in `downsample`.
        downsample: f64,
    },
    Echo {
        /// Delay time in beats, so echoes stay in time with the song.
        beats: f64,
        feedback: f64,
        mix: f64,
    },
    Reverb {
        size: f64,
        damping: f64,
        mix: f64,
    },
    Compressor {
        threshold_db: f64,
        ratio: f64,
        attack_ms: f64,
        release_ms: f64,
        makeup_db: f64,
    },
    Limiter {
        ceiling_db: f64,
    },
}

impl Effect {
    /// Seconds of sound this effect adds after the input ends.
    pub fn tail(&self, bpm: f64) -> f64 {
        match *self {
            Effect::Echo {
                beats, feedback, ..
            } => {
                let delay = beats.max(0.01) * 60.0 / bpm;
                let fb = feedback.clamp(0.0, 0.95);
                let repeats = if fb > 0.0 {
                    (0.001f64.ln() / fb.ln()).ceil().max(1.0)
                } else {
                    1.0
                };
                (delay * (repeats + 1.0)).min(8.0)
            }
            Effect::Reverb { size, .. } => 0.8 + 4.5 * size.clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// Short name for lists.
    pub fn label(&self) -> &'static str {
        match self {
            Effect::Gain { .. } => "gain",
            Effect::Filter { .. } => "filter",
            Effect::Eq { .. } => "eq",
            Effect::Distortion { .. } => "distortion",
            Effect::Bitcrush { .. } => "bitcrush",
            Effect::Echo { .. } => "echo",
            Effect::Reverb { .. } => "reverb",
            Effect::Compressor { .. } => "compressor",
            Effect::Limiter { .. } => "limiter",
        }
    }
}

// ---- presets ------------------------------------------------------------

/// Named looks. Each is a recipe of raw effects scaled by an amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    LoFi,
    Dreamy,
    Gritty,
    Telephone,
    Radio,
    BigHall,
    Warm,
    Bright,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Preset::LoFi,
        Preset::Dreamy,
        Preset::Gritty,
        Preset::Telephone,
        Preset::Radio,
        Preset::BigHall,
        Preset::Warm,
        Preset::Bright,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::LoFi => "Lo-fi",
            Preset::Dreamy => "Dreamy",
            Preset::Gritty => "Gritty",
            Preset::Telephone => "Telephone",
            Preset::Radio => "Radio",
            Preset::BigHall => "Big hall",
            Preset::Warm => "Warm",
            Preset::Bright => "Bright",
        }
    }

    /// One plain sentence about what it does.
    pub fn describe(self) -> &'static str {
        match self {
            Preset::LoFi => "crunchy, dusty and muffled, like an old sampler",
            Preset::Dreamy => "soft and far away, with echoes and a long tail",
            Preset::Gritty => "loud and dirty, pushed into distortion",
            Preset::Telephone => "thin and boxy, like a phone call",
            Preset::Radio => "squashed and mid-heavy, like a small speaker",
            Preset::BigHall => "a large, echoing room",
            Preset::Warm => "fuller lows, softer highs, a little saturation",
            Preset::Bright => "clearer and crisper highs",
        }
    }

    /// Finds a preset by name, ignoring case, spaces and dashes.
    pub fn from_name(name: &str) -> Option<Preset> {
        let key = |s: &str| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        };
        let k = key(name);
        Preset::ALL.into_iter().find(|p| key(p.name()) == k)
    }

    /// The raw effects for this preset at `amount` (0..=1).
    pub fn effects(self, amount: f64) -> Vec<Effect> {
        let a = amount.clamp(0.0, 1.0);
        let lerp = |from: f64, to: f64| from + (to - from) * a;
        // Exponential interpolation, for frequencies.
        let elerp = |from: f64, to: f64| from * (to / from).powf(a);
        match self {
            Preset::LoFi => vec![
                Effect::Bitcrush {
                    bits: lerp(16.0, 7.0),
                    downsample: lerp(1.0, 4.0),
                },
                Effect::Filter {
                    kind: FilterKind::Lowpass,
                    hz: elerp(20_000.0, 3_500.0),
                    q: 0.7,
                },
                Effect::Distortion {
                    drive: lerp(1.0, 3.0),
                    shape: Shape::Soft,
                    mix: lerp(0.0, 0.6),
                },
            ],
            Preset::Dreamy => vec![
                Effect::Filter {
                    kind: FilterKind::Lowpass,
                    hz: elerp(20_000.0, 7_000.0),
                    q: 0.7,
                },
                Effect::Echo {
                    beats: 0.75,
                    feedback: lerp(0.2, 0.5),
                    mix: lerp(0.0, 0.3),
                },
                Effect::Reverb {
                    size: 0.85,
                    damping: 0.3,
                    mix: lerp(0.0, 0.5),
                },
            ],
            Preset::Gritty => vec![
                Effect::Eq {
                    low_db: 0.0,
                    mid_db: lerp(0.0, 4.0),
                    mid_hz: 1_500.0,
                    high_db: 0.0,
                },
                Effect::Distortion {
                    drive: lerp(1.0, 16.0),
                    shape: Shape::Soft,
                    mix: lerp(0.0, 1.0),
                },
                Effect::Compressor {
                    threshold_db: lerp(0.0, -18.0),
                    ratio: lerp(1.0, 4.0),
                    attack_ms: 5.0,
                    release_ms: 80.0,
                    makeup_db: lerp(0.0, 4.0),
                },
            ],
            Preset::Telephone => vec![
                Effect::Filter {
                    kind: FilterKind::Highpass,
                    hz: elerp(20.0, 400.0),
                    q: 0.9,
                },
                Effect::Filter {
                    kind: FilterKind::Lowpass,
                    hz: elerp(20_000.0, 3_200.0),
                    q: 0.9,
                },
                Effect::Distortion {
                    drive: lerp(1.0, 4.0),
                    shape: Shape::Soft,
                    mix: lerp(0.0, 0.5),
                },
            ],
            Preset::Radio => vec![
                Effect::Filter {
                    kind: FilterKind::Highpass,
                    hz: elerp(20.0, 250.0),
                    q: 0.7,
                },
                Effect::Filter {
                    kind: FilterKind::Lowpass,
                    hz: elerp(20_000.0, 5_500.0),
                    q: 0.7,
                },
                Effect::Eq {
                    low_db: 0.0,
                    mid_db: lerp(0.0, 5.0),
                    mid_hz: 1_200.0,
                    high_db: 0.0,
                },
                Effect::Compressor {
                    threshold_db: lerp(0.0, -24.0),
                    ratio: lerp(1.0, 6.0),
                    attack_ms: 3.0,
                    release_ms: 120.0,
                    makeup_db: lerp(0.0, 6.0),
                },
            ],
            Preset::BigHall => vec![Effect::Reverb {
                size: 0.97,
                damping: 0.25,
                mix: lerp(0.0, 0.6),
            }],
            Preset::Warm => vec![
                Effect::Eq {
                    low_db: lerp(0.0, 4.0),
                    mid_db: 0.0,
                    mid_hz: 1_000.0,
                    high_db: lerp(0.0, -4.0),
                },
                Effect::Distortion {
                    drive: lerp(1.0, 2.5),
                    shape: Shape::Soft,
                    mix: lerp(0.0, 0.5),
                },
            ],
            Preset::Bright => vec![
                Effect::Filter {
                    kind: FilterKind::Highpass,
                    hz: elerp(20.0, 90.0),
                    q: 0.7,
                },
                Effect::Eq {
                    low_db: lerp(0.0, -2.0),
                    mid_db: 0.0,
                    mid_hz: 1_000.0,
                    high_db: lerp(0.0, 6.0),
                },
                Effect::Compressor {
                    threshold_db: lerp(0.0, -14.0),
                    ratio: lerp(1.0, 2.5),
                    attack_ms: 10.0,
                    release_ms: 100.0,
                    makeup_db: lerp(0.0, 2.0),
                },
            ],
        }
    }
}

// ---- simple processors ----------------------------------------------------

pub fn gain(buf: &mut [f32], db: f64) {
    let g = db_to_amp(db as f32);
    for s in buf {
        *s *= g;
    }
}

/// Constant-power pan of a mono signal to (left, right).
pub fn pan(mono: &[f32], pan: f64) -> (Vec<f32>, Vec<f32>) {
    let (l, r) = crate::preview::pan_gains(pan as f32);
    (
        mono.iter().map(|x| x * l).collect(),
        mono.iter().map(|x| x * r).collect(),
    )
}

/// Linear fade-in and fade-out (seconds).
pub fn fades(buf: &mut [f32], sample_rate: u32, fade_in: f64, fade_out: f64) {
    let sr = f64::from(sample_rate);
    crate::project::render::fade_in(buf, (fade_in * sr) as usize);
    crate::project::render::fade_out(buf, (fade_out * sr) as usize);
}

pub fn reverse(buf: &mut [f32]) {
    buf.reverse();
}

pub fn filter(buf: &mut [f32], sample_rate: u32, kind: FilterKind, hz: f64, q: f64) {
    let sr = sample_rate as f32;
    let hz = hz.clamp(10.0, f64::from(sample_rate) * 0.49) as f32;
    let q = q.clamp(0.1, 20.0) as f32;
    let mut f = match kind {
        FilterKind::Lowpass => Biquad::lowpass(sr, hz, q),
        FilterKind::Highpass => Biquad::highpass(sr, hz, q),
        FilterKind::Bandpass => Biquad::bandpass(sr, hz, q),
    };
    f.process_buffer(buf);
}

/// Three-band EQ: low shelf at 200 Hz, a peak at `mid_hz`, high shelf at
/// 5 kHz.
pub fn eq(buf: &mut [f32], sample_rate: u32, low_db: f64, mid_db: f64, mid_hz: f64, high_db: f64) {
    let sr = sample_rate as f32;
    if low_db != 0.0 {
        Biquad::low_shelf(sr, 200.0, low_db as f32).process_buffer(buf);
    }
    if mid_db != 0.0 {
        Biquad::peaking(sr, mid_hz.clamp(40.0, 16_000.0) as f32, 0.9, mid_db as f32)
            .process_buffer(buf);
    }
    if high_db != 0.0 {
        Biquad::high_shelf(sr, 5_000.0, high_db as f32).process_buffer(buf);
    }
}

/// Distortion. Soft: `tanh(drive·x)` scaled so full scale stays full scale.
/// Fold: a sine wave-folder, where loud parts fold back on themselves.
pub fn distort(buf: &mut [f32], drive: f64, shape: Shape, mix: f64) {
    let d = drive.clamp(1.0, 100.0) as f32;
    let mix = mix.clamp(0.0, 1.0) as f32;
    let norm = d.tanh();
    for s in buf {
        let x = *s;
        let wet = match shape {
            Shape::Soft => (d * x).tanh() / norm,
            Shape::Fold => (FRAC_PI_2 * d * x).sin(),
        };
        *s = x + (wet - x) * mix;
    }
}

/// Reduces bit depth to `bits` and holds each sample for `downsample`
/// samples.
pub fn bitcrush(buf: &mut [f32], bits: f64, downsample: f64) {
    let levels = 2f32.powf(bits.clamp(1.0, 24.0) as f32 - 1.0);
    let step = downsample.max(1.0);
    let mut phase = 0.0f64;
    let mut held = 0.0f32;
    for (i, s) in buf.iter_mut().enumerate() {
        if i == 0 || phase >= 1.0 {
            phase -= phase.floor();
            held = (*s * levels).round() / levels;
        }
        phase += 1.0 / step;
        *s = held;
    }
}

/// Feedback echo with darker repeats. `buf` should already include room
/// for the tail.
pub fn echo(buf: &mut [f32], sample_rate: u32, delay_seconds: f64, feedback: f64, mix: f64) {
    let d = ((delay_seconds * f64::from(sample_rate)).round() as usize).max(1);
    let fb = feedback.clamp(0.0, 0.95) as f32;
    let mix = mix.clamp(0.0, 1.0) as f32;
    let mut line = vec![0.0f32; d];
    let mut idx = 0;
    // One-pole low-pass (~3.5 kHz) in the loop.
    let a = (-std::f32::consts::TAU * 3_500.0 / sample_rate as f32).exp();
    let mut lp = 0.0f32;
    for s in buf {
        let delayed = line[idx];
        lp = (1.0 - a) * delayed + a * lp;
        line[idx] = *s + lp * fb;
        idx = (idx + 1) % d;
        *s += delayed * mix;
    }
}

/// Freeverb-style reverb (Jezar's design): eight damped comb filters in
/// parallel into four all-passes. `channel` 1 uses slightly longer delays
/// so left and right decorrelate. `buf` should include room for the tail.
pub fn reverb(
    buf: &mut [f32],
    sample_rate: u32,
    size: f64,
    damping: f64,
    mix: f64,
    channel: usize,
) {
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
    let scale = f64::from(sample_rate) / 44_100.0;
    let spread = if channel == 1 { 23 } else { 0 };
    let len = |n: usize| (((n + spread) as f64) * scale).round().max(1.0) as usize;
    let feedback = (0.7 + 0.28 * size.clamp(0.0, 1.0)) as f32;
    let damp = (damping.clamp(0.0, 1.0) * 0.4) as f32;
    let mix = mix.clamp(0.0, 1.0) as f32;

    let mut combs: Vec<(Vec<f32>, usize, f32)> =
        COMBS.iter().map(|&n| (vec![0.0; len(n)], 0, 0.0)).collect();
    let mut aps: Vec<(Vec<f32>, usize)> =
        ALLPASSES.iter().map(|&n| (vec![0.0; len(n)], 0)).collect();
    for s in buf {
        let input = *s * 0.015;
        let mut out = 0.0f32;
        for (line, idx, store) in &mut combs {
            let y = line[*idx];
            *store = y * (1.0 - damp) + *store * damp;
            line[*idx] = input + *store * feedback;
            *idx = (*idx + 1) % line.len();
            out += y;
        }
        for (line, idx) in &mut aps {
            let b = line[*idx];
            line[*idx] = out + b * 0.5;
            *idx = (*idx + 1) % line.len();
            out = b - out;
        }
        *s = *s * (1.0 - mix) + out * 3.0 * mix;
    }
}

/// Feed-forward compressor with a 6 dB soft knee.
pub fn compress(
    buf: &mut [f32],
    sample_rate: u32,
    threshold_db: f64,
    ratio: f64,
    attack_ms: f64,
    release_ms: f64,
    makeup_db: f64,
) {
    let sr = f64::from(sample_rate);
    let coef = |ms: f64| (-1.0 / (sr * ms.max(0.1) / 1000.0)).exp() as f32;
    let (att, rel) = (coef(attack_ms), coef(release_ms));
    let t = threshold_db as f32;
    let r = ratio.max(1.0) as f32;
    let knee = 6.0f32;
    let makeup = makeup_db as f32;
    let mut env = 0.0f32;
    for s in buf {
        let x = s.abs();
        let c = if x > env { att } else { rel };
        env = x + (env - x) * c;
        let level = 20.0 * env.max(1e-9).log10();
        let over = level - t;
        let reduction = if over <= -knee / 2.0 {
            0.0
        } else if over >= knee / 2.0 {
            over * (1.0 - 1.0 / r)
        } else {
            (1.0 - 1.0 / r) * (over + knee / 2.0).powi(2) / (2.0 * knee)
        };
        *s *= 10f32.powf((makeup - reduction) / 20.0);
    }
}

/// Runs one effect in place. `buf` must already include room for tails.
pub fn process(effect: &Effect, buf: &mut [f32], sample_rate: u32, bpm: f64, channel: usize) {
    match *effect {
        Effect::Gain { db } => gain(buf, db),
        Effect::Filter { kind, hz, q } => filter(buf, sample_rate, kind, hz, q),
        Effect::Eq {
            low_db,
            mid_db,
            mid_hz,
            high_db,
        } => eq(buf, sample_rate, low_db, mid_db, mid_hz, high_db),
        Effect::Distortion { drive, shape, mix } => distort(buf, drive, shape, mix),
        Effect::Bitcrush { bits, downsample } => bitcrush(buf, bits, downsample),
        Effect::Echo {
            beats,
            feedback,
            mix,
        } => echo(buf, sample_rate, beats * 60.0 / bpm.max(1.0), feedback, mix),
        Effect::Reverb { size, damping, mix } => {
            reverb(buf, sample_rate, size, damping, mix, channel)
        }
        Effect::Compressor {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
        } => compress(
            buf,
            sample_rate,
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
        ),
        Effect::Limiter { ceiling_db } => limit(buf, sample_rate, ceiling_db),
    }
}

/// Runs a chain over a mono signal. `channel` (0 left, 1 right) lets
/// stereo effects such as reverb decorrelate the two sides. The output may
/// be longer than the input (echo and reverb tails); trailing silence is
/// trimmed.
pub fn apply_chain(
    chain: &FxChain,
    input: &[f32],
    sample_rate: u32,
    bpm: f64,
    channel: usize,
) -> Vec<f32> {
    let effects = chain.effects();
    if effects.is_empty() || input.is_empty() {
        return input.to_vec();
    }
    let tail: f64 = effects.iter().map(|e| e.tail(bpm)).sum();
    let mut buf = input.to_vec();
    buf.resize(input.len() + (tail * f64::from(sample_rate)) as usize, 0.0);
    for e in &effects {
        process(e, &mut buf, sample_rate, bpm, channel);
    }
    // Drop the part of the tail that has died away (below -90 dBFS).
    let floor = db_to_amp(-90.0);
    let end = buf
        .iter()
        .rposition(|s| s.abs() > floor)
        .map_or(0, |i| i + 1)
        .max(input.len());
    buf.truncate(end);
    for s in &mut buf {
        if !s.is_finite() {
            *s = 0.0;
        }
    }
    buf
}

/// A look-ahead peak limiter. Below the ceiling it is bit-transparent (gain
/// exactly 1); above, the gain ramps down ahead of each peak and recovers
/// smoothly. Offline only: it returns output aligned with the input.
pub fn limit(samples: &mut [f32], sample_rate: u32, ceiling_db: f64) {
    limit_linked(&mut [samples], sample_rate, ceiling_db);
}

/// Limits several channels with one shared gain so the stereo image holds.
pub fn limit_linked(channels: &mut [&mut [f32]], sample_rate: u32, ceiling_db: f64) {
    let len = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    if len == 0 {
        return;
    }
    let ceiling = db_to_amp(ceiling_db as f32);
    let look = ((sample_rate as f64 * 0.005) as usize).max(1);
    let release = (-1.0 / (sample_rate as f64 * 0.08)).exp() as f32;
    // Required gain per sample.
    let mut need = vec![1.0f32; len];
    let mut any = false;
    for c in channels.iter() {
        for (i, &s) in c.iter().enumerate() {
            let a = if s.is_finite() { s.abs() } else { f32::MAX };
            if a > ceiling {
                need[i] = need[i].min(ceiling / a);
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    // Minimum over the look-ahead window (so gain is down before the peak),
    // then a linear attack ramp across that window and an exponential release.
    let mut window_min = vec![1.0f32; len];
    let mut deque: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in 0..len + look {
        if i < len {
            while deque.back().is_some_and(|&j| need[j] >= need[i]) {
                deque.pop_back();
            }
            deque.push_back(i);
        }
        if i >= look {
            let out = i - look;
            while deque.front().is_some_and(|&j| j < out) {
                deque.pop_front();
            }
            window_min[out] = deque.front().map_or(1.0, |&j| need[j]);
        }
    }
    let mut gain = 1.0f32;
    let attack_step = 1.0 / look as f32;
    for i in 0..len {
        let target = window_min[i];
        if target < gain {
            // Reach the target by the time the peak arrives.
            gain = (gain - attack_step).max(target);
        } else {
            gain = target + (gain - target) * release;
        }
        let g = gain.min(need[i]);
        for c in channels.iter_mut() {
            if let Some(s) = c.get_mut(i) {
                if !s.is_finite() {
                    *s = 0.0;
                } else if g < 1.0 {
                    *s *= g;
                }
            }
        }
    }
}

/// Peak level in dBFS.
pub fn peak_db(x: &[f32]) -> f32 {
    20.0 * x
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()))
        .max(1e-9)
        .log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;
    use crate::synthetic::{sine, white_noise};

    const SR: u32 = 48_000;

    fn level_db(x: &[f32]) -> f32 {
        20.0 * rms(&x[x.len() / 4..x.len() * 3 / 4]).max(1e-9).log10()
    }

    /// Gain (in dB) a processor applies to a sine at `hz`.
    fn response(hz: f32, f: impl Fn(&mut Vec<f32>)) -> f32 {
        let x = sine(hz, SR, 0.5, 0.25);
        let mut y = x.clone();
        f(&mut y);
        level_db(&y) - level_db(&x)
    }

    #[test]
    fn gain_pan_fades_reverse() {
        let mut x = vec![0.25f32; 100];
        gain(&mut x, 6.0206);
        assert!((x[0] - 0.5).abs() < 1e-4);

        let (l, r) = pan(&[1.0], -1.0);
        assert!((l[0] - 1.0).abs() < 1e-6 && r[0].abs() < 1e-6);
        let (l, r) = pan(&[1.0], 0.0);
        assert!((l[0] - r[0]).abs() < 1e-6);

        let mut f = vec![1.0f32; 4_800];
        fades(&mut f, SR, 0.01, 0.01);
        assert_eq!(f[0], 0.0);
        assert_eq!(f[2_400], 1.0);
        assert!(f[4_799] < 0.01);

        let mut r = vec![1.0, 2.0, 3.0];
        reverse(&mut r);
        assert_eq!(r, vec![3.0, 2.0, 1.0]);
    }

    #[test]
    fn filters_pass_and_block() {
        let lp = |y: &mut Vec<f32>| filter(y, SR, FilterKind::Lowpass, 500.0, 0.707);
        assert!(response(100.0, lp).abs() < 0.5);
        assert!(response(5_000.0, lp) < -30.0);
        let hp = |y: &mut Vec<f32>| filter(y, SR, FilterKind::Highpass, 2_000.0, 0.707);
        assert!(response(8_000.0, hp).abs() < 0.5);
        assert!(response(200.0, hp) < -30.0);
        let bp = |y: &mut Vec<f32>| filter(y, SR, FilterKind::Bandpass, 1_000.0, 2.0);
        assert!(response(1_000.0, bp).abs() < 0.5);
        assert!(response(100.0, bp) < -20.0);
    }

    #[test]
    fn eq_shapes_the_bands() {
        let e = |y: &mut Vec<f32>| eq(y, SR, 6.0, 0.0, 1_000.0, -6.0);
        assert!((response(50.0, e) - 6.0).abs() < 1.0);
        assert!((response(15_000.0, e) + 6.0).abs() < 1.0);
        let m = |y: &mut Vec<f32>| eq(y, SR, 0.0, 5.0, 1_000.0, 0.0);
        assert!((response(1_000.0, m) - 5.0).abs() < 0.5);
    }

    #[test]
    fn distortion_adds_harmonics_and_stays_bounded() {
        for shape in [Shape::Soft, Shape::Fold] {
            let x = sine(200.0, SR, 0.5, 1.0);
            let mut y = x.clone();
            distort(&mut y, 10.0, shape, 1.0);
            assert!(y.iter().all(|s| s.abs() <= 1.0 + 1e-6), "{shape:?}");
            // Energy above 1 kHz appears where there was none.
            let mut hi = y.clone();
            filter(&mut hi, SR, FilterKind::Highpass, 1_000.0, 0.707);
            filter(&mut hi, SR, FilterKind::Highpass, 1_000.0, 0.707);
            assert!(level_db(&hi) - level_db(&y) > -20.0, "{shape:?}");
        }
        let mut dry = vec![0.3f32; 10];
        distort(&mut dry, 10.0, Shape::Soft, 0.0);
        assert_eq!(dry, vec![0.3; 10]);
    }

    #[test]
    fn bitcrush_quantises_and_holds() {
        let mut x: Vec<f32> = (0..64).map(|i| (i as f32 / 64.0) * 2.0 - 1.0).collect();
        bitcrush(&mut x, 3.0, 4.0);
        let mut levels: Vec<i32> = x.iter().map(|s| (s * 1000.0).round() as i32).collect();
        levels.dedup();
        assert!(levels.len() <= 16);
        assert!(x.chunks(4).all(|c| c.iter().all(|&s| s == c[0])));
    }

    #[test]
    fn echo_repeats_an_impulse() {
        let mut x = vec![0.0f32; 48_000];
        x[0] = 1.0;
        echo(&mut x, SR, 0.25, 0.5, 0.5);
        assert_eq!(x[0], 1.0);
        let first = x[12_000..12_100].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let second = x[24_000..24_200].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(first > 0.05, "first echo {first}");
        assert!(second < first && second > 0.0, "second echo {second}");
        assert!(x[1_000..11_000].iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn reverb_rings_and_decays() {
        let mut l = vec![0.0f32; 4 * 48_000];
        l[0] = 1.0;
        let mut r = l.clone();
        reverb(&mut l, SR, 0.9, 0.3, 1.0, 0);
        reverb(&mut r, SR, 0.9, 0.3, 1.0, 1);
        let early = rms(&l[4_800..24_000]);
        let late = rms(&l[150_000..170_000]);
        assert!(early > 1e-4, "early {early}");
        assert!(late < early * 0.5, "decays: {late} vs {early}");
        assert!(l.iter().all(|s| s.is_finite()));
        assert_ne!(l, r, "left and right differ");
        // Bigger rooms ring longer.
        let mut small = vec![0.0f32; 4 * 48_000];
        small[0] = 1.0;
        reverb(&mut small, SR, 0.1, 0.3, 1.0, 0);
        assert!(rms(&small[96_000..120_000]) < rms(&l[96_000..120_000]));
    }

    #[test]
    fn compressor_reduces_loud_parts_by_the_ratio() {
        // A -6 dBFS sine, threshold -20, ratio 4: 14 dB over becomes 3.5.
        let x = sine(1_000.0, SR, 1.0, 0.5);
        let mut y = x.clone();
        compress(&mut y, SR, -20.0, 4.0, 1.0, 50.0, 0.0);
        let peak_in = peak_db(&x[24_000..]);
        let peak_out = peak_db(&y[24_000..]);
        assert!(
            (peak_out - (-20.0 + (peak_in + 20.0) / 4.0)).abs() < 1.5,
            "{peak_in} -> {peak_out}"
        );
        // Quiet signals pass untouched.
        let q = sine(1_000.0, SR, 0.2, 0.01);
        let mut qy = q.clone();
        compress(&mut qy, SR, -20.0, 4.0, 1.0, 50.0, 0.0);
        assert!((peak_db(&qy) - peak_db(&q)).abs() < 0.1);
    }

    #[test]
    fn limiter_is_transparent_below_the_ceiling() {
        let x = sine(220.0, SR, 0.2, 0.5);
        let mut y = x.clone();
        limit(&mut y, SR, -1.0);
        assert_eq!(x, y);
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        let mut y = sine(220.0, SR, 0.5, 4.0);
        limit(&mut y, SR, -1.0);
        let ceiling = db_to_amp(-1.0);
        assert!(y.iter().all(|s| s.is_finite() && s.abs() <= ceiling + 1e-6));
        assert!(y.iter().fold(0.0f32, |m, s| m.max(s.abs())) > 0.8 * ceiling);
    }

    /// A harsh full-scale test signal: a square-ish tone plus noise bursts.
    fn full_scale() -> Vec<f32> {
        let mut x: Vec<f32> = sine(110.0, SR, 2.0, 1.0)
            .iter()
            .map(|s| (s * 4.0).clamp(-1.0, 1.0))
            .collect();
        let n = white_noise(x.len(), 1.0, 4);
        for (i, s) in x.iter_mut().enumerate() {
            if (i / 4_800) % 3 == 0 {
                *s = n[i];
            }
        }
        x
    }

    #[test]
    fn every_preset_is_finite_bounded_and_deterministic_through_the_limiter() {
        let x = full_scale();
        let ceiling = db_to_amp(-1.0);
        for p in Preset::ALL {
            for amount in [0.3, 1.0] {
                for ch in [0, 1] {
                    let chain = FxChain::preset(p, amount);
                    let mut y = apply_chain(&chain, &x, SR, 120.0, ch);
                    assert!(y.iter().all(|s| s.is_finite()), "{} {amount}", p.name());
                    limit(&mut y, SR, -1.0);
                    assert!(
                        y.iter().all(|s| s.abs() <= ceiling + 1e-6),
                        "{} {amount}",
                        p.name()
                    );
                    let again = {
                        let mut z = apply_chain(&chain, &x, SR, 120.0, ch);
                        limit(&mut z, SR, -1.0);
                        z
                    };
                    assert_eq!(y, again, "{} is not deterministic", p.name());
                }
            }
        }
    }

    #[test]
    fn sliders_and_raw_effects_are_bounded_too() {
        let x = full_scale();
        let chain = FxChain {
            drive: 1.0,
            reverb: 1.0,
            echo: 1.0,
            filter: -0.5,
            raw: vec![
                Effect::Distortion {
                    drive: 50.0,
                    shape: Shape::Fold,
                    mix: 1.0,
                },
                Effect::Bitcrush {
                    bits: 2.0,
                    downsample: 8.0,
                },
                Effect::Gain { db: 24.0 },
            ],
            ..FxChain::default()
        };
        let mut y = apply_chain(&chain, &x, SR, 90.0, 0);
        assert!(y.len() > x.len(), "tails are kept");
        limit(&mut y, SR, -1.0);
        assert!(
            y.iter()
                .all(|s| s.is_finite() && s.abs() <= db_to_amp(-1.0) + 1e-6)
        );
    }

    #[test]
    fn amount_zero_is_the_dry_signal_and_amount_scales() {
        let x = sine(440.0, SR, 0.5, 0.5);
        for p in Preset::ALL {
            assert_eq!(apply_chain(&FxChain::preset(p, 0.0), &x, SR, 120.0, 0), x);
        }
        // More amount, more change.
        let diff = |a: f64| {
            let y = apply_chain(&FxChain::preset(Preset::Telephone, a), &x, SR, 120.0, 0);
            x.iter().zip(&y).map(|(a, b)| (a - b).abs()).sum::<f32>()
        };
        assert!(diff(0.2) < diff(0.6) && diff(0.6) < diff(1.0));
    }

    #[test]
    fn presets_by_name() {
        assert_eq!(Preset::from_name("lo-fi"), Some(Preset::LoFi));
        assert_eq!(Preset::from_name("BIG HALL"), Some(Preset::BigHall));
        assert_eq!(Preset::from_name("bighall"), Some(Preset::BigHall));
        assert_eq!(Preset::from_name("nope"), None);
        assert!(Preset::ALL.len() >= 8);
    }

    #[test]
    fn empty_chain() {
        assert!(FxChain::default().is_empty());
        let c = FxChain {
            reverb: 0.3,
            ..FxChain::default()
        };
        assert!(!c.is_empty());
    }
}
