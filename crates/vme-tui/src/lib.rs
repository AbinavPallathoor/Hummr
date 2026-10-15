//! Terminal editor for Voice Music Editor.
//!
//! The editor (piano roll, tracker grid, mixer) arrives in feature 7; this
//! crate exists now so the workspace layout is fixed from the start and the
//! engine/UI boundary stays explicit: everything here talks to `vme-core`.

pub use vme_core;
