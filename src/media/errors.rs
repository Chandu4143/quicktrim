//! User-facing error model.
//!
//! Media backends produce noisy, technical failures. Everything that crosses
//! into the interface is mapped to one of these variants, each of which knows
//! how to explain itself to a person who does not care about codecs.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// A required helper binary (ffmpeg/ffprobe/libmpv) could not be found.
    ToolMissing { tool: String },
    /// The input file does not exist, is a directory, or cannot be read.
    InputUnavailable { detail: String },
    /// The file is not media we can work with.
    Unsupported { detail: String },
    /// The file looks like media but is damaged or truncated.
    Corrupt { detail: String },
    /// The requested start/end points do not describe a usable clip.
    InvalidSelection { detail: String },
    /// The destination volume is out of space.
    DiskFull,
    /// We were not allowed to read or write in a location.
    PermissionDenied { detail: String },
    /// Processing failed for a reason that is not more specific.
    ExportFailed { detail: String },
    /// The user cancelled, deliberately.
    Cancelled,
    /// Filesystem trouble outside the categories above.
    Io { detail: String },
    /// A bug or an unexpected backend state.
    Internal { detail: String },
}

impl AppError {
    /// A short, actionable sentence shown in the interface.
    pub fn message(&self) -> String {
        match self {
            Self::ToolMissing { tool } => format!(
                "QuickTrim is missing the bundled media tool “{tool}”. Reinstall QuickTrim to restore it."
            ),
            Self::InputUnavailable { detail } => format!(
                "That video could not be opened. {detail} Check that the file still exists and that you have permission to read it."
            ),
            Self::Unsupported { detail } => format!(
                "This file does not contain a video QuickTrim can play. {detail}"
            ),
            Self::Corrupt { detail } => format!(
                "This video appears to be damaged or incomplete. {detail} Try another copy of the file."
            ),
            Self::InvalidSelection { detail } => {
                format!("The selected range is not valid. {detail}")
            }
            Self::DiskFull => "There is not enough free space to save the trimmed video. Free some space and try again.".to_string(),
            Self::PermissionDenied { detail } => format!(
                "QuickTrim is not allowed to write there. {detail} Choose a different destination."
            ),
            Self::ExportFailed { detail } => {
                format!("The trimmed video could not be created. {detail}")
            }
            Self::Cancelled => "Export cancelled. The original video was not changed.".to_string(),
            Self::Io { detail } => format!("A file operation failed. {detail}"),
            Self::Internal { detail } => format!(
                "Something unexpected went wrong. {detail} The original video was not changed."
            ),
        }
    }

    /// Technical detail for the diagnostic log only.
    pub fn diagnostic(&self) -> String {
        format!("{self:?}")
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        map_io_error(&err, "unknown path")
    }
}

/// Maps raw I/O failures onto messages a person can act on.
pub fn map_io_error(err: &std::io::Error, path: &str) -> AppError {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => AppError::InputUnavailable {
            detail: format!("“{path}” was not found."),
        },
        ErrorKind::PermissionDenied => AppError::PermissionDenied {
            detail: format!("“{path}” is not writable."),
        },
        ErrorKind::AlreadyExists => AppError::Io {
            detail: format!("“{path}” already exists."),
        },
        _ => {
            let text = err.to_string();
            if text.contains("space") || err.raw_os_error() == Some(28)
                || err.raw_os_error() == Some(112)
            {
                AppError::DiskFull
            } else {
                AppError::Io {
                    detail: format!("“{path}”: {text}."),
                }
            }
        }
    }
}

/// Interprets the tail of an FFmpeg error log. FFmpeg messages are stable
/// enough for this to be useful, and we never show the log itself.
pub fn map_ffmpeg_failure(stderr_tail: &str) -> AppError {
    let text = stderr_tail.to_ascii_lowercase();

    if text.contains("no space left") || text.contains("disk full") {
        return AppError::DiskFull;
    }
    if text.contains("permission denied") || text.contains("access is denied") {
        return AppError::PermissionDenied {
            detail: "The destination folder rejected the write.".to_string(),
        };
    }
    if text.contains("invalid data found")
        || text.contains("moov atom not found")
        || text.contains("corrupt")
        || text.contains("truncat")
        || text.contains("partial file")
    {
        return AppError::Corrupt {
            detail: "The file ended unexpectedly while it was being read.".to_string(),
        };
    }
    if text.contains("does not contain any stream")
        || text.contains("could not find codec parameters")
        || text.contains("unknown decoder")
        || text.contains("decoder (codec") && text.contains("not found")
    {
        return AppError::Unsupported {
            detail: "Its video track could not be decoded on this build.".to_string(),
        };
    }
    if text.contains("no such file or directory") {
        return AppError::InputUnavailable {
            detail: "The file disappeared while it was being read.".to_string(),
        };
    }

    AppError::ExportFailed {
        detail: "The media engine reported an error.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_full_is_recognised_from_ffmpeg_output() {
        assert_eq!(
            map_ffmpeg_failure("av_interleaved_write_frame(): No space left on device"),
            AppError::DiskFull
        );
    }

    #[test]
    fn truncated_file_maps_to_corrupt() {
        assert!(matches!(
            map_ffmpeg_failure("moov atom not found\nInvalid data found when processing input"),
            AppError::Corrupt { .. }
        ));
    }

    #[test]
    fn missing_decoder_maps_to_unsupported() {
        assert!(matches!(
            map_ffmpeg_failure("Unknown decoder 'prores_ks'"),
            AppError::Unsupported { .. }
        ));
    }

    #[test]
    fn user_messages_never_leak_the_internal_variant_name() {
        let err = AppError::ExportFailed {
            detail: "The media engine reported an error.".into(),
        };
        assert!(!err.message().contains("ExportFailed"));
        assert!(err.message().contains("could not be created"));
    }
}
