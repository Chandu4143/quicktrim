# Demo media

The files here are used by the top of the README.

| File | What it is |
| --- | --- |
| `logo.svg` | The project mark: two IN/OUT handles around a playhead. |
| `demo.gif` | Ten-second loop of the real app: mark the range, save, done. 1280 px, 10 fps. |
| `demo.mp4` | The full 26-second demo, 1920×1080, 30 fps, H.264, silent. |
| `demo-poster.png` | The 1080p still frame used as the video's poster. |
| `screenshot.png` | A single frame of the app with a range selected, no editing. |

## How it was made

`demo.mp4` is not a mockup. The footage was recorded from the built
`quicktrim.exe` on Windows 11: the window is captured with `gdigrab`, the mouse
and keyboard are driven with real Win32 input, and the interactions are genuine
(play, seek, set IN, drag OUT, save, wait for the export, read the notice).

The dark cards, captions and the 1920×1080 frame around the window were
composited over that capture. Nothing inside the window is animated or
substituted: the timeline values, progress bar and "SAVED" notice are the app's
own.

To re-record after a UI change:

```powershell
cargo build --release
# helpers (ffmpeg.exe, ffprobe.exe, libmpv-2.dll) next to the release binary
# then capture the window while driving it, and composite the cards
```

The capture and compositing scripts are not part of the repository; they are
production tools, not build steps. Recreating them is a few hundred lines of
Win32 input and Pillow drawing.

## Demo footage credit

The clip being trimmed is
["Ocean surface waves"](https://commons.wikimedia.org/wiki/Category:Videos_of_ocean_surface_waves)
from Wikimedia Commons, released under **CC0 1.0** (public domain dedication).
Three of the series were joined into one 35.7-second source clip for the demo.
CC0 requires no attribution; it is credited here anyway.
