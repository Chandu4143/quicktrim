//! Export: turn a validated selection into a new, playable file.
//!
//! Strategy, in order of preference:
//! 1. stream copy, but only when both cut points land on keyframes and the
//!    codecs are legal in MP4 (this is near-instant and lossless);
//! 2. H.264/AAC re-encode with sensible defaults (always accurate).
//!
//! A copy that produces a clip of the wrong length is discarded and retried as
//! an encode, so the user never receives a subtly wrong file.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use super::errors::{map_ffmpeg_failure, AppError};
use super::metadata::{self, MediaInfo};
use super::tools;
use super::trim::TrimRange;

/// How close to a keyframe a cut point must be to allow stream copy.
const KEYFRAME_TOLERANCE_SECS: f64 = 0.045;
/// A copy whose duration lands further than this from the request is rejected.
/// 200 ms leaves room for AAC priming and the container's own rounding while
/// still catching a copy that grabbed the wrong span.
const COPY_DURATION_TOLERANCE_US: i64 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    StreamCopy,
    ReEncode,
}

impl ExportMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::StreamCopy => "saving without re-encoding",
            Self::ReEncode => "re-encoding for an accurate cut",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportPlan {
    pub input: PathBuf,
    pub output: PathBuf,
    pub range: TrimRange,
    pub mode: ExportMode,
    pub expected_duration_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportProgress {
    /// 0.0 … 1.0, monotonic.
    pub fraction: f32,
    pub processed_us: i64,
}

#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub output: PathBuf,
    pub mode_used: ExportMode,
}

/// Builds the plan without touching the filesystem beyond cheap checks.
pub fn plan(info: &MediaInfo, range: TrimRange) -> Result<ExportPlan, AppError> {
    plan_to(info, range, None)
}

pub fn plan_to(
    info: &MediaInfo,
    range: TrimRange,
    requested_output: Option<PathBuf>,
) -> Result<ExportPlan, AppError> {
    let output = requested_output.unwrap_or_else(|| crate::output::default_output_path(&info.path));
    let output = ensure_mp4(output);

    let parent = output.parent().unwrap_or(Path::new("."));
    if !parent.exists() {
        return Err(AppError::Io {
            detail: format!("The folder “{}” does not exist.", parent.display()),
        });
    }
    if output == info.path {
        return Err(AppError::Io {
            detail: "QuickTrim will not overwrite the video it is trimming.".to_string(),
        });
    }

    let mode = if copy_is_safe(info) && cuts_on_keyframes(info, range) {
        ExportMode::StreamCopy
    } else {
        ExportMode::ReEncode
    };

    Ok(ExportPlan {
        input: info.path.clone(),
        output,
        range,
        mode,
        expected_duration_us: range.duration_us(),
    })
}

fn ensure_mp4(mut path: PathBuf) -> PathBuf {
    let is_mp4 = path
        .extension()
        .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("mp4"))
        .unwrap_or(false);
    if !is_mp4 {
        path.set_extension(crate::output::OUTPUT_EXTENSION);
    }
    path
}

fn copy_is_safe(info: &MediaInfo) -> bool {
    // Only codecs that mainstream desktop players reliably open from MP4.
    // VP9 in MP4 is legal but poorly supported by Windows/macOS players, so it
    // is re-encoded rather than copied.
    let video_ok = matches!(
        info.video.codec.as_str(),
        "h264" | "hevc" | "av1" | "mpeg4"
    );
    let audio_ok = match &info.audio {
        None => true,
        Some(audio) => matches!(audio.codec.as_str(), "aac" | "mp3" | "ac3" | "eac3" | "alac"),
    };
    video_ok && audio_ok
}

fn cuts_on_keyframes(info: &MediaInfo, range: TrimRange) -> bool {
    let start = range.start_secs();
    let end = range.end_secs();

    let start_ok = start <= 0.01 || keyframe_near(&info.path, start).unwrap_or(false);
    if !start_ok {
        return false;
    }

    end >= info.duration_secs() - 0.05
        || keyframe_near(&info.path, end).unwrap_or(false)
}

