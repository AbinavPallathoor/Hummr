# Status: recording-first rework

Branch `recording-first` (from `main` at tag `v0.1-hum-to-midi`).
Legend: **done** / **partial** (what is missing) / **skipped** (why) / todo.

| Item | State | Note |
|------|-------|------|
| M0 Setup | done | tag + branch pushed, baseline 105 tests green, clippy clean |
| M1 Project model | done | `project` module (Song/Track/Clip, lanes, split, move), TOML `format = 1` with round-trip tests, mixdown with hand-computed-sum test, undo/redo, `hummr new`, `hummr export` (.wav/.mid). Speed/pitch/effects are wired but their DSP lands in M4/M5 |
| M2 Onsets and tempo | todo | |
| M3 Slice, snap, match | todo | |
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
