# Hummr (`hummr`)

Hum or sing a part, and `hummr` turns it into notes for a real instrument.
It runs offline in your terminal and is built for people who are better at code
than at music: projects are plain text, they work with git, and everything is a
CLI command.

Written entirely in Rust. It needs no network access at runtime and runs on
macOS, Windows and Linux.

> **Status:** Transcription, recording and a first version of the terminal
> editor work. Playback currently uses a small built-in preview synth.
> SoundFont rendering, projects, watch mode, accompaniment and the beatbox
> classifier are next.

## Install

You need a Rust toolchain (1.85 or newer) from <https://rustup.rs>.

```sh
git clone https://github.com/AbinavPallathoor/Hummr.git
cd Hummr
cargo install --path crates/hummr-cli     # installs the `hummr` binary
```

### Platform notes

- **Linux:** audio recording and playback (coming in features 2 and 3) uses
  ALSA, so install the development headers first:
  `sudo apt install libasound2-dev pkg-config` (Debian/Ubuntu) or
  `sudo dnf install alsa-lib-devel` (Fedora).
- **macOS:** to record, your terminal app (Terminal, iTerm2, Ghostty, VS Code
  and so on) needs microphone permission. Turn it on under **System Settings →
  Privacy & Security → Microphone**. Without permission, macOS records
  silence instead of reporting an error, so `hummr` warns you when a take is
  silent.
- **Windows:** no extra setup is needed.

## Usage

### The editor

```sh
hummr                      # open the editor
hummr takes/take-001.wav   # open a recording (or a .mid file)
hummr --no-anim --mouse    # no animations; click and scroll with the mouse
```

Press **`r`** and hum. The pitch meter shows the note you're singing as you
sing it. Press **`r`** again to stop. Each take is saved to `./takes/` and
transcribed automatically. Then:

| Key | Action |
|---|---|
| `space` / `enter` | play everything / play from the selected note (your voice and the instrument, through the mixer) |
| `h` `l` | select the previous / next note |
| `j` `k`, `J` `K` | move the note down / up a semitone, or an octave |
| `<` `>`, `H` `L` | make the note shorter / longer, or move it earlier / later (by grid steps) |
| `x`, `u` | delete the note, undo |
| `i` | instrument picker (type to fuzzy-search, for example `vio` for violin) |
| `t` | switch between the note tracker and "why notes changed" |
| `tab` | move focus to the next panel. In the mixer: `j`/`k` volume, `[`/`]` pan, `m` mute, `s` solo |
| `:` | command line: `:w`, `:export song.wav`, `:key D minor`, `:tempo 100`, `:quantize 1/8`, `:instrument flute`, `:anim off`, `:q` |
| `?` | full help |

In the piano roll, lighter rows are notes in the key. The instrument you pick
is saved into the MIDI file. For now, playback and `:export *.wav` use a simple
preview synth; real SoundFont instruments come next. To turn animations off,
use `--no-anim`, set `HUMMR_NO_ANIM=1`, or type `:anim off`.

### Record from the command line

```sh
hummr record -o take.wav            # press Enter to stop
hummr record -o take.wav --seconds 8
```

### Transcribe a hummed take to MIDI

```sh
hummr transcribe take.wav --key auto --quantize 1/8 -o lead.mid --explain
```

```
take.wav  10.1 s · 44.1 kHz
  tuning   sung 27¢ flat on average; corrected
  key      C major  (detected, score 0.98; runner-up A minor 0.63)
  tempo    100 BPM  (detected, 1/8 grid, fit 0.97)
  notes    14

  C4 1/4 C4 1/4 G4 1/4 G4 1/4 A4 1/4 A4 1/4 G4 1/2 F4 1/4 F4 1/4 E4 1/4 E4 1/4
  D4 1/4 D4 1/4 C4 1/2

why notes changed:
    -    0.386s  length  held 13 notes through the short breaths before the next note (...)
  #4     2.155s  timing  start 2.155s → bar 1 beat 4 (+10 ms)
  ...
wrote lead.mid
```

