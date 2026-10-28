//! Playback through libmpv's render API.
//!
//! Why libmpv: correct A/V sync, seeking, VFR handling, rotation and codec
//! coverage are all hard problems that mpv already solves identically on
//! Windows, macOS and Linux. QuickTrim only needs to turn mpv's frames into a
//! Slint image, which the software render API makes straightforward.
//!
//! The library is loaded at runtime (`libmpv-2.dll` / `libmpv.2.dylib` /
//! `libmpv.so.2`), so a missing copy produces a clear message instead of a
//! loader failure at startup.
//!
//! All libmpv calls happen on one dedicated thread. The UI never blocks on
//! media, and shutting down the player is deterministic.

use std::ffi::CString;
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use libloading::Library;

use super::errors::AppError;

pub type FrameCallback = Box<dyn Fn(VideoFrame) + Send + 'static>;
pub type EventCallback = Box<dyn Fn(PlayerEvent) + Send + 'static>;

/// One decoded preview frame, top-down RGB24.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub position_secs: f64,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Loaded { duration_secs: f64 },
    Time { position_secs: f64, paused: bool },
    Ended,
    MediaError(String),
    MissingLibrary,
}

#[derive(Debug)]
enum Command {
    Load(PathBuf),
    Play,
    Pause,
    TogglePlay,
    Seek(f64),
    SetBounds { start: f64, end: f64 },
    SetMute(bool),
    SetViewSize(u32, u32),
    Shutdown,
}

pub struct Player {
    tx: Sender<Command>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Player {
    pub fn spawn(on_frame: FrameCallback, on_event: EventCallback) -> Result<Self, AppError> {
        let library = load_library()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new()
            .name("quicktrim-mpv".into())
            .spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_player(library, rx, on_frame, on_event);
                }));
            })
            .map_err(|err| AppError::Internal {
                detail: format!("Could not start the playback thread: {err}."),
            })?;

        Ok(Self {
            tx,
            join: Some(join),
        })
    }

    pub fn load(&self, path: &Path) {
        self.send(Command::Load(path.to_path_buf()));
    }

    pub fn play(&self) {
        self.send(Command::Play);
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn toggle_play(&self) {
        self.send(Command::TogglePlay);
    }

    pub fn seek(&self, position_secs: f64) {
        self.send(Command::Seek(position_secs));
    }

    pub fn set_bounds(&self, start_secs: f64, end_secs: f64) {
        self.send(Command::SetBounds {
            start: start_secs,
            end: end_secs,
        });
    }

    pub fn set_mute(&self, muted: bool) {
        self.send(Command::SetMute(muted));
    }

    pub fn set_view_size(&self, width: u32, height: u32) {
        self.send(Command::SetViewSize(width, height));
    }

    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn load_library() -> Result<Library, AppError> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(explicit) = std::env::var_os("QUICKTRIM_LIBMPV") {
        candidates.push(PathBuf::from(explicit));
    }

    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        for name in platform_library_names() {
            candidates.push(dir.join(name));
        }
    }
    for name in platform_library_names() {
        candidates.push(PathBuf::from(name));
    }

    let mut last_error = String::new();
    for candidate in &candidates {
        // SAFETY: loading a shared library is inherently unsafe; we only load
        // a known filename from the app directory or the system search path.
        match unsafe { Library::new(candidate) } {
            Ok(library) => return Ok(library),
            Err(err) => last_error = err.to_string(),
        }
    }

    let _ = &last_error;
    Err(AppError::ToolMissing {
        tool: "libmpv".to_string(),
    })
}

fn platform_library_names() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["libmpv-2.dll", "libmpv.dll", "mpv-2.dll"]
    }
    #[cfg(target_os = "macos")]
    {
        &["libmpv.2.dylib", "libmpv.dylib"]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &["libmpv.so.2", "libmpv.so"]
    }
}

// ---------------------------------------------------------------------------
// libmpv FFI (dynamic)
// ---------------------------------------------------------------------------

type MpvHandle = *mut c_void;
type RenderHandle = *mut c_void;

#[repr(C)]
#[allow(dead_code)]
struct MpvEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvRenderParam {
    type_: c_int,
    data: *mut c_void,
}

