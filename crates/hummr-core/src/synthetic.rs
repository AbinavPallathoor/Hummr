//! Deterministic synthetic signals for tests and demos: sines, noise and a
//! hum-like voice with vibrato, pitch drift, glides and breath noise.

use std::f32::consts::PI;

use crate::theory::midi_to_hz;

pub fn sine(freq: f32, sample_rate: u32, seconds: f32, amp: f32) -> Vec<f32> {
    let n = (seconds * sample_rate as f32) as usize;
    (0..n)
        .map(|i| amp * (2.0 * PI * freq * i as f32 / sample_rate as f32).sin())
        .collect()
}

/// Small xorshift PRNG so tests are reproducible without a `rand` dependency.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in [-1, 1).
    pub fn bipolar(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }

    /// Approximately standard normal (sum of uniforms).
    pub fn normal(&mut self) -> f32 {
        (0..6).map(|_| self.bipolar()).sum::<f32>() / 2f32.sqrt()
    }
}

pub fn white_noise(len: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..len).map(|_| amp * rng.bipolar()).collect()
}

/// One sung note: fractional MIDI pitch, start and duration in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    pub midi: f32,
    pub start: f64,
    pub duration: f64,
    /// Linear amplitude, 0..1.
    pub amp: f32,
}

impl Tone {
    pub fn new(midi: f32, start: f64, duration: f64) -> Self {
        Self {
            midi,
            start,
            duration,
            amp: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HumParams {
    pub sample_rate: u32,
    pub vibrato_hz: f32,
    pub vibrato_cents: f32,
    /// Constant offset applied to every note (a singer who is "flat").
    pub detune_cents: f32,
    /// Standard deviation of slow random pitch drift.
    pub drift_cents: f32,
    pub harmonics: usize,
    /// Harmonic amplitude falls off as 1/k^rolloff.
    pub rolloff: f32,
    /// Breath noise level relative to full scale.
    pub noise_db: f32,
    pub attack_seconds: f64,
    pub release_seconds: f64,
    /// Portamento into a note that starts right as the previous one ends.
    pub glide_seconds: f64,
    /// Trailing silence after the last note.
    pub tail_seconds: f64,
}

impl HumParams {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            vibrato_hz: 5.5,
            vibrato_cents: 25.0,
            detune_cents: 0.0,
            drift_cents: 8.0,
            harmonics: 10,
            rolloff: 1.4,
            noise_db: -45.0,
            attack_seconds: 0.03,
            release_seconds: 0.05,
            glide_seconds: 0.04,
            tail_seconds: 0.3,
        }
    }
}

fn envelope(t: f64, tone: &Tone, p: &HumParams) -> f32 {
    let rel = t - tone.start;
    if rel < 0.0 || rel > tone.duration {
        return 0.0;
    }
    let a = (rel / p.attack_seconds.max(1e-4)).min(1.0);
    let r = ((tone.duration - rel) / p.release_seconds.max(1e-4)).min(1.0);
    (a.min(r) as f32) * tone.amp
}

/// Renders a hum-like voice singing `tones` (which should not overlap).
pub fn hum(tones: &[Tone], p: &HumParams, seed: u64) -> Vec<f32> {
    let sr = f64::from(p.sample_rate);
    let end = tones
        .iter()
        .map(|t| t.start + t.duration)
        .fold(0.0, f64::max)
        + p.tail_seconds;
    let n = (end * sr) as usize;
    let mut out = vec![0.0f32; n];
    let mut rng = Rng::new(seed);
    let noise_amp = crate::dsp::db_to_amp(p.noise_db);

    // Slow drift: smoothed random walk, updated every 10 ms.
    let mut drift = 0.0f32;
    let mut drift_target = 0.0f32;
    let mut phase = 0.0f64;
    let mut vib_phase = 0.0f64;

    let mut sorted: Vec<Tone> = tones.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut current = 0usize;
    for (i, out_s) in out.iter_mut().enumerate() {
        let t = i as f64 / sr;
        while current + 1 < sorted.len() && t >= sorted[current + 1].start {
            current += 1;
        }
        if i % (p.sample_rate as usize / 100).max(1) == 0 {
            drift_target = rng.normal() * p.drift_cents / 100.0;
        }
        drift += (drift_target - drift) * 0.0005;

        let mut amp = 0.0f32;
        for tone in &sorted {
            amp += envelope(t, tone, p);
        }

        let mut breath = rng.bipolar() * noise_amp;
        if let Some(tone) = sorted.get(current) {
            let mut midi = tone.midi;
            // Glide from the previous note if this one starts legato.
            if current > 0 && p.glide_seconds > 0.0 {
                let prev = sorted[current - 1];
                let gap = tone.start - (prev.start + prev.duration);
                let since = t - tone.start;
                if gap.abs() < 0.02 && since >= 0.0 && since < p.glide_seconds {
                    let k = (since / p.glide_seconds) as f32;
                    midi = prev.midi + (tone.midi - prev.midi) * k;
                }
            }
            let vib =
                (2.0 * std::f64::consts::PI * vib_phase).sin() as f32 * p.vibrato_cents / 100.0;
            let m = midi + p.detune_cents / 100.0 + drift + vib;
            let f = f64::from(midi_to_hz(m));
            phase = (phase + f / sr).fract();
            vib_phase = (vib_phase + f64::from(p.vibrato_hz) / sr).fract();

            if amp > 0.0 {
                let mut s = 0.0f32;
                for k in 1..=p.harmonics {
                    if f * k as f64 >= sr / 2.0 {
                        break;
                    }
                    let w = (2.0 * std::f64::consts::PI * phase * k as f64).sin() as f32;
                    s += w / (k as f32).powf(p.rolloff);
                }
                *out_s = amp * s * 0.6;
                breath *= 1.0 + 4.0 * amp;
            }
        }
        *out_s += breath;
    }
    out
}

/// Note sequence for "Twinkle Twinkle Little Star" (first two phrases) in C
/// major: `(midi, beats)`.
pub const TWINKLE: [(u8, f64); 14] = [
    (60, 1.0),
    (60, 1.0),
    (67, 1.0),
    (67, 1.0),
    (69, 1.0),
    (69, 1.0),
    (67, 2.0),
    (65, 1.0),
    (65, 1.0),
    (64, 1.0),
    (64, 1.0),
    (62, 1.0),
    (62, 1.0),
    (60, 2.0),
];

/// Turns `(midi, beats)` pairs into sung tones at `bpm`, leaving a small
/// articulation gap between notes and adding timing jitter.
pub fn melody(
    notes: &[(u8, f64)],
    bpm: f64,
    start: f64,
    jitter_seconds: f64,
    seed: u64,
) -> Vec<Tone> {
    let mut rng = Rng::new(seed);
    let beat = 60.0 / bpm;
    let mut t = start;
    let mut tones = Vec::with_capacity(notes.len());
    for &(m, beats) in notes {
        let dur = beats * beat;
        let j = f64::from(rng.bipolar()) * jitter_seconds;
        let s = (t + j).max(0.0);
        tones.push(Tone::new(f32::from(m), s, dur * 0.85));
        t += dur;
    }
    tones
}

/// A drum in a synthetic loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drum {
    Kick,
    Snare,
    Hat,
}

