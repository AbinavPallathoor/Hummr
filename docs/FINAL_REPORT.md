# Final report: recording-first rework

Branch `recording-first`, cut from `main` at tag `v0.1-hum-to-midi`. It has
23 commits, one or more per milestone, all pushed. `main` was not touched
and nothing was merged.

Every commit passed `cargo build --all-targets`, `cargo test` and
`cargo clippy --all-targets -- -D warnings`. The suite now has 254 tests:

| Crate | Tests |
|---|---|
| core | 147 |
| timeline/notes UI | 90 |
| CLI | 17 |

All milestones M0 to M11 are **done**. Partial items inside them are listed
under "What is partial or skipped".

## What works

- **Recording-first timeline.**
  - Tracks of clips with waveforms, a ruler, a playhead, overlap lanes and
    zoom.
  - Recording uses a one-bar count-in while the song plays.
  - Editing: split, delete, copy, paste, duplicate, grid moves and moves
    between tracks.
  - Undo and redo for everything.
  - Mute, solo and track commands, plus a sample picker with 8 built-in
    sounds and your WAVs (mp3/flac/ogg with `--features decode`).
- **Projects.** Plain-text `.hummr` files with a `sources/` folder. Your
  recordings are never modified. `hummr new` and `hummr export` (WAV mix
  or MIDI) are available.
- **Tempo.**
  - Hit and beat detection with a confidence in plain words
    (`hummr detect`).
  - The first loop sets the song tempo.
  - A loop at a tempo more than 2% off asks: Match to song, Change song
    to N, or Leave as is. Keys switch between half and double the tempo.
  - Header editing: `:bpm`, `<` `>`, tap tempo `T`, `:time`, `:key` and
    `:snap`.
- **Snapping.** Cut at the hits and snap them with a strength
  (`hummr snap`). Snapped clips follow tempo changes.
- **Sound shaping.**
  - WSOLA time-stretch and pitch-shift.
  - 12 effects, plus 8 presets with an amount.
  - Noise cleanup (`hummr clean`).
- **Conversions ("Turn into").**
  - Notes on 14 instruments.
  - Your own sound as the instrument.
  - Beatbox to drums.
  - All are reversible.
- **Inspector.** Opens on a clip, has three panels, and loops the clip
  while you change it. **Mixer** (`M`) has meters, volume, pan, mute and
  solo.
- **SoundFont instruments.** Played via rustysynth from
  `HUMMR_SOUNDFONT` or a config file.
- **Notes editor hand-off.** `e` opens a clip in the 0.1 piano roll; `Esc`
  writes the notes back.
- **Easy start.**
  - `hummr` alone opens the song in the current folder, or a starter song
    with a beat and a three-step guide.
  - The `?` help scrolls.
  - Microphone problems produce plain messages.
- **Animations.** Fades, a new-clip flash and smooth scrolling;
  `:anim off` turns them off.
- **`hummr demo`** writes 21 WAVs and an `ABOUT.txt` into `./demo/`.
- **Docs.** README, USAGE, ARCHITECTURE, LIMITS, CHANGELOG and four
  timeline screenshots.

## Measured results for each acceptance test

All measurements are on seeded synthetic signals, by tests in the repo.
"Limit" is the requirement in the brief.

