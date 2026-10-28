//! End-to-end tests against real media, driven by the bundled FFmpeg.
//!
//! These are the tests that prove the promise in the README: the original is
//! never touched, the cut lands where the user asked, and a cancelled export
//! leaves nothing behind. They generate their own fixtures so they run on any
//! machine that has the bundled `ffmpeg`/`ffprobe` next to the test binary.
//!
//! Fixtures are built once per run in `target/it-fixtures` and reused, so a
//! second `cargo test` is fast.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use quicktrim::media::errors::AppError;
use quicktrim::media::export::{self, ExportMode};
use quicktrim::media::metadata;
use quicktrim::media::trim::TrimRange;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Points the media engine at the helpers that sit next to the test binary.
fn setup() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug");
        let ffmpeg = bin.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        let ffprobe = bin.join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        assert!(
            ffmpeg.is_file(),
            "missing {}: copy the bundled FFmpeg next to the binary before running these tests",
            ffmpeg.display()
        );
        assert!(ffprobe.is_file(), "missing {}", ffprobe.display());
        // Edition 2021: the test process owns these for its whole lifetime.
        std::env::set_var("QUICKTRIM_FFMPEG", &ffmpeg);
        std::env::set_var("QUICKTRIM_FFPROBE", &ffprobe);
    });
}

fn fixture_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        setup();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/it-fixtures");
        std::fs::create_dir_all(&dir).expect("create fixture directory");
        dir
    })
}

fn ffmpeg(args: &[&str]) {
    let exe = std::env::var("QUICKTRIM_FFMPEG").expect("ffmpeg path");
    let output = std::process::Command::new(exe)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("run ffmpeg");
    assert!(
        output.status.success(),
        "fixture generation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Fixture generation is serialised: two tests asking for the same file at the
/// same time must not write it simultaneously.
fn fixture_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture(name: &str, build: impl FnOnce(&Path)) -> PathBuf {
    let _guard = fixture_lock();
    let path = fixture_dir().join(name);
    if !path.is_file() {
        build(&path);
        assert!(path.is_file(), "ffmpeg did not produce {}", path.display());
    }
    path
}

/// The shared `testsrc2` + `sine` source, with output options appended.
fn testsrc(duration: u32, size: &str) -> Vec<String> {
    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("testsrc2=size={size}:rate=30:duration={duration}"),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("sine=frequency=440:duration={duration}"),
    ]
}

/// Runs an FFmpeg command assembled from `testsrc` plus `tail`.
fn synth(path: &Path, mut args: Vec<String>, tail: &[&str]) {
    args.extend(tail.iter().map(|s| s.to_string()));
    args.push(path.to_string_lossy().into_owned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    ffmpeg(&refs);
}

fn short_gop() -> PathBuf {
    fixture("short-gop.mp4", |path| {
        synth(
            path,
            testsrc(12, "640x360"),
            &[
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-keyint_min",
                "30",
                "-sc_threshold",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
            ],
        )
    })
}

fn long_gop() -> PathBuf {
    fixture("long-gop.mp4", |path| {
        synth(
            path,
            testsrc(12, "640x360"),
            &[
                "-c:v",
                "libx264",
                "-g",
                "300",
                "-keyint_min",
                "300",
                "-sc_threshold",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
            ],
        )
    })
}

fn silent() -> PathBuf {
    fixture("silent.mp4", |path| {
        synth(
            path,
            testsrc(8, "640x360"),
            &[
                "-map",
                "0:v:0",
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
            ],
        )
    })
}

fn portrait() -> PathBuf {
    fixture("portrait.mp4", |path| {
        synth(
            path,
            testsrc(8, "360x640"),
            &[
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
                "-c:a",
                "aac",
            ],
        )
    })
}

fn mov() -> PathBuf {
    fixture("clip.mov", |path| {
        synth(
            path,
            testsrc(8, "640x360"),
            &[
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
                "-c:a",
                "aac",
            ],
        )
    })
}

fn mkv() -> PathBuf {
    fixture("clip.mkv", |path| {
        synth(
            path,
            testsrc(8, "640x360"),
            &[
                "-c:v",
                "libx264",
                "-g",
                "30",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "22",
                "-c:a",
                "aac",
            ],
        )
    })
}

fn webm() -> PathBuf {
    fixture("clip.webm", |path| {
        synth(
            path,
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "testsrc2=size=320x180:rate=30:duration=6".into(),
            ],
            &[
                "-c:v",
                "libvpx-vp9",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
                "-b:v",
                "600k",
            ],
        )
    })
}

fn info(path: &Path) -> metadata::MediaInfo {
    metadata::probe(path).unwrap_or_else(|err| panic!("probe {}: {err:?}", path.display()))
}

fn range(media: &metadata::MediaInfo, start: f64, end: f64) -> TrimRange {
    TrimRange::from_seconds(start, end, media.duration_secs()).expect("valid range")
}

fn fingerprint(path: &Path) -> (u64, u64) {
    let bytes = std::fs::read(path).expect("read source");
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let len = bytes.len() as u64;
    (hasher.finish(), len)
}

/// Every export in a directory must leave exactly one file, the result.
fn assert_no_partials(dir: &Path) {
    let leftovers: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".qt-part-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files were not cleaned up: {leftovers:?}"
    );
}