| Option | Meaning |
|---|---|
| `--key auto` | Detect the key and snap out-of-key notes to the nearest in-key note. You can also pass a key (`"D minor"`, `F#m`, `Bb`, `"A minor pentatonic"`), or `none` to keep the nearest semitone. |
| `--quantize 1/8` | Snap notes to a grid: `1/4`, `1/8`, `1/16`, `1/8t` (triplets) or `off`. |
| `--tempo auto` | Detect the tempo from the note onsets, or give one (for example `--tempo 96`). |
| `--explain` | Show every change made: snapped pitches (for example `moved F#4 to F4: outside C major`), timing shifts, and dropped blips. |
| `--no-tuning` | Don't correct for singing consistently sharp or flat. |
| `--pitch-csv f.csv` | Write the frame-by-frame pitch track, which helps when debugging a take that comes out wrong. |

The note line uses the same notation as project files (feature 4):
`C4 1/4 E4 1/8 r 1/8`. That's a note name followed by its length as a
fraction of a whole note, and `r` is a rest.

**Input:** WAV files only for now, at any sample rate, mono or stereo. To
convert a phone recording, run `ffmpeg -i memo.m4a take.wav`.

**Tips for good takes:** hum `doo`, `da` or `la` rather than a closed `mmm`
when you want repeated notes to stay separate. Keep a steady tempo, and leave
a little silence at the start.

### Try it without a microphone

```sh
cargo run -p hummr-core --example synth_take -- take.wav 100   # synthetic hummed Twinkle Twinkle
hummr transcribe take.wav --explain
```

## How transcription works

1. **pYIN pitch tracking** (`hummr-core/src/pitch`). YIN runs with 100
   thresholds weighted by a Beta prior, which gives several candidate pitches
   per 5 ms frame. A Viterbi pass over a pitch/voicing HMM then chooses the
   smoothest plausible path, which removes octave jumps and glitches. It's
   implemented here because the only Rust pYIN crate is unmaintained and pulls
   in `ndarray` 0.15.
2. **Note segmentation** (`segment.rs`). A note-level HMM (one state per
   semitone plus silence) splits notes where the pitch changes, but keeps
   vibrato and scoops inside one note. RMS energy dips split repeated notes at
   the same pitch. If you sing consistently sharp or flat, that offset is
   estimated from note medians and removed.
3. **Key** (`theory.rs`). Krumhansl–Schmuckler key finding, then snapping each
   out-of-key note to the in-key note nearest to what was actually sung.
4. **Tempo and quantization** (`rhythm.rs`, `transcribe.rs`). The tempo is the
   one whose grid best fits the onsets (the circular resultant of onset phases,
   with a mild prior toward 105 BPM). Notes then snap to the grid, and short
   breaths between notes are treated as legato rather than rests.

## Development

```sh
cargo build
cargo test        # includes synthetic-audio DSP tests and golden MIDI files
cargo clippy --all-targets -- -D warnings
```

- Golden MIDI bytes live in `crates/hummr-core/tests/golden/`. Regenerate them
  with `UPDATE_GOLDEN=1 cargo test` after an intended change.
- Readable MIDI dumps are `insta` snapshots (`cargo insta review`).
- To print an editor screen as text, without a real terminal, run
  `cargo run -p hummr-tui --example screen -- take.wav "lll" 110 34`. The
  second argument is a sequence of keys to press first.

Workspace layout:

| Crate | Purpose |
|---|---|
| `hummr-core` | Audio engine: pitch tracking, segmentation, key, quantization, MIDI/WAV I/O. It has no UI dependencies. |
| `hummr-cli` | The `hummr` binary. |
| `hummr-tui` | Terminal editor (ratatui + tachyonfx). |

## Known limitations

- There's one tempo per take, so a singer who speeds up or slows down a lot
  over a long take will quantize less well. Use `--quantize off`, or record
  shorter takes.
- Transcription is monophonic: one note at a time, which suits a voice.
- Automatic key detection only picks major or natural minor. Pass `--key` for
  other modes.

## License

Code: MIT OR Apache-2.0. The GeneralUser GS SoundFont (to be bundled in
feature 2) is © S. Christian Collins under its own permissive license.