| Area | Test | Result |
|---|---|---|
| M1 project | Two overlapping tracks mix to the hand-computed sum; the TOML file round-trips unchanged | pass |
| M2 tempo | 75 loops at 60–180 BPM with ±15 ms jitter | 51/51 within 1% in 80–160 BPM (worst 0.31%). The 4 misses (60–75 BPM) offer the right tempo as the first alternative. |
| M2 tempo | Worst phase error | 15 ms |
| M2 tempo | Noise confidence | 0.00 |
| M2 tempo | Random hits | confidence ≤ 0.34 |
| M3 snapping | Strength 1, 4 loops at 84–140 BPM | worst hit 0.47 ms from the grid; energy change ≤ 0.04 dB |
| M3 snapping | Strength 0 | max difference < 1e-5 |
| M4 stretch | Pitch kept, ratios 0.5–2 | within 0.6 cents (limit 20) |
| M4 shift | ±12 semitones | within 0.5 cents (limit 15) |
| M4 stretch | Length and speed | length exact; 84–244× real time |
| M5 effects | Amount 0 is dry; compressor ratio; limiter holds −1 dBFS; presets finite, bounded and deterministic | pass |
| M6 cleanup (strength 0.8) | Hiss floor (−30/−40/−50 dBFS) | down 15.5–16.2 dB (limit ≥ 12) |
| M6 cleanup | Tone change | ≤ 0.01 dB (limit ≤ 2) |
| M6 cleanup | Hum at 50/150 Hz | down ≥ 12 dB |
| M7 conversions | Hummed note root pitch | within 5.6 cents (limit 25) |
| M7 conversions | Sampler at ±12 st | within 0.4 cents |
| M7 conversions | Drum classification | 100% on 300 beatbox-style + 100 kit hits (limit 90%) |
| M8 SoundFont | A4 and E5 from a generated .sf2 | within 0.3% |
| M8 SoundFont | Missing font | falls back with a message |
| M9a layout | Text snapshots | 80×24, 100×30, 120×40, with an overlap lane |
| M9b–c keys | Key-driven test for every required key | pass |
| M9c overlap | Overlap creates a second lane, which collapses and is restored by undo | pass (snapshot) |
| M9c split | Split then play vs. the original | worst difference < 1e-6 |
| M9d inspector | Match tempo, 96 → 120 BPM | clip ends at 8 beats ± 0.05 |
| M9d inspector | Slider steps | one undo step per run |
| M9e re-timing | Cut-and-snapped loop re-timed to 96/120/80 BPM | every hit within 0.31/1.00/1.17 ms of the 1/8 grid |
| M9e tap tempo | Taps 0.5/0.51/0.49/0.9/0.5 s | 120 BPM |
| M9f prompt | 104 BPM loop into an empty song | sets 104 ("sure") |
| M9f prompt | Half-filled beats | "100 BPM or 50? not sure" (confidence 0.37) |
| M9f prompt | 120 BPM loop into a 96 BPM song | prompt, half/double switch, match, change song and Esc all checked |
| M9f prompt | 97 and 192 BPM loops, and one-shots | no prompt |
| M9g mixer | Meter fall | −12.0 dB after 0.5 s (24 dB/s) |
| M9g mixer | Strip edits; snapshot | one undo step per run; snapshot at 100×30 |
| M9h animations | Effects start, end, and are skipped with `:anim off` | pass |
| M9h animations | Animated playhead | stays within 1 column of the one-third mark |
| M9i notes editor | Edits made in the 0.1 editor | come back exactly; one undo restores them |
| M10 easy start | Starter in an empty folder | snapshot at 80×24 |
| M10 easy start | Guide, saving, reopening | guide ticks; nothing is written before `:w`; the saved song reopens |
| M10 easy start | Several projects in a folder | the most recent of two opens |
| M10 easy start | Help on small and wide screens | scrolls at 80×24; two columns at 120×40 |
| M11 demo | 21 files | non-silent, finite, peaks −12.0 to −1.0 dBFS |
| M11 demo | Cleanup on the hiss-only lead-in | 23.8 dB lower |
| Responsiveness | Timeline draw at 120×40 (release) | 0.076 ms per frame (TestBackend, excludes terminal output) |
| Responsiveness | Cold render of every track of the 10 s demo song (release) | 23 ms, on the background thread |

## What is partial or skipped, and why

- **No microphone/speaker latency compensation.** Takes can land a few
  milliseconds late. Measuring the round trip needs real hardware, which
  isn't available here. Documented in LIMITS.
- **No limiter during playback.** Playback uses the player's soft clip;
  the −1 dBFS limiter runs on export only. This keeps the audio callback
  light. Documented.
