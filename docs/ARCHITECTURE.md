# Architecture

Three crates in one workspace:

| Crate | What it is | Depends on |
|---|---|---|
| `hummr-core` | All audio and music logic: the project model, analysis, DSP, rendering, playback and recording. No UI. | `cpal`, `hound`, `midly`, `realfft`, `serde`/`toml`, `rustysynth`, `thiserror`; optional `symphonia` (`decode` feature) |
| `hummr-tui` | The terminal app: the timeline ("studio") and the 0.1 notes editor. | `hummr-core`, `ratatui`, `tachyonfx` |
| `hummr-cli` | The `hummr` binary: argument parsing and the scriptable commands. | `hummr-core`, `hummr-tui`, `clap` |

Everything runs offline. Network access is only needed by cargo, to fetch
crates at build time.

## The project model (`hummr-core::project`)

```
Project ─┬─ Song (plain data, serialised to song.hummr)
         │   ├─ bpm, time_sig, key, snap
         │   ├─ sources: [SourceInfo]      id → sources/<file>.wav
         │   └─ tracks: [Track]
         │        └─ clips: [Clip]         source region + position + settings + kind
         └─ SourceBank                     decoded audio, Arc<AudioBuffer> per source id
```

- A `Clip` references a region of a source (seconds) and sits at
  `start_beat`. Its settings (gain, pan, pitch, speed, fades, reverse,
  repeat, effects, noise cleanup, stretch-follow) are applied at render
  time; sources are never modified. `ClipKind` is `Audio`, `Sliced` (cut
  points and where each slice plays, in beats) or `Notes` (notes in beats
  plus a `Voice`: a GM instrument, the clip's own sound, or drums).
- Positions are in beats, so sliced and notes clips re-time for free when
  the tempo changes; audio clips keep their length in seconds unless they
  follow the tempo.
- Overlaps are shown, not prevented: `Song::lanes` packs a track's clips
  greedily into lanes (first lane where it fits), recomputed on demand.
- `History` keeps whole-`Song` snapshots for undo/redo. A song is small
  (no audio), so a snapshot per edit is cheap and can't get out of sync.
- `file.rs` saves atomically (temp file + rename), writes new sources as
  WAV into `sources/`, copies them when saving elsewhere, and decodes
  everything to 48 kHz on open. Missing sources become silence with a
  warning instead of failing the open.

## Rendering (`project::render`, `export`)

```
source region ─► cleanup ─► kind ─► speed/stretch ─► pitch ─► fades/reverse ─► clip effects ─► gain/pan ─┐
   (Audio: the region; Sliced: slices placed on beats; Notes: the voice plays the notes)                 │
                                                       track = Σ clips, then track effects ◄─────────────┘
                                                       mix   = Σ tracks × volume/pan/mute/solo ─► limiter (export)
```

- `render_clip` renders one clip to mono at 48 kHz. `ClipCache` caches the
  result keyed by everything that affects the sound (not the clip's id,
  name or position) plus the tempo, so moving or duplicating a clip costs
  no DSP.
- `render_track_cached` places a track's clips and applies its effects.
  Volume, pan, mute and solo are left to the mixer.
- `export` mixes all tracks and runs the −1 dBFS look-ahead limiter, or
  writes the notes clips as MIDI.

## Analysis and DSP modules (`hummr-core`)

| Module | |
|---|---|
| `onset` | Hit detection: spectral flux with adaptive peak picking, attack refined to the sample |
| `beat` | Tempo (autocorrelation of the onset curve plus grid fit), beat phase, downbeat, confidence, half/double alternatives, plain-words description |
| `slice` | Cut at hits, place slices on the grid with a strength, 5 ms crossfades |
| `project::ops` | Tempo analysis of a clip, "is it rhythmic", and the three match modes (cut and snap, stretch, keep) |
| `stretch` | WSOLA time-stretch and pitch-shift |
| `fx` | Gain, pan, fades, reverse, filters, EQ, distortion, bitcrush, tempo-synced echo, Freeverb-style reverb, compressor, limiter; presets with an amount |
| `denoise` | Spectral gating learned from a noise-only region |
| `project::convert`, `voice`, `drums` | Turn into notes / own sound / drums and back; voices behind the `InstrumentVoice` trait |
| `soundfont` | GM instruments from a `.sf2` via rustysynth, falling back to the built-in `preview` synth |
| `kit` | Procedural starter sounds (drums, 808, pluck, pad) |
| `pitch`, `segment`, `theory`, `rhythm`, `transcribe` | The 0.1 hum-to-notes pipeline (pYIN, note HMM, key finding, quantising), used by "turn into notes" |
| `audio` | `cpal` microphone capture (`Recorder`) and playback (`Player`) |
| `synthetic` | Seeded test signals: hums, beatbox, drum loops, noise; used by tests and `hummr demo` |

## Playback (`hummr-tui::studio::engine`)

```
UI thread                         render thread                     audio callback (cpal)
─────────                         ─────────────                     ─────────────────────
edit → Song changed ──sync()──►  job per changed track ──►         Player: per-track stereo buffers,
                                  render_track_cached               live gain/pan/mute, soft clip,
       poll() ◄── rendered track + rendered clips (waveforms)       peaks for the meters
       replace_track(i) ───────────────────────────────────────────►
```

- `Engine::sync` compares each track's sound-affecting settings with what
  was last rendered and sends only changed tracks to one background
  thread. Newer jobs for the same track replace queued ones.
- Finished tracks are swapped into the running `Player` one at a time, so
  playback never stops while you edit. Volume, pan, mute and solo are
  player parameters and change instantly.
- The inspector preview sends a one-clip "song" to the same thread and
  loops it on the player; the song's tracks are rendered again (mostly
  from the clip cache) when the inspector closes.
