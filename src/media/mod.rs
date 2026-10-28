//! Media engine: everything that touches a video file.
//!
//! The GUI depends on this module; this module never depends on the GUI.
//! All heavy work happens on background threads, and every operation is
//! cancellable or bounded.

pub mod errors;
pub mod export;
pub mod metadata;
pub mod playback;
pub mod tools;
pub mod trim;

pub use errors::AppError;
pub use export::{ExportMode, ExportOutcome, ExportPlan, ExportProgress};
pub use metadata::{AudioStream, MediaInfo, VideoStream};
pub use trim::TrimRange;
