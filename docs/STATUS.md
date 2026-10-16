# Status: recording-first rework

Branch `recording-first` (from `main` at tag `v0.1-hum-to-midi`).
Legend: **done** / **partial** (what is missing) / **skipped** (why) / todo.

| Item | State | Note |
|------|-------|------|
| M0 Setup | done | tag + branch pushed, baseline 105 tests green, clippy clean |
| M1 Project model | done | `project` module (Song/Track/Clip, lanes, split, move), TOML `format = 1` with round-trip tests, mixdown with hand-computed-sum test, undo/redo, `hummr new`, `hummr export` (.wav/.mid). Speed/pitch/effects are wired but their DSP lands in M4/M5 |
| M2 Onsets and tempo | done | `onset` (spectral flux + attack refinement), `beat` (BPM, phase, downbeat, confidence, half/double), `kit` (procedural sounds), `hummr detect`. Measured on 75 jittered (±15 ms) loops 60–180 BPM: 51/51 within 1% in 80–160 (worst 0.31%), the 4 misses (60–75 BPM) have the right tempo as first alternative, worst phase error 15 ms; noise confidence 0.00, random hits ≤ 0.34 (16 seeds) |
| M3 Slice, snap, match | done | `slice` (cut at attacks, snap with strength, 5 ms crossfades, beat-anchored), `project::ops` (analyse, rhythmic?, match: cut-and-snap / stretch / keep), `hummr snap`. Measured: strength 1 → worst hit 0.47 ms from the grid (4 loops 84–140 BPM), energy change ≤ 0.04 dB, max sample jump ≤ the original's; strength 0 → max difference < 1e-5; moved sine slices: max jump 0.0165 vs 0.0146 for the tone itself |
| M4 Stretch and pitch-shift | done | WSOLA `time_stretch`, `pitch_shift`, `stretch_and_shift` (wired into clip speed/pitch/follow-tempo). Measured: length exact, pitch kept within 0.6 cents (ratios 0.5–2), shifts ±12 st within 0.5 cents, no NaN/inf, 84–244× real time. Artifacts documented in docs/LIMITS.md |
| M5 Effects | done | `fx`: gain, pan, fades, reverse, filters, 3-band EQ, distortion, bitcrush, tempo-synced echo, Freeverb reverb, compressor, look-ahead limiter; 8 presets (Lo-fi, Dreamy, Gritty, Telephone, Radio, Big hall, Warm, Bright) with an amount, plus drive/reverb/echo/filter sliders. Tested: amount 0 = dry, effect strength grows with amount, compressor matches its ratio, limiter holds −1 dBFS and is transparent below it, every preset finite, bounded and deterministic |
| M6 Noise cleanup | done | `denoise` (spectral subtraction learned from a marked region or the first 300 ms), clip `clean`/`noise_region` applied at render, `hummr clean`. Measured at strength 0.8: hiss floor down 15.5–16.2 dB (noise at −30/−40/−50 dBFS), 440 Hz tone changed ≤ 0.01 dB, noise under the tone down 14.1 dB, 50/150 Hz hum down ≥ 12 dB. Help text says steady hiss/hum only |
| M7 Conversions | done | `project::convert` (Turn into notes / own sound / drums / none, reversible, linked to the source region), `voice` (`InstrumentVoice`: preview synth, sampler, drum kit), `drums` classifier. Measured: root pitch of hummed notes within 5.6 cents (limit 25); sampler pitches ±12 st within 0.4 cents; drum classification 100% on 300 seeded beatbox-style hits and 100 kit hits (limit 90%); a 16-hit beatbox pattern converts with every hit within 0.01 beat; Twinkle hum → exactly the sung pitches |
| M8 SoundFont voice | done | `soundfont` (rustysynth behind `InstrumentVoice`, `HUMMR_SOUNDFONT` or `~/.config/hummr/config.toml`, plain fallback message). Measured with a generated test .sf2: A4 and E5 within 0.3% of the right frequency, notes start on time, release ends within the 4 s tail, renders deterministic; missing/broken files fall back to the built-in synth with a message (CLI test). Not bundled: no small GM font with an MIT-compatible license |
| M9a Timeline layout | done | `studio` module: `Screen` enum, header (BPM · time · key · snap), ruler with bar numbers and ▼ playhead, track headers with lanes (`↳ lane 2`), clip boxes with ▁…█ waveforms (cached per clip and zoom), double border for the selection, status and hint lines, "terminal too small" below 80×24. insta snapshots at 80×24, 100×30, 120×40 (with an overlap lane) |
| M9b Navigation keys | done | `studio::keys`/`command`/`engine`: h/l (snap step), j/k (rows and lanes), {/} bars, Home/End, +/−/0 zoom and fit, space play/stop, ? help, : commands (:w, :q, :q!, :wq, :export, :anim); background per-track rendering with a per-clip cache feeding the existing Player; `hummr edit x.hummr` opens the timeline. Key-driven tests for each, plus a render-thread test |
| M9c Clip editing | todo | |
| M9d Inspector | todo | |
| M9e Header editing | todo | |
| M9f Tempo-match prompt | todo | |
| M9g Mixer strips | todo | |
| M9h Animations | todo | |
| M9i Notes editor hand-off | todo | |
| M10 Easy start | todo | |
| M11 Docs and demos | todo | |
| Final report | todo | |
