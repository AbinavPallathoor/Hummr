# Status: recording-first rework

Branch `recording-first` (from `main` at tag `v0.1-hum-to-midi`).
Legend: **done** / **partial** (what is missing) / **skipped** (why) / todo.

| Item | State | Note |
|------|-------|------|
| M0 Setup | done | tag + branch pushed, baseline 105 tests green, clippy clean |
| M1 Project model | done | `project` module (Song/Track/Clip, lanes, split, move), TOML `format = 1` with round-trip tests, mixdown with hand-computed-sum test, undo/redo, `hummr new`, `hummr export` (.wav/.mid). Speed/pitch/effects are wired but their DSP lands in M4/M5 |
| M2 Onsets and tempo | done | `onset` (spectral flux + attack refinement), `beat` (BPM, phase, downbeat, confidence, half/double), `kit` (procedural sounds), `hummr detect`. Measured on 75 jittered (±15 ms) loops 60–180 BPM: 51/51 within 1% in 80–160 (worst 0.31%), the 4 misses (60–75 BPM) have the right tempo as first alternative, worst phase error 15 ms; noise confidence 0.00, random hits ≤ 0.34 (16 seeds) |
| M3 Slice, snap, match | done | `slice` (cut at attacks, snap with strength, 5 ms crossfades, beat-anchored), `project::ops` (analyse, rhythmic?, match: cut-and-snap / stretch / keep), `hummr snap`. Measured: strength 1 → worst hit 0.47 ms from the grid (4 loops 84–140 BPM), energy change ≤ 0.04 dB, max sample jump ≤ the original's; strength 0 → max difference < 1e-5; moved sine slices: max jump 0.0165 vs 0.0146 for the tone itself |
| M4 Stretch and pitch-shift | todo | |
| M5 Effects | todo | |
| M6 Noise cleanup | todo | |
| M7 Conversions | todo | |
| M8 SoundFont voice | todo | |
| M9a Timeline layout | todo | |
| M9b Navigation keys | todo | |
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
