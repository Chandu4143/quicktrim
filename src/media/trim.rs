//! Trim selection: the one piece of domain logic that must never be wrong.
//!
//! Times are integer microseconds end to end. Floating point appears only when
//! talking to the media engine, which is the conversion tested most heavily.

use super::errors::AppError;

/// Shorter selections are almost always a mis-drag, and some encoders refuse
/// to emit a single frame this short.
pub const MIN_SELECTION_US: i64 = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimRange {
    start_us: i64,
    end_us: i64,
}

impl TrimRange {
    /// Validates against the source duration.
    pub fn new(start_us: i64, end_us: i64, duration_us: i64) -> Result<Self, AppError> {
        if duration_us <= 0 {
            return Err(AppError::Unsupported {
                detail: "Its duration is unknown.".to_string(),
            });
        }
        if start_us < 0 {
            return Err(AppError::InvalidSelection {
                detail: "The start cannot be before the beginning of the video.".to_string(),
            });
        }
        if end_us > duration_us {
            return Err(AppError::InvalidSelection {
                detail: "The end cannot be after the end of the video.".to_string(),
            });
        }
        if start_us >= end_us {
            return Err(AppError::InvalidSelection {
                detail: "The start must come before the end.".to_string(),
            });
        }
        if end_us - start_us < MIN_SELECTION_US {
            return Err(AppError::InvalidSelection {
                detail: "Select at least a fraction of a second.".to_string(),
            });
        }
        Ok(Self { start_us, end_us })
    }

    pub fn from_seconds(start: f64, end: f64, duration: f64) -> Result<Self, AppError> {
        if !start.is_finite() || !end.is_finite() || !duration.is_finite() {
            return Err(AppError::InvalidSelection {
                detail: "One of the times is not a number.".to_string(),
            });
        }
        Self::new(
            (start * 1_000_000.0).round() as i64,
            (end * 1_000_000.0).round() as i64,
            (duration * 1_000_000.0).round() as i64,
        )
    }

    pub fn start_us(&self) -> i64 {
        self.start_us
    }

    pub fn end_us(&self) -> i64 {
        self.end_us
    }

    pub fn duration_us(&self) -> i64 {
        self.end_us - self.start_us
    }

    pub fn start_secs(&self) -> f64 {
        self.start_us as f64 / 1_000_000.0
    }

    pub fn end_secs(&self) -> f64 {
        self.end_us as f64 / 1_000_000.0
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_us() as f64 / 1_000_000.0
    }

    /// Moves one boundary while keeping the range valid.
    ///
    /// Returns `None` when the move would collapse the selection, so callers
    /// can simply ignore a drag that has hit the minimum.
    pub fn with_start(mut self, start_us: i64) -> Option<Self> {
        if start_us < 0 || start_us > self.end_us - MIN_SELECTION_US {
            return None;
        }
        self.start_us = start_us;
        Some(self)
    }

    pub fn with_end(mut self, end_us: i64, duration_us: i64) -> Option<Self> {
        if end_us > duration_us || end_us < self.start_us + MIN_SELECTION_US {
            return None;
        }
        self.end_us = end_us;
        Some(self)
    }

    /// True when the range covers the whole file, in which case the "trim"
    /// is a plain save and we should say so rather than pretend.
    pub fn covers_whole_file(&self, duration_us: i64) -> bool {
        self.start_us <= 1_000 && (duration_us - self.end_us) <= 1_000
    }
}

/// `mm:ss.mmm`, or `h:mm:ss.mmm` for long media.
pub fn format_timecode(us: i64) -> String {
    let us = us.max(0);
    let total_ms = (us + 500) / 1_000;
    let ms = total_ms % 1_000;
    let total_secs = total_ms / 1_000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3_600;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}.{ms:03}")
    } else {
        format!("{mins:02}:{secs:02}.{ms:03}")
    }
}

