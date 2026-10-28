//! Output paths and the safe-replace dance.
//!
//! Rules that hold everywhere in the app:
//! * the source file is only ever read;
//! * we never write directly to the final name;
//! * we never overwrite an existing file without being told to.

use std::path::{Path, PathBuf};

use crate::media::errors::AppError;

pub const OUTPUT_EXTENSION: &str = "mp4";
const SUFFIX: &str = "_trimmed";

/// `Holiday clip.MOV` -> `Holiday clip_trimmed.mp4`, in the same folder.
pub fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "video".to_string());

    let file_name = format!("{stem}{SUFFIX}.{OUTPUT_EXTENSION}");
    match input.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

/// Returns `base` if it is free, otherwise `name (2).mp4`, `name (3).mp4`, …
pub fn unique_path(base: &Path) -> PathBuf {
    if !base.exists() {
        return base.to_path_buf();
    }

    let stem = base
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".to_string());
    let extension = base
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| OUTPUT_EXTENSION.to_string());
    let parent = base.parent().map(Path::to_path_buf).unwrap_or_default();

    for index in 2..=9_999 {
        let candidate = parent.join(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    // Astronomically unlikely; still better than overwriting something.
    parent.join(format!("{stem}-{}.{extension}", std::process::id()))
}

/// A sibling temporary file, so the final rename is atomic and same-volume.
pub fn temp_output_path(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.mp4".to_string());
    let parent = final_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    parent.join(format!(".{name}.qt-part-{}-{nonce}", std::process::id()))
}

/// Publishes a verified temporary file. Refuses to clobber.
pub fn finalize(temp: &Path, final_path: &Path) -> Result<(), AppError> {
    let size = std::fs::metadata(temp)
        .map_err(|err| crate::media::errors::map_io_error(&err, &temp.display().to_string()))?
        .len();
    if size == 0 {
        return Err(AppError::ExportFailed {
            detail: "The media engine produced an empty file.".to_string(),
        });
    }

    if final_path.exists() {
        return Err(AppError::Io {
            detail: format!("“{}” already exists.", final_path.display()),
        });
    }

    std::fs::rename(temp, final_path)
        .map_err(|err| crate::media::errors::map_io_error(&err, &final_path.display().to_string()))
}

/// Removes a temporary artefact, ignoring "not there" and transient sharing
/// violations (Windows can hold a file for a moment after a killed process).
pub fn remove_partial(path: &Path) {
    for attempt in 0..5 {
        match std::fs::remove_file(path) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if attempt < 4 => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return,
        }
    }
}

/// Opens the system file manager with the file selected.
pub fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = path.parent().unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(target)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_keeps_folder_and_stem() {
        let path = Path::new("C:/videos/Holiday clip.MOV");
        assert_eq!(
            default_output_path(path),
            PathBuf::from("C:/videos/Holiday clip_trimmed.mp4")
        );
    }

    #[test]
    fn default_name_handles_dots_and_unicode() {
        let path = Path::new("/home/u/वीडियो.final.v2.mkv");
        assert_eq!(
            default_output_path(path),
            PathBuf::from("/home/u/वीडियो.final.v2_trimmed.mp4")
        );
    }

    #[test]
    fn collision_picks_next_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("clip_trimmed.mp4");
        std::fs::write(&base, b"x").unwrap();
        let unique = unique_path(&base);
        assert_eq!(unique.file_name().unwrap(), "clip_trimmed (2).mp4");
        assert!(!unique.exists());
    }

    #[test]
    fn temp_file_is_a_hidden_sibling() {
        let final_path = Path::new("C:/videos/clip_trimmed.mp4");
        let temp = temp_output_path(final_path);
        assert_eq!(temp.parent(), final_path.parent());
        let name = temp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with(".clip_trimmed.mp4.qt-part-"));
        assert_ne!(temp, final_path);
    }

    #[test]
    fn finalize_refuses_to_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("a.part");
        let final_path = dir.path().join("a.mp4");
        std::fs::write(&temp, b"new").unwrap();
        std::fs::write(&final_path, b"old").unwrap();

        assert!(finalize(&temp, &final_path).is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"old");
        assert_eq!(std::fs::read(&temp).unwrap(), b"new");
    }

    #[test]
    fn finalize_moves_verified_output_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".a.qt-part");
        let final_path = dir.path().join("a.mp4");
        std::fs::write(&temp, b"new").unwrap();

        finalize(&temp, &final_path).unwrap();
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"new");
    }

    #[test]
    fn empty_output_is_never_published() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join(".a.qt-part");
        let final_path = dir.path().join("a.mp4");
        std::fs::write(&temp, b"").unwrap();
        assert!(finalize(&temp, &final_path).is_err());
        assert!(!final_path.exists());
    }
}
