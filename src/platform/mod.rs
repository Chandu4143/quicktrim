//! OS-specific glue kept behind one small surface.
//!
//! File-manager quick actions, installers and single-instance handling grow
//! here in Phase 4; the MVP only needs a native file picker and a way to
//! reveal a finished file.

use std::path::PathBuf;

/// Extensions we offer in the file picker. This is a convenience filter, never
/// a validity check: the media probe decides what can actually be opened.
pub const PICKER_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "mpg", "mpeg", "ts", "flv", "3gp",
    "ogv",
];

/// Opens the platform's native "open file" sheet.
///
/// Returns `None` when the user cancels or when no native dialog is available
/// (for example on a minimal Linux session); the interface still offers drag
/// and drop and command-line opening.
pub fn pick_video() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open video")
        .add_filter("Video", PICKER_EXTENSIONS)
        .add_filter("All files", &["*"])
        .pick_file()
}

/// Turns a `file://` URI (as produced by drag and drop) into a local path.
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("file://") {
        // file:///C:/path on Windows, file:///home/x on Unix.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let decoded = percent_decode(rest);
        #[cfg(windows)]
        {
            let without_slash = decoded.strip_prefix('/').unwrap_or(&decoded);
            return Some(PathBuf::from(without_slash.replace('/', "\\")));
        }
        #[cfg(not(windows))]
        {
            return Some(PathBuf::from(decoded));
        }
    }

    if trimmed.contains("://") {
        // Remote URLs are deliberately not accepted: QuickTrim is offline only.
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Makes the trimmed file easy to find in Explorer/Finder/the Linux file
/// manager. Failures are cosmetic and deliberately ignored by the caller.
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    crate::output::reveal_in_file_manager(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_windows_file_uri() {
        let path = path_from_uri("file:///C:/Users/Me/Videos/My%20Clip.mp4").unwrap();
        assert_eq!(path.to_string_lossy(), "C:\\Users\\Me\\Videos\\My Clip.mp4");
    }

    #[test]
    #[cfg(not(windows))]
    fn decodes_unix_file_uri() {
        let path = path_from_uri("file:///home/me/v%C3%ADdeo%20clip.mp4").unwrap();
        assert_eq!(path.to_string_lossy(), "/home/me/vídeo clip.mp4");
    }

    #[test]
    fn rejects_network_urls() {
        assert!(path_from_uri("https://example.com/clip.mp4").is_none());
    }

    #[test]
    fn accepts_plain_paths() {
        let path = path_from_uri("C:/videos/clip.mp4").unwrap();
        assert_eq!(path.to_string_lossy(), "C:/videos/clip.mp4");
    }
}
