# 0003 — UI: Slint with the software renderer

* Status: accepted
* Date: 2026-10-27
* Phase: 0

## Context

The interface is deliberately small: one preview pane, a two-handle timeline,
a handful of buttons, a progress bar and a message bar. The hard requirements
are not visual, they are operational:

* **No webview.** QuickTrim exists to avoid the Electron/Tauri footprint.
* a raw RGB frame from libmpv has to appear in the window every 16 ms;
* the shell must be one small executable, with no runtime to install;
* keyboard, focus and accessibility must behave like a desktop app, not a page;
* it has to survive being run over RDP, in a VM, and on a laptop with a broken
  GPU driver — the preview is software-decoded anyway.

Candidates:

| Option | Why not |
| --- | --- |
| Electron / Tauri / Wails | A webview, a browser engine, or both. Defeats the point. |
| Qt (C++ or `qt-rs`) | Excellent, but a large C++ runtime, licensing friction and a heavy build. |
| GTK4 | Needs system GTK on Windows and macOS; packaging becomes the project. |
| egui / iced | Immediate-mode and young-retained-mode respectively: fine for tools, but text layout, focus rings and accessibility are weaker than what a declarative UI gives. |
| **Slint** | Declarative `.slint` UI, a small statically linked runtime, a real software renderer, accessibility support, and a GPLv3 option that matches the rest of the stack. |

## Decision

Use **Slint 1.18** with the **winit** backend and the **software** renderer:

```toml
slint = { version = "1.18", default-features = false, features = [
    "backend-winit", "renderer-software", "compat-1-2", "accessibility", "std",
] }
```

* The whole interface is one file, `ui/main-window.slint`, compiled by
  `slint-build` in `build.rs`.
* No GPU API is touched. Slint draws into a CPU buffer that the backend hands
  to the window (softbuffer), which also means QuickTrim behaves identically on
  Intel, AMD, NVIDIA, in a VM, and over remote desktop.
* A 16 ms `slint::Timer` on the UI thread drains the mpsc channels from the
  probe, player and export threads and is the *only* place that mutates window
  properties. Worker threads never hold a `MainWindow`.
* mpv frames are rendered into an RGB24 buffer inside a 1280×720 box, sized to
  the source aspect (1280×720 for 16:9, 960×720 for 4:3), wrapped in
  `Image::from_rgb8`, and letterboxed into the pane with `image-fit: contain`.
* Colours, type and spacing live in one `Palette` global; the look is a dark
  near-black surface with a single amber accent, so the video is the only
  saturated thing on screen.

### The white-window bug, and the 1 Hz repaint

Slint's software renderer keeps a partial-rendering cache: it remembers which
regions changed and repaints only those, on the assumption that the window
surface still contains the previous frame (`RepaintBufferType::ReusedBuffer`).
When the OS discards that surface — the window is hidden and shown again, a
remote session reconnects, the desktop compositor resets — Slint's cache is
stale but still considered valid, and the window stays blank until the user
resizes it.

This reproduced deterministically in testing: `ShowWindow(hwnd, SW_HIDE)` then
`SW_SHOW` left a completely white client area, while the app was still
responding and playing audio.

The backend does not expose a way to set `RepaintBufferType` from application
code, and forcing a full repaint every frame would waste CPU on a UI that is
static most of the time. Instead, `main-window.slint` carries a full-window
`Rectangle` whose colour alternates between `#0d0d0f` and `#0d0d0e` — one
step apart, invisible — once every 60 ticks (~1 s). Because that rectangle
covers the whole window, the dirty region becomes the whole window, and the
surface is rebuilt from scratch. The window heals itself within a second of
coming back, at a measured cost of **0.12 s CPU per 6 s idle (~2% of one
core)** and no visible flicker.

## Consequences

**Good**

* One small binary (8.7 MB release, stripped) with no webview and no GPU.
* The UI and the media engine are cleanly separated: `App` owns channels,
  `main-window.slint` owns pixels and input, and the export path can be tested
  headlessly.
* Slint's accessibility tree gives the buttons and sliders names and roles
  without hand-rolled platform code.
* Rendering a 1280×720 preview on the CPU is cheap at this size, and it never
  competes with FFmpeg for the GPU.

**Costs**

* ~2% of one core while idle, because of the self-heal repaint. A correct
  `RepaintBufferType` hook upstream would remove the need for it.
* Text and shapes are rasterised on the CPU; large windows on old hardware are
  slower than a GPU path. Acceptable for a 1080×780 utility window.
* No GPU effects. Deliberate: the design has no blurs, shadows or animated
  gradients that would need them.
* The workaround depends on the full-window rectangle always being the bottom
  layer of `main-window.slint`. Moving it, or making it smaller, re-opens the
  bug. The comment in the file says so.

## Alternatives worth revisiting

If Slint gains a public switch for full-buffer repainting (or exposes
`RepaintBufferType`), delete the tick layer and the `keep_surface_alive` call.
If the bundle has to shrink further, the renderer is not the place to look:
libmpv and FFmpeg are 97% of the download.
