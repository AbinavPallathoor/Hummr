# Decisions

One line each: decision — alternatives — why.

- Branch `recording-first` is cut from `main` (which equals `beat-first`) — continue on `beat-first` — the brief says to branch from `main`; both point at the same commit.
- Project file uses serde 1.0.229 + toml 1.1.8 (both released Jul/Oct 2026, heavily used) — hand-written TOML writer/parser — derive-based (de)serialisation is far less code and less error-prone.
- Internal sample rate is fixed at 48 kHz; every source is converted on load with a windowed-sinc resampler — render at each source's own rate — one rate makes clip maths sample-exact, and 48 kHz is what most current audio hardware runs at.
- Sources are mono (stereo files are downmixed) — keep stereo sources — the whole engine (pitch tracking, slicing, stretching) is mono; stereo comes from pan and stereo effects. Revisit if people import stereo loops.
- `hummr new NAME` creates `NAME/NAME.hummr` plus `NAME/sources/` — `NAME.hummr` in the current folder — a folder per song keeps sources from colliding and makes a project easy to move or zip.
- Imported WAVs are copied byte for byte into `sources/`; in-app recordings are written as 32-bit float WAV — always re-encode — byte copies guarantee "never edit originals".
- Timeline positions are in beats, source regions in seconds — everything in seconds — beats make tempo changes re-time sliced/notes clips for free.
- Undo/redo stores whole-song snapshots — diffs/commands — a Song holds no audio (sources are Arc buffers outside it), so a snapshot is a few KB and snapshots can't get out of sync.
- Clip "speed" means time-stretch that keeps pitch, "pitch" means pitch-shift that keeps length — tape-style varispeed — that's what non-musicians expect from two separate sliders.
- Effects on a clip/track are: a named preset with an amount macro, four friendly sliders (drive, filter, echo, reverb) and a list of raw effects — one flat list of effects — matches "presets first, sliders, raw parameters one level down".
- Track pan is a linear balance (centre = unity), clip pan is constant-power — constant-power everywhere — tracks are already stereo after effects; balance keeps the centre level unchanged.
- Splitting a repeated clip first unrolls it into one clip per repeat — refuse, or a partial-repeat model — simple, predictable and keeps the model small.
- Clip fades default to 0 so split-then-play is sample-exact; sliced clips use 5 ms complementary crossfades at the cuts — always declick clip edges — exact splits are an acceptance requirement.
- A project whose source file is missing still opens; that sound is silent and a warning says which — refuse to open — losing access to the whole song over one file is worse.
- MIDI drum parts go on channel 10 (`SongTrack::drums`) — keep drums on a melodic channel — General MIDI players expect drums there.
