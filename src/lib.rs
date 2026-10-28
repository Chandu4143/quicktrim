//! QuickTrim: open a video, choose a range, save the result.
//!
//! The crate is split so the media engine can be tested without a display:
//!
//! * [`media`] — probing, playback, export, trimming rules.
//! * [`output`] — safe output naming and atomic publication.
//! * [`platform`] — the small amount of OS-specific glue.
//! * [`app`] — the Slint-facing application shell.

slint::include_modules!();

pub mod app;
pub mod media;
pub mod output;
pub mod platform;