fn export_into(media: &metadata::MediaInfo, from: f64, to: f64) -> (export::ExportPlan, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    // Keep the directory alive by leaking it: the test process is short-lived
    // and the OS reclaims the temp tree.
    let dir = dir.keep();
    let output = dir.join("out.mp4");
    let plan = export::plan_to(media, range(media, from, to), Some(output)).expect("plan");
    (plan, dir)
}

// ---------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------

#[test]
fn probes_a_real_file() {
    let media = info(&short_gop());
    assert_eq!(media.video.codec, "h264");
    assert_eq!(media.display_size(), (640, 360));
    assert!(media.has_audio());
    assert!((media.duration_secs() - 12.0).abs() < 0.2);
    assert!(!media.path.as_os_str().is_empty());
}

#[test]
fn reports_portrait_dimensions() {
    let media = info(&portrait());
    assert_eq!(media.display_size(), (360, 640));
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

#[test]
fn aligned_cut_plans_a_stream_copy() {
    let media = info(&short_gop());
    let plan = export::plan(&media, range(&media, 4.0, 8.0)).expect("plan");
    assert_eq!(
        plan.mode,
        ExportMode::StreamCopy,
        "1 s GOP with cuts on keyframes should not re-encode"
    );
}

#[test]
fn unaligned_cut_plans_a_re_encode() {
    let media = info(&long_gop());
    let plan = export::plan(&media, range(&media, 4.35, 9.62)).expect("plan");
    assert_eq!(
        plan.mode,
        ExportMode::ReEncode,
        "a cut inside a 10 s GOP must re-encode"
    );
}

#[test]
fn refuses_to_overwrite_the_source() {
    let media = info(&short_gop());
    let err = export::plan_to(&media, range(&media, 1.0, 3.0), Some(media.path.clone()))
        .expect_err("source must be refused");
    assert!(matches!(err, AppError::Io { .. }));
}

// ---------------------------------------------------------------------------
// Exporting
// ---------------------------------------------------------------------------

#[test]
fn keyframe_aligned_export_is_fast_and_exact() {
    let media = info(&short_gop());
    let (plan, dir) = export_into(&media, 4.0, 9.0);

    let progress = std::sync::Mutex::new(Vec::<f32>::new());
    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |p| {
        progress.lock().unwrap().push(p.fraction);
    })
    .expect("export");

    assert_eq!(outcome.mode_used, ExportMode::StreamCopy);
    assert!(outcome.output.is_file());
    let result = info(&outcome.output);
    let drift = (result.duration_secs() - 5.0).abs();
    assert!(drift < 0.35, "duration drifted by {drift:.3} s");
    assert_eq!(result.display_size(), (640, 360));
    assert!(result.has_audio(), "sound must survive a stream copy");

    let fractions = progress.lock().unwrap().clone();
    assert!(!fractions.is_empty(), "progress must be reported");
    assert!(fractions.windows(2).all(|w| w[1] >= w[0]), "progress regressed: {fractions:?}");
    assert_eq!(*fractions.last().unwrap(), 1.0);
    assert_no_partials(&dir);
}

#[test]
fn unaligned_export_re_encodes_with_accurate_duration() {
    let media = info(&long_gop());
    let (plan, dir) = export_into(&media, 3.37, 8.62);

    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {}).expect("export");
    assert_eq!(outcome.mode_used, ExportMode::ReEncode);

    let result = info(&outcome.output);
    let expected = 8.62 - 3.37;
    let drift = (result.duration_secs() - expected).abs();
    assert!(drift < 0.25, "duration drifted by {drift:.3} s");
    assert_eq!(result.video.codec, "h264");
    assert_eq!(result.display_size(), (640, 360));
    assert_no_partials(&dir);
}

