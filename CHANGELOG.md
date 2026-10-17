# Changelog

## Unreleased: recording first (branch `recording-first`)

Hummr becomes a small recording studio: record or import anything, then
fix and transform it. The 0.1 hum-to-MIDI editor is kept as the notes
editor.

### Added

- **Timeline** (the new default screen): tracks of clips with waveforms, a
  beat ruler and playhead, overlap lanes, zoom, and every edit undoable.
  Recording with a one-bar count-in while the song plays; split, delete,
  copy, paste, duplicate, grid moves and track moves; mute, solo, new and
  renamed tracks; a sample picker with eight built-in sounds and WAV files
  (mp3/flac/ogg with `--features decode`).
- **Projects**: `.hummr` TOML files with a `sources/` folder; recordings are
  never modified. `hummr new`, `hummr export` (WAV mix with a −1 dBFS
  limiter, or MIDI from the notes clips).
- **Tempo**: hit and beat detection with a plain-words confidence
  (`hummr detect`); the first imported loop sets the song tempo; loops at
  another tempo ask to match, change the song, or leave them. `:bpm`,
  `<`/`>`, tap tempo (`T`), `:time`, `:key`, `:snap`.
- **Snapping**: cut a clip at its hits and move them onto the grid with a
  strength (`hummr snap`); snapped clips follow tempo changes.
- **Time-stretch and pitch-shift** (WSOLA): clip speed, pitch and
  "follow song tempo".
- **Effects**: twelve effects and eight presets (Lo-fi, Dreamy, Gritty,
  Telephone, Radio, Big hall, Warm, Bright) with an amount.
- **Noise cleanup** for steady hiss and hum (`hummr clean`).
- **Turn into**: notes on one of 14 instruments, your own sound as the
  instrument, or drums from a beatbox; reversible.
- **SoundFont instruments** via rustysynth (`HUMMR_SOUNDFONT` or
  `~/.config/hummr/config.toml`), falling back to the built-in synth.
- **Inspector** (`Enter` on a clip) with Turn into, Effects and Fix panels
  and a looping preview.
- **Mixer** (`M`): level meters, volume, pan, mute and solo per track, and
  a master meter.
- **Notes editor hand-off**: `e` opens a notes clip in the 0.1 piano roll;
  `Esc` brings the edits back as one undo step.
- **Easy start**: `hummr` alone opens the song in the current folder or a
  starter song with a beat and a three-step guide; `?` help in plain words
  with scrolling; clearer microphone messages.
- **Animations** (tachyonfx): overlay and message fades, a flash on new
  clips, smooth scrolling while playing; `:anim off` turns them off.
- `hummr demo` writes 21 example WAVs into `./demo/`.
- Docs: README rewrite, `docs/USAGE.md`, `docs/ARCHITECTURE.md`,
  `docs/LIMITS.md`, this changelog.

### Changed

- `hummr` with no arguments now opens the timeline instead of the hum-to-MIDI
  editor. That editor is still `hummr edit take.wav` / `hummr edit song.mid`.

## 0.1: hum to MIDI (tag `v0.1-hum-to-midi`)

- Hum or sing into the microphone and get notes: pYIN pitch tracking, note
  segmentation, key detection and snapping, tempo and quantising, with an
  explanation of every change (`hummr transcribe --explain`).
- A piano-roll editor with layered recording, re-recording, vim-style note
  editing, a mixer, multi-track MIDI save and load, and WAV export.
