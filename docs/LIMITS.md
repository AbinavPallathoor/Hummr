# Known limits

Hummr is built and tested without anyone listening: every claim below comes
from a measurement on synthetic signals, not from ears. Listen to the files
from `hummr demo` and judge for yourself.

## Time-stretch and pitch-shift (WSOLA)

How it works: the sound is rebuilt from overlapping 42 ms pieces, each
nudged by up to 12 ms to the spot where its waveform best continues the
last piece.

Measured (tests in `crates/hummr-core/src/stretch.rs`):

- Length is exact (to the sample) for ratios 0.25–4.
- Pitch of a steady sung tone is preserved within 0.6 cents for stretch
  ratios 0.5–2 (requirement: 20 cents).
- Pitch-shifts of −12 to +12 semitones land within 0.5 cents of the target
  (requirement: 15 cents).
- Level stays within 1.5 dB.
- Speed: 90–240× real time on one core (2× stretch is the slowest).

What you will probably hear:

- **Drums and other sharp hits get blurred or doubled when stretched.**
  On a synthetic drum loop every hit survives, but hits move by up to
  10 ms (0.75×) to 24 ms (2×) from where they belong, and a stretched
  attack can sound like a soft flam. That is why rhythmic clips are *cut
  and snapped* (each hit keeps its own sound and just moves) rather than
  stretched when you match them to the song's tempo.
- **Big pitch shifts sound like a different-sized person.** Shifting moves
  the whole spectrum, including the resonances of your mouth and throat
  ("formants"), so +12 sounds like a chipmunk and −12 like a giant. There
  is no formant correction.
- **Chords and full mixes can sound "phasey"** (a slightly hollow,
  swimming quality) because the waveform matching follows one dominant
  pitch. Single voices, hums and single instruments work best.
- Very slow stretches (beyond 2×) start to sound grainy and buzzy.

## Tempo detection

- Tested on synthetic kick/snare/hat loops with ±15 ms timing wobble:
  correct within 1% for every loop from 80 to 160 BPM; slower loops (60–75
  BPM) may be reported at double speed and faster ones at half speed, with
  the right answer offered as the first alternative.
- Humming, singing and pads have no clear hits, so Hummr says "no steady
  beat found" rather than guessing; record those with the count-in click
  instead (then nothing needs detecting).
- Real recordings with reverb, background music or very soft hits have not
  been measured.

## Noise cleanup

- Works on **steady** background noise only: hiss, fan or air-conditioning
  rush, mains hum. It learns the noise from a quiet moment and turns down
  whatever matches it. Voices, music, traffic, clicks and other sounds that
  come and go are louder than the learned noise, so they are kept.
- Measured on white hiss under a 440 Hz tone at strength 0.8: the noise
  between notes drops about 18 dB, the noise under the tone about 15 dB,
  and the tone's level changes by less than 0.01 dB.
- Unless you mark a noise-only region, the noise is learned from the
  clip's first 300 ms. If you were already singing then, the learned
  "noise" includes some of the voice and the cleanup will thin the sound
  out. Mark a quiet region by hand in that case.
- High strengths can leave faint warbling ("musical noise") in what remains
  of the hiss; lower the strength if you hear it.

## Turn into notes / own sound / drums

- **Notes** uses the same hum-to-notes transcriber as `hummr transcribe`
  (one note at a time; chords and fast runs are not supported).
- **Own sound** plays your recording at other pitches by pitch-shifting
  it, so the limits of pitch-shifting apply: past a few semitones the
  voice sounds smaller or bigger. Notes more than an octave away from the
  recorded note are played an octave closer. Notes longer than the sound
  stretch it (up to 4×); very long notes may sound grainy.
- **Drums** sorts each hit into kick, snare or hat by where its energy
  sits (low thump, mid noise, high hiss). Measured on synthetic hits only;
  hits that sound like two drums at once become one drum, and toms,
  cymbals and claps become the nearest of the three.

## Playback, recording and the timeline

- **Playback has no limiter.** While you play the song in the app, tracks
  are summed and a gentle soft clip catches peaks; the −1 dBFS look-ahead
  limiter only runs when you export. A song that is too loud can sound a
  little more distorted in the app than in the exported WAV. Turn tracks
  down in the mixer if the master meter sits in the red.
- **No latency compensation.** The time between the speaker playing a
  click and the microphone hearing you is not measured or subtracted, so
  a take can land a few milliseconds late (typically 5–30 ms, depending on
  the sound device). With snap on, `H`/`L` or *Snap to grid* fix it.
- **Headphones.** Recording plays the song back while you sing. Through
  speakers the microphone records the song too, and nothing removes it.
- **One tempo per song.** There are no tempo changes within a song.
- **Overlapping clips both play.** A clip placed over another goes to a
  second lane and is mixed with it; it does not replace it. Split and
  delete the part you don't want.
- **Preview loop.** The inspector's loop renders the clip in the
  background after each change; on long clips with heavy settings
  (own-sound conversion, large stretches) you may hear the old version for
  a moment.
- **SoundFonts are not bundled.** Free General MIDI fonts are either tens
  of megabytes or GPL-licensed, so none ships with Hummr. Without one,
  notes play on a small built-in synth that sounds like a basic keyboard.
  The 0.1 notes editor always uses the built-in synth.
- **Untested without hardware.** The tests cover everything except real
  sound devices: speaker output and microphone input are only exercised
  through the timing logic around them (count-in, take trimming, the
  render thread), not with actual audio hardware.
- **Import formats.** WAV always; mp3, flac and ogg only when built with
  `--features decode`. Only FLAC decoding is covered by a test.