#[test]
fn exporting_never_modifies_the_source() {
    let media = info(&short_gop());
    let before = fingerprint(&media.path);
    let (plan, _dir) = export_into(&media, 2.5, 6.1);
    export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {}).expect("export");
    let after = fingerprint(&media.path);
    assert_eq!(before, after, "the source file changed during export");
}

#[test]
fn cancelled_export_leaves_nothing_behind() {
    let media = info(&short_gop());
    let (plan, dir) = export_into(&media, 2.0, 10.0);
    let cancel = Arc::new(AtomicBool::new(true));

    let err = export::run(&plan, cancel, |_| {}).expect_err("must be cancelled");
    assert!(matches!(err, AppError::Cancelled));
    assert!(!plan.output.exists(), "a cancelled export published a file");
    assert_no_partials(&dir);
}

#[test]
fn cancelling_mid_export_kills_ffmpeg_and_allows_a_retry() {
    let media = info(&long_gop());
    let (plan, dir) = export_into(&media, 0.5, 11.5);
    let cancel = Arc::new(AtomicBool::new(false));

    // Cancel from the progress callback: this runs on the export thread, and
    // the engine checks the flag on every loop, so the child is killed while
    // it is still encoding.
    let flag = cancel.clone();
    let progress_flag = flag.clone();
    let err = export::run(&plan, flag, move |p| {
        if p.fraction > 0.0 {
            progress_flag.store(true, Ordering::Relaxed);
        }
    })
    .expect_err("a cancelled export must not report success");

    assert!(matches!(err, AppError::Cancelled));
    assert!(!plan.output.exists(), "cancelled export published a file");
    assert_no_partials(&dir);

    // The killed process must have released the temp file: a retry with the
    // same plan has to succeed immediately.
    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {})
        .expect("retry after cancellation");
    assert!(outcome.output.is_file());
    assert_no_partials(&dir);
}

#[test]
fn refuses_to_clobber_an_existing_output() {
    let media = info(&short_gop());
    let (plan, _dir) = export_into(&media, 1.0, 2.0);
    std::fs::write(&plan.output, b"not a video").expect("seed output");
    let err = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {})
        .expect_err("must refuse");
    assert!(matches!(err, AppError::Io { .. }));
    assert_eq!(std::fs::read(&plan.output).unwrap(), b"not a video");
}

#[test]
fn silent_video_exports_without_an_audio_track() {
    let media = info(&silent());
    assert!(!media.has_audio());
    let (plan, dir) = export_into(&media, 1.0, 4.0);
    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {}).expect("export");
    let result = info(&outcome.output);
    assert!(!result.has_audio());
    assert!((result.duration_secs() - 3.0).abs() < 0.3);
    assert_no_partials(&dir);
}

#[test]
fn mov_and_mkv_inputs_produce_mp4() {
    for source in [mov(), mkv()] {
        let media = info(&source);
        let (plan, dir) = export_into(&media, 1.2, 4.4);
        let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {})
            .unwrap_or_else(|err| panic!("export {}: {err:?}", source.display()));
        assert_eq!(
            outcome.output.extension().and_then(|e| e.to_str()),
            Some("mp4")
        );
        let result = info(&outcome.output);
        assert!((result.duration_secs() - 3.2).abs() < 0.3);
        assert!(result.has_audio());
        assert_no_partials(&dir);
    }
}

#[test]
fn webm_input_is_transcoded_to_mp4() {
    let media = info(&webm());
    assert_eq!(media.video.codec, "vp9");
    assert!(
        !media.has_audio(),
        "the webm fixture is video-only on purpose"
    );

    let (plan, dir) = export_into(&media, 1.0, 4.0);
    assert_eq!(plan.mode, ExportMode::ReEncode, "vp9 is not safe in MP4");
    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {}).expect("export");
    let result = info(&outcome.output);
    assert_eq!(result.video.codec, "h264");
    assert_eq!(result.display_size(), (320, 180));
    assert_no_partials(&dir);
}

#[test]
fn portrait_video_keeps_its_shape() {
    let media = info(&portrait());
    let (plan, dir) = export_into(&media, 1.0, 3.5);
    let outcome = export::run(&plan, Arc::new(AtomicBool::new(false)), |_| {}).expect("export");
    let result = info(&outcome.output);
    assert_eq!(result.display_size(), (360, 640));
    assert_no_partials(&dir);
}
