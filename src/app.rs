//! The application shell: owns the window, the player and the export job.
//!
//! Threading contract:
//! * worker threads never touch Slint; they push messages into channels;
//! * one UI timer drains those channels and is the only place that mutates
//!   window properties;
//! * the media player has its own thread and is joined on drop.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::{
    CloseRequestResponse, ComponentHandle, Image, SharedPixelBuffer, SharedString, Timer, TimerMode,
    Weak,
};

use crate::media::errors::AppError;
use crate::media::export::{self, ExportOutcome};
use crate::media::metadata::{self, MediaInfo};
use crate::media::playback::{Player, PlayerEvent, VideoFrame};
use crate::media::trim::{format_timecode, TrimRange, MIN_SELECTION_US};
use crate::output;
use crate::platform;
use crate::MainWindow;

/// Preview frames are rendered at a fixed comfortable size. Slint letterboxes
/// them; rendering at the exact pane size is a phase-5 optimisation.
const PREVIEW_WIDTH: u32 = 1280;
const PREVIEW_HEIGHT: u32 = 720;

enum ProbeResult {
    Ok(PathBuf, MediaInfo),
    Err(AppError),
}

enum ExportMessage {
    Progress(f32),
    Finished(Result<ExportOutcome, AppError>),
}

struct ExportHandle {
    cancel: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

struct Session {
    info: MediaInfo,
    range: TrimRange,
    output: Option<PathBuf>,
    output_choice: Option<PathBuf>,
}

/// Latest frame, with only one slot: a slow UI drops frames instead of
/// building a backlog.
#[derive(Default)]
struct FrameSlot {
    latest: Mutex<Option<VideoFrame>>,
}

impl FrameSlot {
    fn push(&self, frame: VideoFrame) {
        if let Ok(mut slot) = self.latest.lock() {
            *slot = Some(frame);
        }
    }

    fn take(&self) -> Option<VideoFrame> {
        self.latest.lock().ok().and_then(|mut slot| slot.take())
    }
}

pub struct App {
    ui: Weak<MainWindow>,
    session: RefCell<Option<Session>>,
    player: RefCell<Option<Player>>,
    probe_rx: RefCell<Option<Receiver<ProbeResult>>>,
    player_events: RefCell<Option<Receiver<PlayerEvent>>>,
    frames: RefCell<Option<Arc<FrameSlot>>>,
    export_rx: RefCell<Option<Receiver<ExportMessage>>>,
    export_handle: RefCell<Option<ExportHandle>>,
    exporting: Cell<bool>,
    last_scrub: Cell<Option<Instant>>,
    last_boundary: Cell<Boundary>,
    ticks: Cell<u32>,
    timer: RefCell<Option<Timer>>,
}

impl App {
    pub fn new(ui: Weak<MainWindow>) -> Rc<Self> {
        Rc::new(Self {
            ui,
            session: RefCell::new(None),
            player: RefCell::new(None),
            probe_rx: RefCell::new(None),
            player_events: RefCell::new(None),
            frames: RefCell::new(None),
            export_rx: RefCell::new(None),
            export_handle: RefCell::new(None),
            exporting: Cell::new(false),
            last_scrub: Cell::new(None),
            last_boundary: Cell::new(Boundary::Start),
            ticks: Cell::new(0),
            timer: RefCell::new(None),
        })
    }

    /// Wires every callback and starts the UI tick that drains worker threads.
    pub fn start(self: &Rc<Self>, ui: &MainWindow) {
        let app = Rc::downgrade(self);

        ui.on_open_video({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.choose_file();
                }
            }
        });