/// Asks ffprobe for keyframes in a one-second window around `time`.
/// A failure or an unparseable answer simply disables stream copy.
fn keyframe_near(path: &Path, time: f64) -> Result<bool, AppError> {
    let ffprobe = tools::ffprobe()?;
    let from = (time - 0.6).max(0.0);
    let to = time + 0.6;

    let output = tools::command(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-skip_frame",
            "nokey",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
            "-read_intervals",
            &format!("{from:.3}%{to:.3}"),
        ])
        .arg(path)
        .output()
        .map_err(|_| AppError::ToolMissing {
            tool: "ffprobe".to_string(),
        })?;

    if !output.status.success() {
        return Ok(false);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter_map(|line| line.trim().parse::<f64>().ok())
        .any(|pts| (pts - time).abs() <= KEYFRAME_TOLERANCE_SECS))
}

/// Runs the export and returns only after the output is verified and in place.
///
/// `on_progress` is called from this thread; the caller decides how to marshal
/// that onto the UI thread.
pub fn run(
    plan: &ExportPlan,
    cancel: Arc<AtomicBool>,
    mut on_progress: impl FnMut(ExportProgress),
) -> Result<ExportOutcome, AppError> {
    if plan.output.exists() {
        return Err(AppError::Io {
            detail: format!("“{}” already exists.", plan.output.display()),
        });
    }

    let temp = crate::output::temp_output_path(&plan.output);
    crate::output::remove_partial(&temp);

    // A stream copy is attempted first only when the plan says it is safe;
    // if the result cannot be trusted, the same clip is re-encoded.
    let mut mode = plan.mode;
    let mut retry_reason: Option<AppError> = None;

    loop {
        if cancel.load(Ordering::Relaxed) {
            crate::output::remove_partial(&temp);
            return Err(AppError::Cancelled);
        }

        let result = attempt(plan, &temp, mode, &cancel, &mut on_progress)
            .and_then(|()| validate_for(plan, &temp, mode));

        match result {
            Ok(()) => break,
            Err(err) if should_retry_with_encode(mode, &err) => {
                diag::warn(&format!(
                    "stream copy not usable ({err:?}); retrying with re-encode"
                ));
                crate::output::remove_partial(&temp);
                mode = ExportMode::ReEncode;
                retry_reason = Some(err);
                continue;
            }
            Err(err) => {
                crate::output::remove_partial(&temp);
                return Err(err);
            }
        }
    }

    if cancel.load(Ordering::Relaxed) {
        crate::output::remove_partial(&temp);
        return Err(AppError::Cancelled);
    }

    if let Err(err) = crate::output::finalize(&temp, &plan.output) {
        crate::output::remove_partial(&temp);
        return Err(err);
    }

    if let Some(reason) = retry_reason {
        diag::warn(&format!("export finished after retry; first failure: {reason:?}"));
    }

    on_progress(ExportProgress {
        fraction: 1.0,
        processed_us: plan.expected_duration_us,
    });

    Ok(ExportOutcome {
        output: plan.output.clone(),
        mode_used: mode,
    })
}

fn validate_for(plan: &ExportPlan, temp: &Path, mode: ExportMode) -> Result<(), AppError> {
    let tolerance = match mode {
        ExportMode::StreamCopy => COPY_DURATION_TOLERANCE_US,
        ExportMode::ReEncode => validation_tolerance_us(plan.expected_duration_us),
    };
    validate_output(temp, plan.expected_duration_us, tolerance)
}

fn should_retry_with_encode(mode: ExportMode, err: &AppError) -> bool {
    if mode != ExportMode::StreamCopy {
        return false;
    }
    !matches!(
        err,
        AppError::Cancelled
            | AppError::DiskFull
            | AppError::PermissionDenied { .. }
            | AppError::InputUnavailable { .. }
            | AppError::ToolMissing { .. }
    )
}

fn validation_tolerance_us(expected_us: i64) -> i64 {
    // Containers round durations to their own timescale. Allow 250 ms or 5% of
    // the selection, whichever is larger, but never more than the clip itself.
    let base = 250_000.max(expected_us / 20);
    base.min(expected_us)
}