/// One hit of a synthetic loop: where it really is and which beat it was
/// meant for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopHit {
    pub time: f64,
    pub beat: f64,
    pub drum: Drum,
}

/// Beat positions (within one 4/4 bar) of a few common patterns.
fn pattern(which: usize) -> Vec<(f64, Drum)> {
    let mut p = match which % 3 {
        // Straight rock beat, hats on every 8th.
        0 => vec![
            (0.0, Drum::Kick),
            (1.0, Drum::Snare),
            (2.0, Drum::Kick),
            (3.0, Drum::Snare),
        ],
        // Kick pushes on the "and" of 2, hats on every 8th.
        1 => vec![
            (0.0, Drum::Kick),
            (1.0, Drum::Snare),
            (1.5, Drum::Kick),
            (2.0, Drum::Kick),
            (3.0, Drum::Snare),
        ],
        // Four on the floor: kick every beat, snare on 2 and 4, hats only
        // on the off-beats.
        _ => {
            let mut v = vec![
                (0.0, Drum::Kick),
                (1.0, Drum::Kick),
                (1.0, Drum::Snare),
                (2.0, Drum::Kick),
                (3.0, Drum::Kick),
                (3.0, Drum::Snare),
            ];
            for i in 0..4 {
                v.push((f64::from(i) + 0.5, Drum::Hat));
            }
            return v;
        }
    };
    for i in 0..8 {
        p.push((f64::from(i) * 0.5, Drum::Hat));
    }
    p
}

