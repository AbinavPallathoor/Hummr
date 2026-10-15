//! Pitch tracking.
//!
//! [`yin`] holds the per-frame YIN machinery (difference function, CMNDF,
//! trough picking). [`pyin`] builds probabilistic YIN on top: many thresholds
//! give a set of weighted pitch candidates per frame, then a Viterbi pass over
//! a pitch/voicing HMM picks the smoothest plausible path, which removes
//! octave jumps and one-frame glitches.

pub mod live;
pub mod pyin;
pub mod yin;

pub use pyin::{PitchFrame, PitchTrack, PyinParams, pyin};
