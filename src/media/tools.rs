//! Locates and launches the bundled media helpers.
//!
//! QuickTrim never invokes a shell: paths are passed as separate process
//! arguments, so spaces, quotes and non-ASCII characters are inert.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::errors::AppError;

pub const FFMPEG_ENV: &str = "QUICKTRIM_FFMPEG";
pub const FFPROBE_ENV: &str = "QUICKTRIM_FFPROBE";

/// Resolves a helper binary, preferring the copy shipped next to the app.
/// Search order: explicit environment override, application directory, PATH.
pub fn resolve_tool(env_var: &str, binary_name: &str) -> Result<PathBuf, AppError> {
    if let Some(path) = std::env::var_os(env_var) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        for candidate in bundled_names(binary_name) {
            let path = dir.join(candidate);
            if path.is_file() {
                return Ok(path);
            }
        }
    }

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        for candidate in bundled_names(binary_name) {
            let path = dir.join(candidate);
            if path.is_file() {
                return Ok(path);
            }
        }
    }

    Err(AppError::ToolMissing {
        tool: binary_name.to_string(),
    })
}

fn bundled_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        vec![format!("{name}.exe"), name.to_string()]
    }
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

pub fn ffmpeg() -> Result<PathBuf, AppError> {
    resolve_tool(FFMPEG_ENV, "ffmpeg")
}

pub fn ffprobe() -> Result<PathBuf, AppError> {
    resolve_tool(FFPROBE_ENV, "ffprobe")
}

/// A `Command` that never owns a console window and never reads stdin.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tool_produces_an_actionable_error() {
        let err = resolve_tool("QUICKTRIM_DEFINITELY_UNSET_VAR", "no-such-tool-xyz").unwrap_err();
        assert!(matches!(err, AppError::ToolMissing { .. }));
    }
}
