# 0001 — Playback engine: libmpv in software render mode

* Status: accepted
* Date: 2026-10-27
* Phase: 0

## Context

QuickTrim needs to show the video it is trimming. The preview is not decoration:
the user chooses IN and OUT points by looking at frames, so the preview has to be
frame-accurate, seek quickly, handle any codec the user throws at it, respect
rotation metadata, and stay in sync with sound.

The candidates were:

| Option | Why not |
| --- | --- |
| FFmpeg decoding to RGB in-process (`ffmpeg-next`) | We would reimplement seeking, A/V sync, VFR, rotation, hardware decode and audio output. That is a media player, not a trimmer. |
| Spawning `ffplay` / `mpv` in a child window | Cannot embed reliably; focus, z-order and fullscreen leak; two processes to manage for one preview; the trim UI would be laid over a foreign window. |
| mpv IPC (JSON over a pipe) driving an external mpv | Same window problem, plus a second executable and a socket protocol to supervise. |
| Qt Multimedia / GStreamer | Qt is a much heavier dependency and its playback backends differ per OS; GStreamer is awkward on Windows and macOS packaging. |
| Native OS players (Media Foundation, AVFoundation, GStreamer) | Three implementations, three sets of bugs, three sets of codec availability. Windows would not open half of what the FFmpeg exporter can export. |
| **libmpv** | One engine on all three platforms, the same codec coverage as the exporter, battle-tested seeking and A/V sync, and a stable render API that hands us pixels. |

## Decision

Use **libmpv** through `libloading` (no build-time dependency on mpv headers or
libraries), with `vo=libmpv` and the render API in **software** mode:

* one `mpv_handle` and one `mpv_render_context` live on a dedicated thread;
* mpv is configured with `idle=yes`, `pause=yes`, `keep-open=yes`, `hr-seek=yes`,
  `config=no`, `ytdl=no`, `osc=no`, `force-window=no`;
* frames are rendered into an `rgb24` buffer (render params 17–20) sized to the
  largest rectangle that fits a 1280×720 box at the source aspect — 1280×720 for
  16:9, 960×720 for 4:3 — then wrapped in a Slint `Image::from_rgb8` and
  letterboxed by the UI with `image-fit: contain`;
* the thread polls `mpv_wait_event` and pushes `Loaded`, `Time`, `Ended`,
  `MediaError` and frame callbacks into channels; **it never touches Slint**;
* the player is joined on shutdown, which terminates and destroys mpv.

The library is resolved at runtime from `QUICKTRIM_LIBMPV`, then the executable's
directory, then the usual system names (`libmpv-2.dll`, `libmpv.2.dylib`,
`libmpv.so.2`).

## Consequences

**Good**

* One playback implementation to test, on every OS.
* Playback accepts the same broad set of containers and codecs as the exporter.
* The render API gives raw frames, so the preview can be blended into any UI
  toolkit without a child window.
* No C++ compiler or mpv SDK is needed at build time — `libloading` is enough.

**Costs**

* The shipped `libmpv-2.dll` is 115 MB (prototype). It is the single largest
  piece of the bundle and the first thing to trim in Phase 3.
* Software rendering costs CPU: a 1280×720 RGB blit per frame. That is fine for
  a preview pane, and it avoids GPU/driver variance entirely.
* The FFI surface is hand-written, so it must be reviewed against `client.h`
  when libmpv is upgraded. The constants currently used were checked against
  mpv's headers.

## Alternatives worth revisiting

Hardware rendering (`MPV_RENDER_PARAM_OPENGL_FBO`/`SW` on a GPU surface) would
cut CPU at 4K, but it forces the UI toolkit onto GL and reintroduces driver
variance. Not worth it for a 720p preview.