/// Parses `mm:ss.mmm`, `h:mm:ss.mmm`, `ss`, `ss.mmm` or `mm:ss`.
pub fn parse_timecode(input: &str) -> Option<i64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() > 3 {
        return None;
    }

    let (hours, minutes, seconds) = match parts.len() {
        1 => (0.0, 0.0, parts[0].parse::<f64>().ok()?),
        2 => (0.0, parts[0].parse::<f64>().ok()?, parts[1].parse::<f64>().ok()?),
        3 => (
            parts[0].parse::<f64>().ok()?,
            parts[1].parse::<f64>().ok()?,
            parts[2].parse::<f64>().ok()?,
        ),
        _ => return None,
    };

    if !(0.0..60.0).contains(&seconds) || !(0.0..60.0).contains(&minutes) || hours < 0.0 {
        return None;
    }

    let total = hours * 3_600.0 + minutes * 60.0 + seconds;
    Some((total * 1_000_000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: i64 = 60_000_000;

    #[test]
    fn accepts_a_sensible_range() {
        let range = TrimRange::new(2_000_000, 5_500_000, DURATION).unwrap();
        assert_eq!(range.duration_us(), 3_500_000);
        assert!((range.duration_secs() - 3.5).abs() < 1e-9);
    }

    #[test]
    fn rejects_inverted_and_empty_ranges() {
        assert!(matches!(
            TrimRange::new(5_000_000, 5_000_000, DURATION),
            Err(AppError::InvalidSelection { .. })
        ));
        assert!(matches!(
            TrimRange::new(5_000_000, 4_000_000, DURATION),
            Err(AppError::InvalidSelection { .. })
        ));
    }

    #[test]
    fn rejects_out_of_bounds_times() {
        assert!(TrimRange::new(-1, 1_000_000, DURATION).is_err());
        assert!(TrimRange::new(0, DURATION + 1, DURATION).is_err());
    }

    #[test]
    fn rejects_selections_below_the_minimum() {
        assert!(TrimRange::new(1_000, 1_010, DURATION).is_err());
        assert!(TrimRange::new(1_000, 1_000 + MIN_SELECTION_US, DURATION).is_ok());
    }

    #[test]
    fn boundary_moves_never_invert_the_range() {
        let range = TrimRange::new(10_000_000, 20_000_000, DURATION).unwrap();
        assert!(range.with_start(19_999_999).is_none());
        assert_eq!(
            range.with_start(12_000_000).unwrap().start_us(),
            12_000_000
        );
        assert!(range.with_end(10_000_001, DURATION).is_none());
        assert!(range.with_end(DURATION + 1, DURATION).is_none());
    }

    #[test]
    fn seconds_and_microseconds_round_trip() {
        let range = TrimRange::from_seconds(1.234567, 4.5, 10.0).unwrap();
        assert_eq!(range.start_us(), 1_234_567);
        assert!((range.start_secs() - 1.234567).abs() < 1e-9);
    }

    #[test]
    fn non_finite_seconds_are_rejected() {
        assert!(TrimRange::from_seconds(f64::NAN, 1.0, 10.0).is_err());
        assert!(TrimRange::from_seconds(0.0, f64::INFINITY, 10.0).is_err());
    }

    #[test]
    fn formats_short_and_long_timecodes() {
        assert_eq!(format_timecode(0), "00:00.000");
        assert_eq!(format_timecode(1_500_000), "00:01.500");
        assert_eq!(format_timecode(65_250_000), "01:05.250");
        assert_eq!(format_timecode(3_725_001_000), "1:02:05.001");
    }

    #[test]
    fn parses_common_timecode_forms() {
        assert_eq!(parse_timecode("1:05.25"), Some(65_250_000));
        assert_eq!(parse_timecode("01:02:05.001"), Some(3_725_001_000));
        assert_eq!(parse_timecode("12.5"), Some(12_500_000));
        assert_eq!(parse_timecode("1:99"), None);
        assert_eq!(parse_timecode("abc"), None);
    }

    #[test]
    fn whole_file_selection_is_detected() {
        let range = TrimRange::new(0, DURATION, DURATION).unwrap();
        assert!(range.covers_whole_file(DURATION));
        let range = TrimRange::new(0, DURATION - 5_000, DURATION).unwrap();
        assert!(!range.covers_whole_file(DURATION));
    }
}
