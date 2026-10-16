//! Audio engine for Hummr (`hummr`).
//!
//! This crate has no UI dependencies. The pipeline for turning a hummed take
//! into notes is:
//!
//! 1. [`pitch::pyin`]: probabilistic YIN pitch tracking with HMM smoothing.
//! 2. [`segment`]: a note-level HMM plus RMS energy dips splits the pitch
//!    track into notes.
//! 3. [`theory`]: key detection and snapping to the nearest in-key note.
//! 4. [`rhythm`]: tempo detection and quantisation to a grid.
//!
//! [`transcribe::transcribe`] runs all of the above and records every change
//! it makes so the CLI can explain itself.

pub mod audio;
pub mod beat;
pub mod buffer;
pub mod denoise;
pub mod dsp;
pub mod error;
pub mod fuzzy;
pub mod fx;
pub mod instruments;
pub mod io;
pub mod kit;
pub mod notation;
pub mod onset;
pub mod pitch;
pub mod preview;
pub mod project;
pub mod resample;
pub mod rhythm;
pub mod segment;
pub mod sequence;
pub mod slice;
pub mod stretch;
pub mod synthetic;
pub mod theory;
pub mod transcribe;
pub mod voice;

pub use buffer::AudioBuffer;
pub use error::{Error, Result};