fn validate_output(path: &Path, expected_us: i64, tolerance_us: i64) -> Result<(), AppError> {
    let info = metadata::probe(path).map_err(|err| AppError::ExportFailed {
        detail: err.message(),
    })?;
    let drift = (info.duration_us - expected_us).abs();
    if drift > tolerance_us {
        return Err(AppError::ExportFailed {
            detail: format!(
                "The result is {:.2} s long but the selection was {:.2} s.",
                info.duration_us as f64 / 1e6,
                expected_us as f64 / 1e6
            ),
        });
    }
    Ok(())
}

/// One FFmpeg invocation, writing to `temp`.
fn attempt(
    plan: &ExportPlan,
    temp: &Path,
    mode: ExportMode,
    cancel: &AtomicBool,
    on_progress: &mut impl FnMut(ExportProgress),
) -> Result<(), AppError> {
    let ffmpeg = tools::ffmpeg()?;
    let args = build_args(&plan.input, temp, plan.range, mode);

    let mut child = tools::command(&ffmpeg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::ToolMissing {
                tool: "ffmpeg".to_string(),
            },
            _ => AppError::Internal {
                detail: format!("Could not start the media engine: {err}."),
            },
        })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Drain stderr so a chatty failure cannot block the process.
    let stderr_handle = stderr.map(|stderr| {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut tail = String::new();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                tail.push_str(&line);
                if tail.len() > 8_192 {
                    // Keep only the most recent chunk.
                    tail.drain(..tail.len() - 4_096);
                }
                line.clear();
            }
            tail
        })
    });

    let (line_tx, line_rx) = mpsc::channel::<String>();
    let stdout_handle = stdout.map(|stdout| {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        })
    });

    let mut last_fraction = 0.0f32;
    let mut cancelled = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            let _ = child.kill();
            break;
        }

        match line_rx.recv_timeout(Duration::from_millis(120)) {
            Ok(line) => {
                if let Some(processed_us) = parse_progress_us(&line) {
                    let fraction = if plan.expected_duration_us > 0 {
                        (processed_us as f32 / plan.expected_duration_us as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    if fraction >= last_fraction {
                        last_fraction = fraction;
                        on_progress(ExportProgress {
                            fraction,
                            processed_us,
                        });
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_)) = child.try_wait() {
                    // Process ended; drain what is left and stop waiting.
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if cancelled {
        let _ = child.wait();
        if let Some(handle) = stdout_handle {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }
        crate::output::remove_partial(temp);
        return Err(AppError::Cancelled);
    }

    let status = child.wait().map_err(|err| AppError::Internal {
        detail: format!("Lost track of the media engine: {err}."),
    })?;

    if let Some(handle) = stdout_handle {
        let _ = handle.join();
    }
    let stderr_tail = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        diag::warn(&format!("ffmpeg exited with {status}: {stderr_tail}"));
        return Err(map_ffmpeg_failure(&stderr_tail));
    }

    Ok(())
}

/// FFmpeg's `-progress` stream uses `out_time_us=<microseconds>`.
fn parse_progress_us(line: &str) -> Option<i64> {
    let (key, value) = line.trim().split_once('=')?;
    if key != "out_time_us" && key != "out_time_ms" {
        return None;
    }
    value.trim().parse::<i64>().ok().filter(|us| *us >= 0)
}

/// Arguments are separate strings; nothing is ever shell-interpolated.
pub fn build_args(input: &Path, output: &Path, range: TrimRange, mode: ExportMode) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-y".into(),
        "-ss".into(),
        format!("{:.6}", range.start_secs()),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-t".into(),
        format!("{:.6}", range.duration_secs()),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0?".into(),
    ];

    match mode {
        ExportMode::StreamCopy => {
            args.extend(["-c", "copy"].map(String::from));
        }
        ExportMode::ReEncode => {
            args.extend(
                [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    "20",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                ]
                .map(String::from),
            );
        }
    }

    args.extend(
        [
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ]
        .map(String::from),
    );
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Minimal diagnostic logging. Writes only to local stderr; never to the
/// network, never into the interface.
pub(crate) mod diag {
    pub fn warn(message: &str) {
        eprintln!("[quicktrim] {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::metadata::{AudioStream, VideoStream};
    use std::path::Path;

    fn info(codec: &str, audio: Option<&str>) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("C:/videos/sample.mp4"),
            duration_us: 30_000_000,
            format_name: "mov,mp4".into(),
            video: VideoStream {
                codec: codec.into(),
                width: 1920,
                height: 1080,
                fps: Some(30.0),
                rotation_deg: 0,
            },
            audio: audio.map(|codec| AudioStream {
                codec: codec.into(),
                channels: 2,
                sample_rate: 48_000,
            }),
        }
    }

    fn range() -> TrimRange {
        TrimRange::new(1_000_000, 5_000_000, 30_000_000).unwrap()
    }

    #[test]
    fn encode_arguments_ask_for_h264_and_aac() {
        let args = build_args(
            Path::new("C:/videos/in.mp4"),
            Path::new("C:/videos/out.mp4"),
            range(),
            ExportMode::ReEncode,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-c:a aac"));
        assert!(joined.contains("-ss 1.000000"));
        assert!(joined.contains("-t 4.000000"));
        assert!(!joined.contains("copy"));
    }

    #[test]
    fn copy_arguments_never_re_encode() {
        let args = build_args(
            Path::new("/videos/in.mkv"),
            Path::new("/videos/out.mp4"),
            range(),
            ExportMode::StreamCopy,
        );
        let joined = args.join(" ");
        assert!(joined.contains("-c copy"));
        assert!(!joined.contains("libx264"));
        assert!(!joined.contains("aac"));
    }

    #[test]
    fn paths_are_passed_as_single_arguments() {
        let args = build_args(
            Path::new("C:/videos/My \"quoted\" 视频.mp4"),
            Path::new("C:/videos/out put.mp4"),
            range(),
            ExportMode::ReEncode,
        );
        assert!(args.contains(&"C:/videos/My \"quoted\" 视频.mp4".to_string()));
        assert!(args.contains(&"C:/videos/out put.mp4".to_string()));
    }

    #[test]
    fn progress_parsing_reads_microseconds() {
        assert_eq!(parse_progress_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_progress_us("out_time_ms=1500000"), Some(1_500_000));
        assert_eq!(parse_progress_us("progress=continue"), None);
        assert_eq!(parse_progress_us("out_time_us=N/A"), None);
    }

    #[test]
    fn plan_refuses_to_target_the_source_file() {
        let media = info("h264", Some("aac"));
        let err = plan_to(&media, range(), Some(media.path.clone())).unwrap_err();
        assert!(matches!(err, AppError::Io { .. }));
    }

    #[test]
    fn plan_normalises_output_extension_to_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let media = info("h264", Some("aac"));
        let plan = plan_to(&media, range(), Some(dir.path().join("out.mkv"))).unwrap();
        assert_eq!(plan.output.extension().unwrap(), "mp4");
    }

    #[test]
    fn incompatible_codecs_force_re_encode() {
        let dir = tempfile::tempdir().unwrap();
        let media = info("prores", Some("pcm_s16le"));
        let plan = plan_to(&media, range(), Some(dir.path().join("out.mp4"))).unwrap();
        assert_eq!(plan.mode, ExportMode::ReEncode);
    }

    #[test]
    fn retry_only_happens_for_copy_mode_and_recoverable_errors() {
        assert!(should_retry_with_encode(
            ExportMode::StreamCopy,
            &AppError::ExportFailed {
                detail: "short".into()
            }
        ));
        assert!(!should_retry_with_encode(
            ExportMode::StreamCopy,
            &AppError::Cancelled
        ));
        assert!(!should_retry_with_encode(
            ExportMode::ReEncode,
            &AppError::ExportFailed {
                detail: "short".into()
            }
        ));
    }
}