type FnCreate = unsafe extern "C" fn() -> MpvHandle;
type FnInit = unsafe extern "C" fn(MpvHandle) -> c_int;
type FnSetOption = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnCommand = unsafe extern "C" fn(MpvHandle, *const *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(MpvHandle, *const c_char, *const c_char) -> c_int;
type FnGetProperty = unsafe extern "C" fn(MpvHandle, *const c_char, c_int, *mut c_void) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(MpvHandle, c_double) -> *mut MpvEvent;
type FnTerminate = unsafe extern "C" fn(MpvHandle);
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnRenderCreate =
    unsafe extern "C" fn(*mut RenderHandle, MpvHandle, *mut MpvRenderParam) -> c_int;
type FnRenderRender = unsafe extern "C" fn(RenderHandle, *mut MpvRenderParam) -> c_int;
type FnRenderUpdate = unsafe extern "C" fn(RenderHandle) -> u64;
type FnRenderFree = unsafe extern "C" fn(RenderHandle);

const MPV_FORMAT_FLAG: c_int = 3;
const MPV_FORMAT_DOUBLE: c_int = 5;

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_FILE_LOADED: c_int = 8;

const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
const MPV_RENDER_PARAM_SW_SIZE: c_int = 17;
const MPV_RENDER_PARAM_SW_FORMAT: c_int = 18;
const MPV_RENDER_PARAM_SW_STRIDE: c_int = 19;
const MPV_RENDER_PARAM_SW_POINTER: c_int = 20;

const MPV_RENDER_UPDATE_FRAME: u64 = 1;

struct Api {
    _library: Library,
    create: FnCreate,
    initialize: FnInit,
    set_option_string: FnSetOption,
    command: FnCommand,
    set_property_string: FnSetProperty,
    get_property: FnGetProperty,
    wait_event: FnWaitEvent,
    terminate_destroy: FnTerminate,
    error_string: FnErrorString,
    render_create: FnRenderCreate,
    render: FnRenderRender,
    render_update: FnRenderUpdate,
    render_free: FnRenderFree,
}

impl Api {
    unsafe fn load(library: Library) -> Result<Self, AppError> {
        unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> Result<T, AppError> {
            library
                .get::<T>(name.as_bytes())
                .map(|symbol| *symbol)
                .map_err(|_| AppError::Internal {
                    detail: format!("The bundled libmpv is missing “{name}”."),
                })
        }

        Ok(Self {
            create: symbol(&library, "mpv_create")?,
            initialize: symbol(&library, "mpv_initialize")?,
            set_option_string: symbol(&library, "mpv_set_option_string")?,
            command: symbol(&library, "mpv_command")?,
            set_property_string: symbol(&library, "mpv_set_property_string")?,
            get_property: symbol(&library, "mpv_get_property")?,
            wait_event: symbol(&library, "mpv_wait_event")?,
            terminate_destroy: symbol(&library, "mpv_terminate_destroy")?,
            error_string: symbol(&library, "mpv_error_string")?,
            render_create: symbol(&library, "mpv_render_context_create")?,
            render: symbol(&library, "mpv_render_context_render")?,
            render_update: symbol(&library, "mpv_render_context_update")?,
            render_free: symbol(&library, "mpv_render_context_free")?,
            _library: library,
        })
    }

    fn error_text(&self, code: c_int) -> String {
        if code >= 0 {
            return "ok".to_string();
        }
        // SAFETY: mpv_error_string returns a static string.
        unsafe {
            let ptr = (self.error_string)(code);
            if ptr.is_null() {
                format!("mpv error {code}")
            } else {
                std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Player thread
// ---------------------------------------------------------------------------

struct Session {
    api: Api,
    mpv: MpvHandle,
    render: Option<RenderHandle>,
    view: (u32, u32),
    aspect: f64,
    bounds: (f64, f64),
    playing: bool,
    paused_last: bool,
    loaded: bool,
    dirty: bool,
    last_time_emit: Instant,
    frame_buffer: Vec<u8>,
    on_frame: FrameCallback,
    on_event: EventCallback,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(handle) = self.render.take() {
            // SAFETY: freed exactly once, before the mpv handle is destroyed.
            unsafe { (self.api.render_free)(handle) };
        }
        // SAFETY: terminates the mpv instance and joins its internal threads.
        unsafe { (self.api.terminate_destroy)(self.mpv) };
    }
}

fn run_player(
    library: Library,
    rx: Receiver<Command>,
    on_frame: FrameCallback,
    on_event: EventCallback,
) {
    let api = match unsafe { Api::load(library) } {
        Ok(api) => api,
        Err(_) => {
            on_event(PlayerEvent::MissingLibrary);
            return;
        }
    };

    let mut session = match Session::create(api, on_frame, on_event) {
        Ok(session) => session,
        Err(err) => {
            eprintln!("[quicktrim] playback unavailable: {err:?}");
            return;
        }
    };

    session.run(rx);
}

impl Session {
    fn create(api: Api, on_frame: FrameCallback, on_event: EventCallback) -> Result<Self, AppError> {
        // SAFETY: standard libmpv lifecycle; every pointer stays on this thread.
        unsafe {
            let mpv = (api.create)();
            if mpv.is_null() {
                return Err(AppError::Internal {
                    detail: "libmpv could not create a player.".to_string(),
                });
            }

            let options: &[(&str, &str)] = &[
                ("vo", "libmpv"),
                ("idle", "yes"),
                ("pause", "yes"),
                ("keep-open", "yes"),
                ("hr-seek", "yes"),
                ("input-default-bindings", "no"),
                ("input-vo-keyboard", "no"),
                ("osc", "no"),
                ("osd-level", "0"),
                ("terminal", "no"),
                ("config", "no"),
                ("ytdl", "no"),
                ("audio-display", "no"),
                ("save-position-on-quit", "no"),
                ("force-window", "no"),
                ("hwdec", "auto-safe"),
                ("audio-fallback-to-null", "yes"),
                ("ao", default_audio_output()),
            ];

            for (name, value) in options {
                let name = CString::new(*name).unwrap();
                let value = CString::new(*value).unwrap();
                let rc = (api.set_option_string)(mpv, name.as_ptr(), value.as_ptr());
                if rc < 0 {
                    eprintln!("[quicktrim] mpv option {name:?}={value:?} rejected: {rc}");
                }
            }

            let rc = (api.initialize)(mpv);
            if rc < 0 {
                let message = api.error_text(rc);
                (api.terminate_destroy)(mpv);
                return Err(AppError::Internal {
                    detail: format!("libmpv could not start: {message}."),
                });
            }

            let api_type = CString::new("sw").unwrap();
            let mut params = [
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_API_TYPE,
                    data: api_type.as_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    type_: 0,
                    data: std::ptr::null_mut(),
                },
            ];
            let mut handle: RenderHandle = std::ptr::null_mut();
            let rc = (api.render_create)(&mut handle, mpv, params.as_mut_ptr());
            if rc < 0 {
                let message = api.error_text(rc);
                (api.terminate_destroy)(mpv);
                return Err(AppError::Internal {
                    detail: format!("The video surface could not be created: {message}."),
                });
            }

            Ok(Self {
                api,
                mpv,
                render: Some(handle),
                view: (960, 540),
                aspect: 16.0 / 9.0,
                bounds: (0.0, f64::MAX),
                playing: false,
                paused_last: true,
                loaded: false,
                dirty: true,
                last_time_emit: Instant::now() - Duration::from_secs(1),
                frame_buffer: Vec::new(),
                on_frame,
                on_event,
            })
        }
    }

    fn run(&mut self, rx: Receiver<Command>) {
        loop {
            let mut shutdown = false;
            loop {
                match rx.try_recv() {
                    Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => {
                        shutdown = true;
                        break;
                    }
                    Ok(command) => self.handle(command),
                    Err(TryRecvError::Empty) => break,
                }
            }
            if shutdown {
                break;
            }

            self.pump_events();
            self.render_if_needed();
            self.enforce_bounds();
            self.emit_time_if_due();

            // Polling keeps the thread simple and predictable; 4 ms is well
            // below a frame interval and costs almost nothing while idle.
            std::thread::sleep(Duration::from_millis(4));
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Load(path) => {
                self.loaded = false;
                let path = path.to_string_lossy().into_owned();
                let _ = self.command(&["loadfile", &path, "replace"]);
                let _ = self.set_flag("pause", true);
                self.playing = false;
            }
            Command::Play => self.play_from_bounds(),
            Command::Pause => {
                let _ = self.set_flag("pause", true);
                self.playing = false;
            }
            Command::TogglePlay => {
                if self.get_flag("pause").unwrap_or(true) {
                    self.play_from_bounds();
                } else {
                    let _ = self.set_flag("pause", true);
                    self.playing = false;
                }
            }
            Command::Seek(position) => self.seek_to(position),
            Command::SetBounds { start, end } => {
                self.bounds = (start.max(0.0), end.max(start));
            }
            Command::SetMute(muted) => {
                let _ = self.set_flag("mute", muted);
            }
            Command::SetViewSize(width, height) => {
                let view = (width.max(64), height.max(64));
                if view != self.view {
                    self.view = view;
                    self.dirty = true;
                }
            }
            Command::Shutdown => {}
        }
    }

    fn play_from_bounds(&mut self) {
        let position = self.position().unwrap_or(0.0);
        let (start, end) = self.bounds;
        if position < start - 0.005 || position >= end - 0.005 {
            self.seek_to(start);
        }
        let _ = self.set_flag("pause", false);
        self.playing = true;
        self.dirty = true;
    }

    fn seek_to(&mut self, position: f64) {
        let duration = self.duration().unwrap_or(f64::MAX);
        let target = position.clamp(0.0, duration.max(0.0));
        let _ = self.command(&["seek", &format!("{target:.6}"), "absolute+exact"]);
        self.dirty = true;
    }

    fn pump_events(&mut self) {
        loop {
            // SAFETY: the event pointer is owned by mpv and valid until the
            // next call to mpv_wait_event on this handle.
            let event = unsafe { (self.api.wait_event)(self.mpv, 0.0) };
            if event.is_null() {
                return;
            }
            let event = unsafe { &*event };
            match event.event_id {
                MPV_EVENT_NONE => return,
                MPV_EVENT_FILE_LOADED => self.on_loaded(),
                MPV_EVENT_END_FILE => {
                    let (reason, error) = if event.data.is_null() {
                        (0, 0)
                    } else {
                        let data = event.data as *const c_int;
                        unsafe { (*data, *data.add(1)) }
                    };
                    self.playing = false;
                    if reason == 4 && error < 0 {
                        (self.on_event)(PlayerEvent::MediaError(self.api.error_text(error)));
                    } else {
                        (self.on_event)(PlayerEvent::Ended);
                    }
                }
                MPV_EVENT_SHUTDOWN => return,
                _ => {}
            }
        }
    }

    fn on_loaded(&mut self) {
        self.loaded = true;
        let duration = self.duration().unwrap_or(0.0);
        if let Some(aspect) = self.get_double("video-params/aspect").filter(|a| {
            a.is_finite() && *a > 0.1 && *a < 20.0
        }) {
            self.aspect = aspect;
        }

        let (start, end) = self.bounds;
        let end = if end > duration && duration > 0.0 {
            duration
        } else {
            end
        };
        self.bounds = (start.max(0.0), end.max(start));
        self.seek_to(start.max(0.0));

        (self.on_event)(PlayerEvent::Loaded {
            duration_secs: duration,
        });
    }

    fn enforce_bounds(&mut self) {
        if !self.playing {
            return;
        }
        let (start, end) = self.bounds;
        let Some(position) = self.position() else {
            return;
        };
        if end.is_finite() && position >= end - 0.004 {
            let _ = self.command(&["seek", &format!("{end:.6}"), "absolute+exact"]);
            let _ = self.set_flag("pause", true);
            self.playing = false;
            self.dirty = true;
        } else if position < start - 0.25 {
            // A seek from mpv itself (e.g. keyframe drift) landed outside.
            self.seek_to(start);
        }
    }

    fn render_if_needed(&mut self) {
        if !self.loaded {
            return;
        }
        let Some(handle) = self.render else {
            return;
        };

        let flags = unsafe { (self.api.render_update)(handle) };
        if flags & MPV_RENDER_UPDATE_FRAME == 0 && !self.dirty {
            return;
        }
        self.dirty = false;

        let (width, height) = self.render_size();
        if width < 2 || height < 2 {
            return;
        }

        self.frame_buffer.resize(width as usize * height as usize * 3, 0);
        let mut size = [width as c_int, height as c_int];
        let mut stride = width as usize * 3;
        let format = b"rgb24\0";
        let mut params = [
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_SW_FORMAT,
                data: format.as_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_SW_STRIDE,
                data: &mut stride as *mut usize as *mut c_void,
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_SW_POINTER,
                data: self.frame_buffer.as_mut_ptr() as *mut c_void,
            },
            MpvRenderParam {
                type_: 0,
                data: std::ptr::null_mut(),
            },
        ];

        let rc = unsafe { (self.api.render)(handle, params.as_mut_ptr()) };
        if rc < 0 {
            return;
        }

        let position = self.position().unwrap_or(0.0);
        (self.on_frame)(VideoFrame {
            width,
            height,
            rgb: self.frame_buffer.clone(),
            position_secs: position,
        });
    }

    /// The preview is rendered at the largest size that fits the pane while
    /// preserving aspect; Slint then letterboxes the image.
    fn render_size(&self) -> (u32, u32) {
        let (view_w, view_h) = self.view;
        let aspect = self.aspect.clamp(0.1, 20.0);
        let mut width = view_w as f64;
        let mut height = width / aspect;
        if height > view_h as f64 {
            height = view_h as f64;
            width = height * aspect;
        }
        let width = (width.round().max(2.0) as u32) & !1;
        let height = (height.round().max(2.0) as u32) & !1;
        (width.min(1920), height.min(1920))
    }

    fn emit_time_if_due(&mut self) {
        let paused = self.get_flag("pause").unwrap_or(!self.playing);
        let due = self.last_time_emit.elapsed() >= Duration::from_millis(100);
        if !due && paused == self.paused_last {
            return;
        }
        self.last_time_emit = Instant::now();
        self.paused_last = paused;
        if let Some(position) = self.position() {
            (self.on_event)(PlayerEvent::Time {
                position_secs: position,
                paused,
            });
        }
    }

    fn duration(&self) -> Option<f64> {
        self.get_double("duration").filter(|d| d.is_finite() && *d > 0.0)
    }

    fn position(&self) -> Option<f64> {
        self.get_double("time-pos").filter(|p| p.is_finite() && *p >= 0.0)
    }

    fn get_double(&self, name: &str) -> Option<f64> {
        let name = CString::new(name).ok()?;
        let mut value: f64 = 0.0;
        let rc = unsafe {
            (self.api.get_property)(
                self.mpv,
                name.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut value as *mut f64 as *mut c_void,
            )
        };
        (rc >= 0).then_some(value)
    }

    fn get_flag(&self, name: &str) -> Option<bool> {
        let name = CString::new(name).ok()?;
        let mut value: c_int = 0;
        let rc = unsafe {
            (self.api.get_property)(
                self.mpv,
                name.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut value as *mut c_int as *mut c_void,
            )
        };
        (rc >= 0).then_some(value != 0)
    }

    fn set_flag(&self, name: &str, enabled: bool) -> Result<(), AppError> {
        let name = CString::new(name).unwrap();
        let value = CString::new(if enabled { "yes" } else { "no" }).unwrap();
        let rc =
            unsafe { (self.api.set_property_string)(self.mpv, name.as_ptr(), value.as_ptr()) };
        if rc < 0 {
            Err(AppError::Internal {
                detail: format!("libmpv rejected “{name:?}”: {}", self.api.error_text(rc)),
            })
        } else {
            Ok(())
        }
    }

    fn command(&self, args: &[&str]) -> Result<(), AppError> {
        let owned: Vec<CString> = args
            .iter()
            .map(|arg| CString::new(*arg).unwrap_or_default())
            .collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());

        let rc = unsafe { (self.api.command)(self.mpv, pointers.as_ptr()) };
        if rc < 0 {
            Err(AppError::Internal {
                detail: format!("libmpv command failed: {}.", self.api.error_text(rc)),
            })
        } else {
            Ok(())
        }
    }
}

fn default_audio_output() -> &'static str {
    #[cfg(windows)]
    {
        "wasapi,null"
    }
    #[cfg(target_os = "macos")]
    {
        "coreaudio,null"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "pulse,alsa,null"
    }
}
