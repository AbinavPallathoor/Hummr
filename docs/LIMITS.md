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
