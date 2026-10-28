//! Media metadata via FFprobe.
//!
//! Only what the interface and the trim planner actually need is extracted.
//! Parsing is deliberately tolerant: containers in the wild omit or mistype
//! fields, and a missing field should degrade, not crash.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;

use super::errors::AppError;
use super::tools;

#[derive(Debug, Clone, PartialEq)]
pub struct VideoStream {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// Average frame rate when the container reports one.
    pub fps: Option<f64>,
    /// Rotation metadata in degrees (clockwise), normalised to a multiple of 90.
    pub rotation_deg: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStream {
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub duration_us: i64,
    pub format_name: String,
    pub video: VideoStream,
    pub audio: Option<AudioStream>,
}

impl MediaInfo {
    /// Dimensions as the viewer will actually see them (rotation applied).
    pub fn display_size(&self) -> (u32, u32) {
        match self.video.rotation_deg.rem_euclid(180) {
            90 => (self.video.height, self.video.width),
            _ => (self.video.width, self.video.height),
        }
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_us as f64 / 1_000_000.0
    }

    /// Frame duration in microseconds, when a frame rate is known.
    pub fn frame_us(&self) -> Option<i64> {
        self.video
            .fps
            .filter(|fps| *fps > 0.0 && fps.is_finite())
            .map(|fps| (1_000_000.0 / fps).round() as i64)
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }
}

pub fn probe(path: &Path) -> Result<MediaInfo, AppError> {
    let ffprobe = tools::ffprobe()?;
    probe_with(&ffprobe, path)
}

pub fn probe_with(ffprobe: &Path, path: &Path) -> Result<MediaInfo, AppError> {
    if !path.exists() {
        return Err(AppError::InputUnavailable {
            detail: format!("“{}” was not found.", path.display()),
        });
    }
    if path.is_dir() {
        return Err(AppError::InputUnavailable {
            detail: "That is a folder, not a video file.".to_string(),
        });
    }

    let output = tools::command(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_entries",
            "format=duration,format_name:stream=index,codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,duration,channels,sample_rate:stream_side_data=rotation:stream_disposition=attached_pic:stream_tags=rotate",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => AppError::ToolMissing {
                tool: "ffprobe".to_string(),
            },
            _ => AppError::Internal {
                detail: format!("Could not run the media probe: {err}."),
            },
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(super::errors::map_ffmpeg_failure(&stderr));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    parse_ffprobe_json(path, &json)
}

/// Split from the process call so it can be tested without FFprobe.
pub fn parse_ffprobe_json(path: &Path, json: &str) -> Result<MediaInfo, AppError> {
    let root: Value = serde_json::from_str(json).map_err(|_| AppError::Corrupt {
        detail: "The file has no readable media information.".to_string(),
    })?;

    let streams = root
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Unsupported {
            detail: "The file has no media streams.".to_string(),
        })?;

    let video = streams
        .iter()
        .find(|stream| {
            stream.get("codec_type").and_then(Value::as_str) == Some("video")
                && !is_attached_picture(stream)
        })
        .ok_or_else(|| AppError::Unsupported {
            detail: "The file has no video track.".to_string(),
        })?;

    let video = VideoStream {
        codec: text(video, "codec_name").unwrap_or_else(|| "unknown".to_string()),
        width: number(video, "width").unwrap_or(0.0).max(0.0) as u32,
        height: number(video, "height").unwrap_or(0.0).max(0.0) as u32,
        fps: rational(video, "avg_frame_rate").or_else(|| rational(video, "r_frame_rate")),
        rotation_deg: rotation_of(video),
    };

    if video.width == 0 || video.height == 0 {
        return Err(AppError::Corrupt {
            detail: "The video track has no usable dimensions.".to_string(),
        });
    }

    let audio = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .map(|stream| AudioStream {
            codec: text(stream, "codec_name").unwrap_or_else(|| "unknown".to_string()),
            channels: number(stream, "channels").unwrap_or(0.0).max(0.0) as u32,
            sample_rate: number(stream, "sample_rate").unwrap_or(0.0).max(0.0) as u32,
        });

    let duration_us = root
        .get("format")
        .and_then(|format| number(format, "duration"))
        .or_else(|| {
            streams
                .iter()
                .filter_map(|stream| number(stream, "duration"))
                .fold(None, |acc: Option<f64>, value| {
                    Some(acc.map_or(value, |a| a.max(value)))
                })
        })
        .map(secs_to_us)
        .filter(|us| *us > 0)
        .ok_or_else(|| AppError::Unsupported {
            detail: "Its duration is unknown, so it cannot be trimmed.".to_string(),
        })?;

    Ok(MediaInfo {
        path: path.to_path_buf(),
        duration_us,
        format_name: root
            .get("format")
            .and_then(|format| text(format, "format_name"))
            .unwrap_or_default(),
        video,
        audio,
    })
}

fn is_attached_picture(stream: &Value) -> bool {
    stream
        .get("disposition")
        .and_then(|disposition| disposition.get("attached_pic"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        == 1
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn rational(value: &Value, key: &str) -> Option<f64> {
    let raw = text(value, key)?;
    let (num, den) = raw.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 || num <= 0.0 {
        return None;
    }
    let fps = num / den;
    (fps.is_finite() && fps > 0.1 && fps < 1000.0).then_some(fps)
}

fn rotation_of(stream: &Value) -> i32 {
    let raw = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter().find_map(|entry| {
                entry
                    .get("rotation")
                    .and_then(Value::as_f64)
                    .or_else(|| entry.get("rotation").and_then(Value::as_str)?.parse().ok())
            })
        })
        .or_else(|| {
            stream
                .get("tags")
                .and_then(|tags| tags.get("rotate"))
                .and_then(|rotate| rotate.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0);

    let normalized = (raw.round() as i32).rem_euclid(360);
    let quarter = (normalized as f64 / 90.0).round() as i32;
    (quarter * 90).rem_euclid(360)
}

fn secs_to_us(secs: f64) -> i64 {
    if !secs.is_finite() || secs <= 0.0 {
        return 0;
    }
    (secs * 1_000_000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "format": { "duration": "12.500000", "format_name": "mov,mp4,m4a,3gp,3g2,mj2" },
      "streams": [
        {
          "index": 0,
          "codec_type": "video",
          "codec_name": "h264",
          "width": 1920,
          "height": 1080,
          "avg_frame_rate": "30000/1001",
          "disposition": { "attached_pic": 0 },
          "side_data_list": [ { "rotation": -90 } ]
        },
        {
          "index": 1,
          "codec_type": "audio",
          "codec_name": "aac",
          "channels": 2,
          "sample_rate": "48000"
        }
      ]
    }"#;

    #[test]
    fn parses_typical_mp4_metadata() {
        let info = parse_ffprobe_json(Path::new("clip.mp4"), SAMPLE).unwrap();
        assert_eq!(info.duration_us, 12_500_000);
        assert_eq!(info.video.codec, "h264");
        assert!(info.has_audio());
        assert_eq!(info.audio.as_ref().unwrap().sample_rate, 48_000);
        assert!((info.video.fps.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn applies_rotation_to_display_size() {
        let info = parse_ffprobe_json(Path::new("portrait.mp4"), SAMPLE).unwrap();
        assert_eq!(info.video.rotation_deg, 270);
        assert_eq!(info.display_size(), (1080, 1920));
    }

    #[test]
    fn ignores_cover_art_as_the_video_track() {
        let json = r#"{
          "format": { "duration": "3.0" },
          "streams": [
            { "codec_type": "video", "codec_name": "mjpeg", "width": 300, "height": 300,
              "disposition": { "attached_pic": 1 } },
            { "codec_type": "video", "codec_name": "h264", "width": 640, "height": 360,
              "avg_frame_rate": "25/1", "disposition": { "attached_pic": 0 } }
          ]
        }"#;
        let info = parse_ffprobe_json(Path::new("with-cover.mp4"), json).unwrap();
        assert_eq!(info.video.codec, "h264");
    }

    #[test]
    fn missing_duration_is_unsupported_not_a_panic() {
        let json = r#"{ "format": {}, "streams": [
            { "codec_type": "video", "codec_name": "h264", "width": 10, "height": 10,
              "avg_frame_rate": "0/0" } ] }"#;
        let err = parse_ffprobe_json(Path::new("mystery.bin"), json).unwrap_err();
        assert!(matches!(err, AppError::Unsupported { .. }));
    }

    #[test]
    fn audio_only_file_is_unsupported() {
        let json = r#"{ "format": { "duration": "5.0" },
            "streams": [ { "codec_type": "audio", "codec_name": "aac" } ] }"#;
        let err = parse_ffprobe_json(Path::new("song.m4a"), json).unwrap_err();
        assert!(matches!(err, AppError::Unsupported { .. }));
    }

    #[test]
    fn divides_duration_by_frame_rate_for_frame_us() {
        let info = parse_ffprobe_json(Path::new("ntsc.mp4"), SAMPLE).unwrap();
        let frame = info.frame_us().unwrap();
        assert!((frame - 33_367).abs() <= 1);
    }
}
