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
- Onsets: log-magnitude spectral flux (SuperFlux-style max filter, 2.7 ms hop) with peaks refined to the attack on a >500 Hz envelope — plain energy onsets — flux finds soft hits; the high-passed refinement keeps hats on top of a ringing kick accurate to <1 ms.
- Tempo: autocorrelation of a body-weighted flux curve × a log-normal prior at 115 BPM, refined by a comb fit and a least-squares line through on-beat hits, then a two-way octave check on a *power* flux curve (halve if alternate pulses are < 0.3 as loud, double if the half-speed grid misses hits and the doubled pulse is even) — prior alone — the prior alone got 160 BPM loops as 80 and 60 BPM loops as 120.
- Synthetic test loops are rock, syncopated-kick and four-on-the-floor patterns — also a half-time pattern with kicks on off-beats — that half-time pattern is genuinely ambiguous (a drummer could count it either way), so it can't fairly define "correct".
- Beat confidence = (hit strength on the 8th grid beyond chance) × (fraction of beats that have a hit) — grid alignment only — fitting a tempo to random hits made grid alignment alone look "sure" (0.54); coverage drops random hits to ≤ 0.34.
- Slices are cut 4 ms before each attack with 5 ms complementary crossfades; each slice stores its attack offset and its unsnapped position so the snap strength is re-computable — destructive snapping — the inspector's strength slider must be reversible.
- Snapping lines up a slice's *attack* (not its cut point) with the grid — snap cut points — the attack is what you hear as "on the beat".
- A sliced clip moves by under half a beat so its own beats sit on song beats — keep its start — otherwise a perfectly cut loop can still be off the song's grid.
- Attack refinement walks back from the peak until the envelope stays quiet for 1.5 ms — stop at the first quiet block — noisy snares dip inside the hit and were placed 13 ms late.
- The M3 test loops jitter per time slot (a kick and hat meant together move together); the M2 tempo test keeps per-drum jitter — per-drum jitter everywhere — independent jitter makes 30 ms flams that no slicer can treat as one hit, and a beatboxed sound covers both drums at once.
- "Match to song" defaults: rhythmic clips (≥ 2 hits/s and confidence ≥ 0.3) are cut and snapped to 1/8 at full strength; others keep their sound — always stretch — matches the brief, and cutting keeps drums crisp.
- `hummr snap` defaults to a 1/16 grid and strength 1 — 1/8 — beatbox hats are often 16ths; 1/8 would pull them onto the wrong line.