- Without a sound device the engine keeps time on a simulated clock, which
  is also how tests run.

## The timeline UI (`hummr-tui::studio`)

| Module | |
|---|---|
| `mod.rs` | `Studio` state: project, history, view, selection, modes; `tick` (engine sync, polling, meters); `edit`/`edit_merge` (undo steps) |
| `keys.rs`, `command.rs` | Key handling per mode, and the `:` commands |
| `draw.rs` | Everything drawn from state: header, ruler, rows, clip boxes, overlays, mixer strips, guide, animations |
| `peaks.rs` | Waveform glyphs per clip and zoom, from the source until the rendered clip arrives |
| `edit.rs`, `song_edit.rs` | Clip and track edits; tempo, time, key, snap, tap tempo |
| `record.rs`, `import.rs`, `tempo_prompt.rs` | Recording with count-in; the sample picker; the tempo question for new samples |
| `inspector.rs`, `mixer.rs`, `notes.rs`, `start.rs` | Inspector panels; mixer; the notes-editor hand-off; starter song and guide |
| `fixture.rs`, `tests.rs`, `snapshots/` | The synthetic demo song, key-driven tests and insta text snapshots |

Every action is a method on `Studio`, and keys only call methods, so tests
drive the app by typing keys (`press(&mut s, "<cr>2jjjll")`) and check the
song, the messages and the text of the rendered screen.

The 0.1 editor (`app.rs`, `ui.rs`, `input.rs`) is unchanged apart from an
`embedded` flag used when the timeline opens it for one notes clip.

## Testing

- DSP tests run on seeded synthetic signals and assert measured numbers
  (cents, milliseconds, dB), listed in `docs/STATUS.md` and
  `docs/LIMITS.md`.
- Timeline tests are key-driven, plus insta snapshots of the whole screen
  at 80×24, 100×30 and 120×40.
- CLI tests run the built binary on temporary folders.
- Real speakers and microphones can't be tested in CI; their code paths
  are kept thin, and the logic around them (count-in timing, take
  trimming, the render thread) is tested without hardware.