/// A synthetic drum loop (kick, snare and hat from [`crate::kit`]) at `bpm`
/// for `bars` bars of 4/4, starting `offset` seconds in, with every hit
/// moved by up to `jitter` seconds either way. Returns the audio and the
/// true hit times.
pub fn drum_loop(
    bpm: f64,
    bars: usize,
    which: usize,
    offset: f64,
    jitter: f64,
    seed: u64,
    sample_rate: u32,
) -> (Vec<f32>, Vec<LoopHit>) {
    render_loop(bpm, bars, which, offset, jitter, false, seed, sample_rate)
}

/// Like [`drum_loop`], but drums meant for the same moment move together,
/// the way one beatboxed sound covers a kick and a hat.
pub fn drum_groove(
    bpm: f64,
    bars: usize,
    which: usize,
    offset: f64,
    jitter: f64,
    seed: u64,
    sample_rate: u32,
) -> (Vec<f32>, Vec<LoopHit>) {
    render_loop(bpm, bars, which, offset, jitter, true, seed, sample_rate)
}

#[allow(clippy::too_many_arguments)]
fn render_loop(
    bpm: f64,
    bars: usize,
    which: usize,
    offset: f64,
    jitter: f64,
    per_slot: bool,
    seed: u64,
    sample_rate: u32,
) -> (Vec<f32>, Vec<LoopHit>) {
    use crate::kit::{Sound, drum_hit};
    let mut rng = Rng::new(seed);
    let spb = 60.0 / bpm;
    let sr = f64::from(sample_rate);
    let mut hits = Vec::new();
    let mut slot_jitter: Vec<(f64, f64)> = Vec::new();
    for bar in 0..bars {
        for &(b, drum) in &pattern(which) {
            let beat = bar as f64 * 4.0 + b;
            let j = if per_slot {
                match slot_jitter.iter().find(|(sb, _)| *sb == beat) {
                    Some(&(_, j)) => j,
                    None => {
                        let j = f64::from(rng.bipolar()) * jitter;
                        slot_jitter.push((beat, j));
                        j
                    }
                }
            } else {
                f64::from(rng.bipolar()) * jitter
            };
            let t = offset + beat * spb + j;
            hits.push(LoopHit {
                time: t.max(0.0),
                beat,
                drum,
            });
        }
    }
    hits.sort_by(|a, b| a.time.total_cmp(&b.time));
    let len = ((offset + bars as f64 * 4.0 * spb + 0.5) * sr) as usize;
    let mut out = vec![0.0f32; len];
    for (i, h) in hits.iter().enumerate() {
        let (sound, vel) = match h.drum {
            Drum::Kick => (Sound::Kick, 1.0),
            Drum::Snare => (Sound::Snare, 0.9),
            Drum::Hat => (Sound::HatClosed, 0.45 + 0.1 * rng.bipolar().abs()),
        };
        let x = drum_hit(sound, vel, sample_rate, seed.wrapping_add(i as u64));
        let at = (h.time * sr).round() as usize;
        for (k, v) in x.iter().enumerate() {
            if let Some(o) = out.get_mut(at + k) {
                *o += v * 0.6;
            }
        }
    }
    (out, hits)
}