        ui.on_save_as({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.choose_output();
                }
            }
        });

        ui.on_close_window({
            let ui = ui.as_weak();
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.hide().ok();
                }
            }
        });

        ui.on_toggle_play({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.player_call(|player| player.toggle_play());
                }
            }
        });

        ui.on_toggle_mute({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.toggle_mute();
                }
            }
        });

        ui.on_seek_requested({
            let app = app.clone();
            move |ms| {
                if let Some(app) = app.upgrade() {
                    app.seek_ms(ms);
                }
            }
        });

        ui.on_seek_start({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    if let Some(start) = app.range().map(|r| r.start_us()) {
                        app.seek_us(start);
                    }
                }
            }
        });

        ui.on_seek_end({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    if let Some(end) = app.range().map(|r| r.end_us()) {
                        app.seek_us(end);
                    }
                }
            }
        });

        ui.on_start_changed({
            let app = app.clone();
            move |ms| {
                if let Some(app) = app.upgrade() {
                    app.move_boundary(ms, Boundary::Start, true);
                }
            }
        });

        ui.on_end_changed({
            let app = app.clone();
            move |ms| {
                if let Some(app) = app.upgrade() {
                    app.move_boundary(ms, Boundary::End, true);
                }
            }
        });

        ui.on_scrub_ended({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.last_scrub.set(None);
                    if let Some(range) = app.range() {
                        let target = match app.last_boundary.get() {
                            Boundary::Start => range.start_us(),
                            Boundary::End => range.end_us(),
                        };
                        app.seek_us(target);
                    }
                }
            }
        });

        ui.on_set_playing_at({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.set_start_at_playhead();
                }
            }
        });

        ui.on_set_ended_at({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.set_end_at_playhead();
                }
            }
        });

        ui.on_save_requested({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.start_export();
                }
            }
        });

        ui.on_cancel_export({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.cancel_export();
                }
            }
        });

        ui.on_reveal_output({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    if let Some(path) = app.output_path() {
                        let _ = platform::reveal(&path);
                    }
                }
            }
        });

        ui.on_file_dropped({
            let app = app.clone();
            move |data: slint::private_unstable_api::re_exports::DataTransfer| {
                let Some(app) = app.upgrade() else { return };
                if let Ok(paths) = data.file_paths() {
                    if let Some(path) = paths.into_iter().next() {
                        app.open(path.to_path_buf());
                    }
                }
            }
        });

        ui.window().on_close_requested({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.shutdown();
                }
                CloseRequestResponse::HideWindow
            }
        });

        // Keyboard shortcuts (window-wide focus scope).
        ui.on_toggle_play_requested({
            let app = app.clone();
            move || {
                if let Some(app) = app.upgrade() {
                    app.player_call(|player| player.toggle_play());
                }
            }
        });

        ui.on_nudge({
            let app = app.clone();
            move |delta_ms| {
                if let Some(app) = app.upgrade() {
                    let position = app.position_ms() + delta_ms;
                    app.seek_ms(position);
                }
            }
        });

        ui.on_request_close({
            let ui = ui.as_weak();
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.hide().ok();
                }
            }
        });

        ui.on_dismiss_message({
            let ui = ui.as_weak();
            move || {
                if let Some(ui) = ui.upgrade() {
                    ui.set_notice_text(SharedString::new());
                    ui.set_error_text(SharedString::new());
                }
            }
        });

        // The tick: 60 Hz is cheap (two empty channel reads) and keeps the
        // playhead and preview smooth.
        let timer = Timer::default();
        {
            let app = app.clone();
            timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                if let Some(app) = app.upgrade() {
                    app.tick();
                }
            });
        }
        *self.timer.borrow_mut() = Some(timer);
    }

    // -- file loading -------------------------------------------------------

    pub fn open(self: &Rc<Self>, path: PathBuf) {
        if self.exporting.get() {
            self.notice("Finish or cancel the current export first.");
            return;
        }

        let Some(ui) = self.ui.upgrade() else { return };
        if !path.is_file() {
            self.show_error(&AppError::InputUnavailable {
                detail: format!("“{}” was not found.", path.display()),
            });
            return;
        }

        ui.set_error_text(SharedString::new());
        ui.set_notice_text(SharedString::new());
        ui.set_output_ready(false);
        ui.set_loading(true);
        ui.set_has_video(false);
        ui.set_preview_image(Image::default());
        ui.set_playing(false);
        ui.set_position_ms(0);
        ui.set_position_text("00:00.000".into());
        ui.set_duration_ms(0);
        ui.set_duration_text("--:--.---".into());
        ui.set_start_text("--:--.---".into());
        ui.set_end_text("--:--.---".into());
        ui.set_selection_text("--:--.---".into());
        ui.set_output_ready(false);
        ui.set_output_path(output::default_output_path(&path).display().to_string().into());
        ui.set_file_name(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
                .into(),
        );

        *self.session.borrow_mut() = None;

        let (tx, rx) = std::sync::mpsc::channel();
        *self.probe_rx.borrow_mut() = Some(rx);

        std::thread::Builder::new()
            .name("quicktrim-probe".into())
            .spawn(move || {
                let result = match metadata::probe(&path) {
                    Ok(info) => ProbeResult::Ok(path, info),
                    Err(err) => ProbeResult::Err(err),
                };
                let _ = tx.send(result);
            })
            .ok();
    }

    fn finish_open(self: &Rc<Self>, path: PathBuf, info: MediaInfo) {
        let Some(ui) = self.ui.upgrade() else { return };

        let duration = info.duration_us;
        let suggested_output = output::default_output_path(&info.path);
        let Ok(range) = TrimRange::new(0, duration, duration) else {
            self.show_error(&AppError::Unsupported {
                detail: "The video is too short to trim.".to_string(),
            });
            return;
        };

        if let Err(err) = self.ensure_player() {
            self.show_error(&err);
        }

        if let Some(player) = self.player.borrow().as_ref() {
            player.set_bounds(0.0, info.duration_secs());
            player.set_view_size(PREVIEW_WIDTH, PREVIEW_HEIGHT);
            player.load(&path);
        }

        *self.session.borrow_mut() = Some(Session {
            info,
            range,
            output: None,
            output_choice: None,
        });

        ui.set_has_video(true);
        ui.set_loading(false);
        ui.set_playing(false);
        ui.set_muted(false);
        ui.set_exporting(false);
        ui.set_export_progress(0.0);
        ui.set_position_text("00:00.000".into());
        ui.set_duration_ms((duration / 1_000) as i32);
        ui.set_duration_text(format_timecode(duration).into());
        ui.set_output_path(suggested_output.display().to_string().into());
        self.push_range(&ui);
        ui.set_preview_image(Image::default());
    }

    fn ensure_player(self: &Rc<Self>) -> Result<(), AppError> {
        if self.player.borrow().is_some() {
            return Ok(());
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let slot = Arc::new(FrameSlot::default());
        let slot_for_thread = slot.clone();
        let on_frame = Box::new(move |frame: VideoFrame| slot_for_thread.push(frame));
        let on_event = Box::new(move |event: PlayerEvent| {
            let _ = tx.send(event);
        });

        let player = Player::spawn(on_frame, on_event)?;
        *self.player.borrow_mut() = Some(player);
        *self.player_events.borrow_mut() = Some(rx);
        *self.frames.borrow_mut() = Some(slot);
        Ok(())
    }

    // -- tick and events ----------------------------------------------------

    fn tick(self: &Rc<Self>) {
        self.drain_probe();
        self.drain_frames();
        self.drain_player_events();
        self.drain_export();
        self.keep_surface_alive();
    }

    /// Forces a full repaint roughly once a second. See the `surface-tick`
    /// comment in `main-window.slint` for why this is necessary.
    fn keep_surface_alive(self: &Rc<Self>) {
        let ticks = self.ticks.get().wrapping_add(1);
        self.ticks.set(ticks);
        if ticks % 60 != 0 {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_surface_tick(!ui.get_surface_tick());
        }
    }

    fn drain_probe(self: &Rc<Self>) {
        let result = {
            let rx = self.probe_rx.borrow();
            match rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
                Some(result) => result,
                None => return,
            }
        };
        *self.probe_rx.borrow_mut() = None;

        match result {
            ProbeResult::Ok(path, info) => self.finish_open(path, info),
            ProbeResult::Err(err) => {
                *self.session.borrow_mut() = None;
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_loading(false);
                    ui.set_has_video(false);
                }
                self.show_error(&err);
            }
        }
    }

    fn drain_frames(&self) {
        let Some(slot) = self.frames.borrow().clone() else {
            return;
        };
        let Some(frame) = slot.take() else { return };
        let Some(ui) = self.ui.upgrade() else { return };

        let mut buffer = SharedPixelBuffer::<slint::Rgb8Pixel>::new(frame.width, frame.height);
        let expected = frame.width as usize * frame.height as usize * 3;
        if frame.rgb.len() != expected || buffer.make_mut_bytes().len() != expected {
            return;
        }
        buffer.make_mut_bytes().copy_from_slice(&frame.rgb);
        ui.set_preview_image(Image::from_rgb8(buffer));

        // Frames are a smoother clock than the 20 Hz time events.
        if self.is_playing() {
            self.set_position((frame.position_secs * 1_000_000.0) as i64);
        }
    }

    fn drain_player_events(self: &Rc<Self>) {
        let events: Vec<PlayerEvent> = {
            let rx = self.player_events.borrow();
            match rx.as_ref() {
                Some(rx) => {
                    let mut events = Vec::new();
                    loop {
                        match rx.try_recv() {
                            Ok(event) => events.push(event),
                            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                        }
                    }
                    events
                }
                None => return,
            }
        };

        for event in events {
            match event {
                PlayerEvent::Loaded { duration_secs } => {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_duration_ms((duration_secs * 1e3) as i32);
                        ui.set_duration_text(format_timecode((duration_secs * 1e6) as i64).into());
                    }
                }
                PlayerEvent::Time {
                    position_secs,
                    paused,
                } => {
                    if !paused {
                        self.set_position((position_secs * 1e6) as i64);
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_playing(true);
                        }
                    } else {
                        self.set_position((position_secs * 1e6) as i64);
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_playing(false);
                        }
                    }
                }
                PlayerEvent::Ended => {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_playing(false);
                    }
                }
                PlayerEvent::MediaError(_) => {
                    self.show_error(&AppError::Corrupt {
                        detail: "The player could not decode this video.".to_string(),
                    });
                }
                PlayerEvent::MissingLibrary => {
                    self.show_error(&AppError::ToolMissing {
                        tool: "libmpv".to_string(),
                    });
                }
            }
        }
    }

    fn drain_export(self: &Rc<Self>) {
        let messages: Vec<ExportMessage> = {
            let rx = self.export_rx.borrow();
            match rx.as_ref() {
                Some(rx) => {
                    let mut messages = Vec::new();
                    loop {
                        match rx.try_recv() {
                            Ok(message) => messages.push(message),
                            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                        }
                    }
                    messages
                }
                None => return,
            }
        };

        for message in messages {
            match message {
                ExportMessage::Progress(fraction) => {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_export_progress(fraction);
                    }
                }
                ExportMessage::Finished(result) => {
                    self.exporting.set(false);
                    if let Some(handle) = self.export_handle.borrow().as_ref() {
                        handle.finished.store(true, Ordering::SeqCst);
                    }
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_exporting(false);
                        match &result {
                            Ok(outcome) => {
                                ui.set_export_progress(1.0);
                                ui.set_notice_label("SAVED".into());
                                ui.set_notice_positive(true);
                                ui.set_notice_text(
                                    format!("Saved “{}”", display_name(&outcome.output)).into(),
                                );
                                ui.set_output_ready(true);
                                ui.set_output_path(outcome.output.display().to_string().into());
                                if let Some(session) = self.session.borrow_mut().as_mut() {
                                    session.output = Some(outcome.output.clone());
                                    session.output_choice = None;
                                }
                            }
                            Err(AppError::Cancelled) => {
                                ui.set_export_progress(0.0);
                                ui.set_notice_label("CANCELLED".into());
                                ui.set_notice_positive(false);
                                ui.set_notice_text(
                                    "Export cancelled. The original video is unchanged.".into(),
                                );
                            }
                            Err(err) => {
                                ui.set_export_progress(0.0);
                                ui.set_error_text(err.message().into());
                            }
                        }
                    }
                }
            }
        }
    }

    // -- selection ----------------------------------------------------------

    fn range(&self) -> Option<TrimRange> {
        self.session.borrow().as_ref().map(|session| session.range)
    }

    fn duration_us(&self) -> i64 {
        self.session
            .borrow()
            .as_ref()
            .map(|session| session.info.duration_us)
            .unwrap_or(0)
    }

    fn position_ms(&self) -> i32 {
        self.ui
            .upgrade()
            .map(|ui| ui.get_position_ms())
            .unwrap_or(0)
    }

    fn is_playing(&self) -> bool {
        self.ui.upgrade().map(|ui| ui.get_playing()).unwrap_or(false)
    }

    fn seek_us(&self, position_us: i64) {
        let duration = self.duration_us();
        let clamped = position_us.clamp(0, duration);
        self.player_call(|player| player.seek(clamped as f64 / 1e6));
        self.set_position(clamped);
    }

    fn seek_ms(&self, ms: i32) {
        self.seek_us(ms as i64 * 1_000);
    }

    fn set_position(&self, position_us: i64) {
        let duration = self.duration_us().max(1);
        let clamped = position_us.clamp(0, duration);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_position_ms((clamped / 1_000) as i32);
            ui.set_position_text(format_timecode(clamped).into());
        }
    }

    fn move_boundary(self: &Rc<Self>, ms: i32, boundary: Boundary, scrub: bool) {
        let duration = self.duration_us();
        let requested = ms as i64 * 1_000;
        self.last_boundary.set(boundary);
        let new_range = {
            let mut session = self.session.borrow_mut();
            let Some(session) = session.as_mut() else {
                return;
            };
            let range = session.range;
            let updated = match boundary {
                Boundary::Start => range.with_start(requested),
                Boundary::End => range.with_end(requested, duration),
            };
            if let Some(updated) = updated {
                session.range = updated;
                Some(updated)
            } else {
                None
            }
        };

        let Some(range) = new_range else { return };
        if let Some(player) = self.player.borrow().as_ref() {
            player.set_bounds(range.start_secs(), range.end_secs());
        }
        if let Some(ui) = self.ui.upgrade() {
            self.push_range(&ui);
        }

        if scrub {
            // While dragging, seek at most 20 times a second so the decoder
            // is not flooded; the release handler sends the final position.
            let now = Instant::now();
            let due = self
                .last_scrub
                .get()
                .map(|last| now.duration_since(last) > Duration::from_millis(50))
                .unwrap_or(true);
            if due {
                self.last_scrub.set(Some(now));
                let target = match boundary {
                    Boundary::Start => range.start_us(),
                    Boundary::End => range.end_us(),
                };
                self.seek_us(target);
            }
        }
    }

    fn set_start_at_playhead(self: &Rc<Self>) {
        let position = self.position_ms();
        self.move_boundary(position, Boundary::Start, false);
        if let Some(start) = self.range().map(|r| r.start_us()) {
            self.seek_us(start);
        }
    }

    fn set_end_at_playhead(self: &Rc<Self>) {
        let position = self.position_ms();
        self.move_boundary(position, Boundary::End, false);
        if let Some(end) = self.range().map(|r| r.end_us()) {
            self.seek_us(end);
        }
    }

    fn push_range(&self, ui: &MainWindow) {
        let Some(range) = self.range() else { return };
        ui.set_start_ms((range.start_us() / 1_000) as i32);
        ui.set_end_ms((range.end_us() / 1_000) as i32);
        ui.set_start_text(format_timecode(range.start_us()).into());
        ui.set_end_text(format_timecode(range.end_us()).into());
        ui.set_selection_text(format_timecode(range.duration_us()).into());
        ui.set_min_gap_ms((MIN_SELECTION_US / 1_000) as i32);
    }

    // -- player helpers -----------------------------------------------------

    fn player_call(&self, f: impl FnOnce(&Player)) {
        if let Some(player) = self.player.borrow().as_ref() {
            f(player);
        }
    }

    fn toggle_mute(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let muted = !ui.get_muted();
        ui.set_muted(muted);
        self.player_call(|player| player.set_mute(muted));
    }

    // -- file pickers -------------------------------------------------------

    fn choose_file(self: &Rc<Self>) {
        if self.exporting.get() {
            self.notice("Finish or cancel the current export first.");
            return;
        }
        if let Some(path) = platform::pick_video() {
            self.open(path);
        }
    }

    fn choose_output(&self) {
        let suggested = {
            let session = self.session.borrow();
            let Some(session) = session.as_ref() else {
                return;
            };
            output::default_output_path(&session.info.path)
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save trimmed video")
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(
                suggested
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "clip_trimmed.mp4".to_string()),
            );
        if let Some(parent) = suggested.parent() {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = dialog.save_file() {
            if let Some(session) = self.session.borrow_mut().as_mut() {
                let mut path = path;
                if path.extension().is_none() {
                    path.set_extension("mp4");
                }
                session.output_choice = Some(path.clone());
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_output_path(path.display().to_string().into());
                }
            }
        }
    }

    // -- export -------------------------------------------------------------

    fn start_export(self: &Rc<Self>) {
        if self.exporting.get() {
            return;
        }

        let Some((info, range, requested)) = ({
            let session = self.session.borrow();
            session.as_ref().map(|session| {
                (
                    session.info.clone(),
                    session.range,
                    session.output_choice.clone(),
                )
            })
        }) else {
            return;
        };

        let output = requested.unwrap_or_else(|| output::default_output_path(&info.path));
        let output = output::unique_path(&output);

        // Validate once more at the boundary; the UI cannot hand us a range
        // that exceeds the file, but this is the last gate before writing.
        if let Err(err) = TrimRange::new(range.start_us(), range.end_us(), info.duration_us) {
            self.show_error(&err);
            return;
        }

        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_error_text(SharedString::new());
        ui.set_notice_text(SharedString::new());
        ui.set_output_ready(false);
        ui.set_output_path(output.display().to_string().into());
        ui.set_exporting(true);
        ui.set_export_progress(0.0);
        self.exporting.set(true);

        let cancel = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        *self.export_handle.borrow_mut() = Some(ExportHandle {
            cancel: cancel.clone(),
            finished: finished.clone(),
        });

        let (tx, rx) = std::sync::mpsc::channel();
        *self.export_rx.borrow_mut() = Some(rx);

        std::thread::Builder::new()
            .name("quicktrim-export".into())
            .spawn(move || {
                let result = (|| {
                    let plan = export::plan_to(&info, range, Some(output))?;
                    export::run(&plan, cancel, |progress| {
                        let _ = tx.send(ExportMessage::Progress(progress.fraction));
                    })
                })();
                let _ = tx.send(ExportMessage::Finished(result));
                finished.store(true, Ordering::SeqCst);
            })
            .ok();
    }

    fn cancel_export(&self) {
        if let Some(handle) = self.export_handle.borrow().as_ref() {
            handle.cancel.store(true, Ordering::SeqCst);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_notice_label("CANCELLING".into());
            ui.set_notice_positive(false);
            ui.set_notice_text("Cancelling…".into());
        }
    }

    fn output_path(&self) -> Option<PathBuf> {
        self.session
            .borrow()
            .as_ref()
            .and_then(|session| session.output.clone())
    }

    // -- lifecycle ----------------------------------------------------------

    fn shutdown(&self) {
        // Cancel any export and give FFmpeg a moment to exit cleanly, so the
        // app never closes with an orphaned child writing to disk.
        if let Some(handle) = self.export_handle.borrow().as_ref() {
            handle.cancel.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_millis(3_000);
            while !handle.finished.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
            }
        }

        // Dropping the player joins the playback thread and destroys libmpv.
        let _ = self.player.borrow_mut().take();
    }

    // -- UI helpers ---------------------------------------------------------

    fn show_error(&self, err: &AppError) {
        eprintln!("[quicktrim] {}", err.diagnostic());
        if let Some(ui) = self.ui.upgrade() {
            ui.set_loading(false);
            ui.set_error_text(err.message().into());
            ui.set_notice_text(SharedString::new());
        }
    }

    fn notice(&self, message: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_notice_label("NOTE".into());
            ui.set_notice_positive(false);
            ui.set_notice_text(message.into());
        }
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Start,
    End,
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
