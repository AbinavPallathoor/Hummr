# Using Hummr

This is the full guide. The [README](../README.md) has the five-minute tour
and the key tables.

- [Songs, tracks and clips](#songs-tracks-and-clips)
- [Starting](#starting)
- [Recording](#recording)
- [Importing samples](#importing-samples)
- [Editing clips](#editing-clips)
- [The inspector](#the-inspector)
- [Tempo, time signature, key and snap](#tempo-time-signature-key-and-snap)
- [The mixer](#the-mixer)
- [Saving and exporting](#saving-and-exporting)
- [The notes editor](#the-notes-editor)
- [The command line](#the-command-line)
- [The project file](#the-project-file)
- [Settings](#settings)

## Songs, tracks and clips

A **song** has one tempo, a time signature, an optional key and a snap
size, shown in the header. It has **tracks** (rows), and each track has
**clips** (boxes) at positions on the beat grid. A clip is a piece of a
recording, or the notes that a recording was turned into.

Clips never change your recordings. A clip only says which part of a
recording to play and what to do to it (effects, cleanup, conversion), so
any change can be undone or switched back off, even after saving.

When two clips on one track overlap, the later one moves to a second
**lane** under the track (`↳ lane 2`), so you can see and select both.
Both play. Move one away and the lane disappears again.

The **playhead** (`▼` in the ruler) is where playing, recording, pasting and
splitting happen. The **selected clip** is the one under the playhead on the
selected row; it has a double border.

## Starting

`hummr` with no arguments looks for `.hummr` files in the current folder
and its direct subfolders and opens the most recently changed one. If there
is none, it opens a new song, *My Song*, with a two-bar starter beat on a
*Beat* track and an empty *Voice* track to record on. Nothing is written to
disk until you save with `:w` (as `my-song/my-song.hummr`), and if you quit
without changing anything you are not asked about saving.

A new song shows a three-step guide: record something, play it, press
`Enter` on a clip. Each step ticks off when you do it; `:guide off` hides
it and `:guide` brings it back.

`hummr new "Song name"` creates `song-name/song-name.hummr` with an empty
`sources/` folder. `hummr path/to/song.hummr` opens a song.

## Recording

Select a track with `j`/`k`, put the playhead where the take should start,
and press `r`. You hear one bar of clicks, then the song plays along while
you record (use headphones). Press `r` or `space` to stop and keep the take,
`Esc` to throw it away. The take starts exactly at the playhead (snapped to
the grid). Takes recorded this way are at the song's tempo by definition,
so they are never tempo-analysed.

If there is no sound output, recording starts at once without clicks. If
the microphone sends only silence (on macOS this means the terminal has no
microphone permission), Hummr says so after a second and a half and tells
you where to switch it on. If there is no microphone, or it can't be
opened, the message says what went wrong and what to check.

## Importing samples

`i` opens the sample picker: eight built-in sounds (kick, snare, clap,
closed and open hi-hat, 808 bass, pluck, pad), then WAV files in the
project folder, its `sources/`, and the current folder. Built with
`--features decode`, mp3, flac and ogg files are listed too. `Enter` adds
the sample as a clip at the playhead on the selected track; the file is
copied into the project's `sources/` (decoded files are stored as WAV).

A sample longer than 1.5 seconds is listened to for a beat:

- If the song has no clips yet, the sample sets the song's tempo. If
  Hummr isn't sure, it says so ("Song tempo set to 118 BPM from this clip,
  but not sure (or maybe 59)").
- If the song already has clips and the sample's tempo is more than 2% off
  (and not simply half or double), Hummr asks: **Match to song**, **Change
  song to N**, or **Leave as is**. `h` and `d` halve or double the heard
  tempo first. `Esc` leaves it as is. See
  [tempo and snapping](../README.md#tempo-and-snapping-in-plain-words).

## Editing clips

| Key | |
|---|---|
| `s` | Split the clip in two at the playhead |
| `x` | Delete it |
| `y`, `p` | Copy it; paste the copy at the playhead on the selected track |
| `d` | Duplicate it right after itself |
| `H`, `L` | Move it to the previous / next grid line (one snap step) |
| `J`, `K` | Move it to the track below / above, keeping its time |
| `u`, `Ctrl-r` | Undo, redo (every edit is one step; a run of slider changes is one step) |
| `n` | New track; `:track name X` renames, `:track delete` removes the selected track |
| `m`, `S` | Mute, solo the track (when any track is soloed, only soloed tracks play) |

Splitting is exact: the two halves play back sample-for-sample like the
original.

## The inspector

`Enter` on a clip opens the inspector. With a sound device, the clip loops
on its own while the inspector is open, and every change is heard as soon
as it has been rendered (`space` turns the loop off and on). `Tab` or
`1`/`2`/`3` switch panels, `j`/`k` choose a setting, `h`/`l` change it,
`Enter` applies actions. Every line has a hint at the bottom.

**Turn into**

- *Notes on an instrument*: follows the pitch and plays it as notes. `h`/`l`
  on *Instrument* chooses piano, electric piano, vibraphone, organ, guitar,
  bass, synth bass, strings, trumpet, sax, flute, square lead, saw lead or
  pad. If the song has no key yet, the key found becomes the song's key.
- *My own sound as the instrument*: the notes are played by your recording
  itself, pitch-shifted to each note.
- *Drums*: each hit becomes a clean kick, snare or hi-hat.
- *None*: back to the recording.

**Effects**: a preset with an amount, then distortion, reverb, echo (on the
beat), filter (darker to the left, thinner to the right), pitch (semitones,
same speed), speed (same pitch), volume, pan, fade in, fade out, reverse,
and loop (how many times the clip plays back to back).

**Fix**

- *Clean noise* with a strength. Cleanup learns the background noise from
  the clip's first 300 ms unless you set *Noise sample from* and *length*
  to a moment with only background noise in it.
- *Match tempo*: `Enter` listens for the clip's beat and shows what it
  heard; `h`/`l` choose cut and snap, stretch, or keep sound; `Enter` again
  fits it to the song.
- *Snap to grid*: on a recording, `Enter` cuts it at its hits and snaps
  them; on a snapped clip, `h`/`l` set the strength.
- *Snap to key* (notes clips): moves off-key notes to the nearest note in
  the song's key.
- *Follow song tempo* (recordings): on, the clip stretches when the song's
  tempo changes; off, it keeps its sound.

## Tempo, time signature, key and snap

| | |
|---|---|
| `:bpm 120`, `:bpm half`, `:bpm double` | Set the tempo (20–400) |
| `<`, `>` | One BPM down / up |
| `T` | Tap tempo: tap at least three times; the median gap of the last eight taps sets the tempo, and a pause of two seconds starts over |
| `:time 3/4` | Time signature (a "beat" is always a quarter note, so 6/8 has three beats a bar) |
| `:key D minor`, `:key none` | Song key, used by *Snap to key* |
| `:snap 1/8`, `:snap off` | Snap size for moves, splits and the recording start |

When the tempo changes, snapped clips and notes clips keep their positions
in beats, so they speed up or slow down with the song. Recordings keep
their sound unless *Follow song tempo* is on.

## The mixer

`M` shows a strip per track under the timeline, plus the master level, and
gives the mixer the keys: `h`/`l` choose a track, `j`/`k` volume (−60 to
+12 dB), `[`/`]` pan, `m` mute, `S` solo, `0` reset. `Esc` hands the keys
back to the timeline and leaves the strips showing; `M` hides them. Meters
show the peak level of what is playing (green, then yellow, then red near
full scale) and fall back smoothly.

Changing volume, pan, mute or solo never interrupts playback.

## Saving and exporting

`:w` saves. A song is a folder: `song.hummr` (plain text, below) and
`sources/` with the recordings as WAV files. `:w other.hummr` saves a copy
somewhere else, copying the recordings along. Quitting with unsaved
changes asks once; press `q` again to quit anyway, or `:q!`.

`:export song.wav` (or `hummr export song.hummr -o song.wav`) renders the
whole song to a 48 kHz stereo WAV with a limiter that keeps peaks at
−1 dBFS. `:export song.mid` writes the notes clips as a multi-track MIDI
file, one part per track.

## The notes editor

`e` on a notes clip opens it in the piano roll from Hummr 0.1, with the
song's tempo, key and snap. `Esc` (or `:q`) goes back to the timeline and
the edited notes replace the clip's notes; `u` on the timeline undoes the
whole edit. The keys there:

| Key | |
|---|---|
| `h` `l` (or `b` `n`) | Previous / next note |
| `0` (or `^`, `g`) / `$` (or `G`) | First / last note |
| `j` `k` / `J` `K` | Pitch down / up a semitone / an octave |
| `<` `>` | Shorter / longer by one grid step |
| `H` `L` | Earlier / later by one grid step |
| `a` | Add a note at the playhead |
| `x` | Delete the note |
| `M` | Join with the next note |
| `v` / `V` | Select notes / select all; edits then apply to the selection |
| `y` / `d` / `p` | Copy / cut / paste |
| `[` `]` `{` `}` | Move the playhead a beat / a bar |
| `space` | Play / stop |
| `I` | Change the instrument |
| `:notes C4 1/4 E4 1/8 r 1/8 G4 1/2` | Type notes in at the playhead (`r` is a rest, `1/8t` a triplet) |

Only the clip's own part comes back to the timeline; extra parts added
there with `i` are dropped (with a warning). Notes clips that use your own
sound or drums play with a stand-in instrument inside the notes editor.

The 0.1 hum-to-MIDI editor is still available on its own:
`hummr edit take.wav` or `hummr edit song.mid`.

## The command line

| Command | |
|---|---|
| `hummr` | Open the song here, or start a new one |
| `hummr FILE.hummr` | Open a song on the timeline |
| `hummr new NAME` | Create `NAME/NAME.hummr` |
| `hummr export SONG.hummr [-o out.wav\|out.mid]` | Render the mix, or the notes |
| `hummr demo [--out DIR]` | Write example WAVs (default `./demo/`) |
| `hummr detect IN.wav [--hits]` | Print the tempo, beat phase, confidence and hits |
| `hummr snap IN.wav --bpm N [--grid 1/16] [--strength 1] [-o OUT]` | Snap a recording's hits to a grid |
| `hummr clean IN.wav [--strength 0.8] [--noise START:LEN] [-o OUT]` | Turn down steady hiss and hum |
| `hummr record [-o take.wav] [--seconds N]` | Record from the default microphone |
| `hummr transcribe IN.wav [--key auto] [--quantize 1/8] [--tempo auto] [-o out.mid] [--explain]` | Hummed audio to MIDI |
| `hummr edit FILE` | `.hummr` on the timeline, `.wav`/`.mid` in the 0.1 editor |

Options common to the editors: `--no-anim` (or `HUMMR_NO_ANIM=1`) turns
off animations; `--mouse` enables the mouse in the 0.1 editor.

## The project file

`song.hummr` is TOML. Times are in beats (quarter notes) from the start of
the song, except where a field says seconds. A new starter song with a
preset on its beat looks like this:

```toml
format = 1
name = "My Song"
bpm = 96.0
time_signature = "4/4"
key = "none"
snap = "1/4"
sample_rate = 48000
next_id = 5

[[sources]]
id = "s1"
file = "sources/s1-starter-beat.wav"
name = "starter beat"
recorded_bpm = 96.0

[[tracks]]
id = 2
name = "Beat"
volume_db = 0.0
pan = 0.0
mute = false
solo = false

[[tracks.clips]]
id = 3
name = "starter beat"
source = "s1"
source_start = 0.0   # seconds into the source
source_len = 5.0     # seconds
start_beat = 0.0
repeat = 2

[tracks.clips.fx]
preset = "Warm"
amount = 0.5

[[tracks]]
id = 4
name = "Voice"
volume_db = 0.0
pan = 0.0
mute = false
solo = false
clips = []
```

Clips can also have `gain_db`, `pan`, `speed`, `pitch` (semitones),
`reverse`, `fade_in`/`fade_out` (seconds), `clean` and `noise_region`
(`[start, length]` in seconds), `stretch` (keep, or follow with the BPM it
was recorded at) and a `kind`: plain audio, `sliced` (the cut points and
where each slice plays), or `notes` (the notes and what plays them). Fields
left at their defaults are not written. Files with a newer `format` than
the app understands are refused with a message rather than misread.

## Settings

| | |
|---|---|
| `HUMMR_SOUNDFONT=/path/font.sf2` | Play notes with a General MIDI SoundFont |
| `~/.config/hummr/config.toml` (or `$XDG_CONFIG_HOME/hummr/config.toml`) | `soundfont = "/path/font.sf2"` |
| `HUMMR_NO_ANIM=1` | No animations |