/// A beatboxed-sounding hit, deliberately unlike the kit's drums: a lip
/// "b" (low thump), a "pf"/"k" (mid noise burst with a little body) or a
/// "ts" (high hiss), each with seeded random pitch, brightness, length and
/// breath noise.
pub fn beatbox_hit(drum: Drum, seed: u64, sample_rate: u32) -> Vec<f32> {
    use crate::dsp::Biquad;
    let mut rng = Rng::new(seed.wrapping_add(drum as u64 * 104_729));
    let sr = sample_rate as f32;
    let u = |r: &mut Rng| 0.5 + 0.5 * r.bipolar();
    let level = 0.4 + 0.5 * u(&mut rng);
    let breath = 0.002 + 0.01 * u(&mut rng);
    type Voice = Box<dyn FnMut(f32, &mut Rng) -> f32>;
    let (len, mut voice): (f32, Voice) = match drum {
        Drum::Kick => {
            let f0 = 55.0 + 60.0 * u(&mut rng);
            let decay = 0.04 + 0.06 * u(&mut rng);
            let burst = 0.1 + 0.25 * u(&mut rng);
            let mut lp = Biquad::lowpass(sr, 400.0 + 400.0 * u(&mut rng), 0.7);
            let mut phase = 0.0f32;
            (
                0.25,
                Box::new(move |t, r| {
                    let f = f0 * (1.0 + 1.5 * (-t / 0.012).exp());
                    phase = (phase + f / sr).fract();
                    let body = (2.0 * PI * phase).sin() * (-t / decay).exp();
                    let pop = lp.process(r.bipolar()) * burst * (-t / 0.006).exp();
                    body + pop
                }),
            )
        }
        Drum::Snare => {
            let centre = 1_000.0 + 2_000.0 * u(&mut rng);
            let decay = 0.03 + 0.07 * u(&mut rng);
            let body_hz = 140.0 + 120.0 * u(&mut rng);
            let body_amt = 0.1 + 0.3 * u(&mut rng);
            let mut bp = Biquad::bandpass(sr, centre, 0.6 + 0.6 * u(&mut rng));
            let mut phase = 0.0f32;
            (
                0.25,
                Box::new(move |t, r| {
                    phase = (phase + body_hz / sr).fract();
                    let noise = bp.process(r.bipolar()) * 2.0 * (-t / decay).exp();
                    let body = (2.0 * PI * phase).sin() * body_amt * (-t / 0.03).exp();
                    noise + body
                }),
            )
        }
        Drum::Hat => {
            let cut = 4_000.0 + 3_000.0 * u(&mut rng);
            let decay = 0.012 + 0.05 * u(&mut rng);
            let mut hp = Biquad::highpass(sr, cut, 0.7);
            let mut hp2 = Biquad::highpass(sr, cut, 0.7);
            (
                0.2,
                Box::new(move |t, r| {
                    hp2.process(hp.process(r.bipolar())) * 1.5 * (-t / decay).exp()
                }),
            )
        }
    };
    let n = (len * sr) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let b = breath * rng.bipolar();
            level * voice(t, &mut rng) + b
        })
        .collect();
    // 2 ms fade-in and 15 ms fade-out so the hit itself has no clicks.
    let fi = (0.002 * sr) as usize;
    for (i, s) in out.iter_mut().take(fi).enumerate() {
        *s *= i as f32 / fi as f32;
    }
    let fo = (0.015 * sr) as usize;
    for k in 0..fo {
        out[n - fo + k] *= 1.0 - (k + 1) as f32 / fo as f32;
    }
    out
}

/// A beatboxed pattern: [`beatbox_hit`]s at the given times (seconds).
pub fn beatbox(hits: &[(f64, Drum)], seed: u64, sample_rate: u32) -> Vec<f32> {
    let end = hits.iter().map(|h| h.0).fold(0.0, f64::max) + 0.5;
    let mut out = vec![0.0f32; (end * f64::from(sample_rate)) as usize];
    for (i, &(t, d)) in hits.iter().enumerate() {
        let at = (t * f64::from(sample_rate)).round() as usize;
        for (k, v) in beatbox_hit(d, seed.wrapping_add(i as u64), sample_rate)
            .iter()
            .enumerate()
        {
            if let Some(o) = out.get_mut(at + k) {
                *o += v;
            }
        }
    }
    out
}