- **Real sound devices are not exercised by tests.** None are available
  here. The logic around them is tested: count-in timing, take trimming,
  the render thread, and the preview hand-off.
- **mp3 and ogg import are not tested.** Only FLAC decoding has a test.
  They sit behind the optional `decode` feature.
- **No SoundFont is bundled.** None of the free GM fonts has a suitable size
  and licence. The 0.1 notes editor always plays with the built-in synth.
- **Inspector preview.** It plays the clip without the track's effects.
  Inside the notes editor, own-sound and drums clips use a stand-in
  instrument, and only the clip's own part comes back.
- **No mouse on the timeline.** The 0.1 editor's `--mouse` still works
  there. The brief did not require mouse support for the timeline.
- **The tempo prompt skips clips under 1.5 s.** These are one-shots with no
  beat to measure.
- **Nothing is released or merged.** The version number and the merge into
  `main` are left to you.

## Known limits

The full list with numbers is in `docs/LIMITS.md`. The main ones:

- Stretching blurs drum hits, so rhythmic clips are cut and snapped
  instead.
- Big pitch shifts change the voice's character; there is no formant
  correction.
- Tempo detection is reliable for 80–160 BPM.
- Cleanup handles only steady noise.
- Conversions to notes follow one note at a time.
- A song has one tempo.

## Try it

```sh
git checkout recording-first
cargo build --release
cargo test && cargo clippy --all-targets -- -D warnings

./target/release/hummr demo            # 21 WAVs + ABOUT.txt in ./demo/
./target/release/hummr new "First Song"
./target/release/hummr first-song/first-song.hummr
# or, in an empty folder:  hummr   (starter song with a beat and a guide)
```

Optional: `cargo build --release --features decode` adds mp3/flac/ogg
import, and `export HUMMR_SOUNDFONT=/path/GeneralUser.sf2` gives real
instrument sounds.

## Listen to these first

1. `demo/00-song.wav`: a snapped beatbox beat, a hummed bass turned into a
   bass guitar, and the hummed melody with the Dreamy preset.
2. `demo/01-beatbox-sloppy.wav`, then `demo/02-beatbox-snapped.wav`:
   snapping, against a steady hi-hat.
3. `demo/03-tempo-left-as-is.wav`, `04-tempo-cut-and-snap.wav`,
   `05-tempo-stretched.wav`: the three answers to the tempo prompt.
4. `demo/20-clean-before.wav` and `21-clean-after.wav`.
5. `demo/32-beatbox-original.wav`, then `33-turn-into-drums.wav`; and
   `30-turn-into-notes.wav` and `31-turn-into-own-sound.wav`.
6. `demo/10-preset-none.wav` against `11`–`18`, the eight presets.

## Decisions you are most likely to want to change

From `docs/DECISIONS.md`:

1. **`hummr` alone opens the timeline,** not the 0.1 editor. The old
   editor is `hummr edit take.wav`.
2. **Snap defaults.** "Match to song" cuts and snaps rhythmic clips to 1/8
   notes at full strength (rhythmic means ≥ 2 hits/s and confidence ≥ 0.3).
   `hummr snap` defaults to 1/16.
3. **The 2% tempo threshold** for asking. No prompt for half/double tempo
   or clips under 1.5 s.
4. **Noise sample.** It defaults to the clip's first 300 ms, as the brief
   says. A take that starts singing at once will learn some voice as
   "noise".
5. **No SoundFont is bundled.** Whether to ship or auto-download one is a
   licensing call.
6. **Playback soft clip vs. export limiter.** The app can sound slightly
   different from the exported file on very loud mixes.
7. **Undo stores whole-song snapshots.** This is simple and robust; it
   would only matter for very large songs.
8. **Starter-song choices.** It stays in memory until `:w`, and the guide
   is tied to the starter song rather than to a "first run" marker file.
9. **Mixer placement.** The mixer is a strip area under the tracks rather
   than its own screen.
10. **Sources are mono at 48 kHz.** Stereo imports are downmixed.
