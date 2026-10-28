# Measurements

Every number here was measured, not estimated. Where a figure depends on the
machine, the machine is described. Re-run the commands if you change anything
that could move them (FFmpeg build, Slint version, render code, export flags).

Measured 2026-10-27, Phase 0.

## Reference machine

| Item | Value |
| --- | --- |
| OS | Windows 11 Home Single Language 23H2, build 22631, x86_64 |
| CPU | 12th Gen Intel Core i5-1235U (10 cores / 12 threads) |
| GPU | Intel Iris Xe (unused — everything is CPU) |
| RAM | 16 GB |
| Display scale | 100%, application window 1080×780 client area |
| Rust | 1.99.1, `x86_64-pc-windows-gnu` |
| Slint | 1.18.1 (software renderer, winit backend) |
| FFmpeg / ffprobe | 9.0.2 essentials build (gyan.dev), static, GPL |
| libmpv | v0.41.0-1205-g01e2533f3 (Windows x86_64 build) |

## Build outputs

| File | Bytes | Size |
| --- | ---: | ---: |
| `quicktrim.exe` (release: LTO, `codegen-units=1`, stripped, `panic=abort`) | 9,112,064 | 8.7 MiB |
| `quicktrim.exe` (debug: `debug=0`, no LTO) | 72,742,840 | 69.4 MiB |
| `ffmpeg.exe` | 105,423,872 | 100.5 MiB |
| `ffprobe.exe` | 105,221,120 | 100.3 MiB |
| `libmpv-2.dll` | 121,043,968 | 115.4 MiB |
| **Bundle total** | **340,801,024** | **325.0 MiB** |

The packaged folder used for the smoke test (`dist/quicktrim/`) is 325.1 MiB:
the four files above plus `LICENSE`, `THIRD_PARTY_NOTICES.md` and the licence
inventory.

The release profile cut our own binary by 87% (69.4 → 8.7 MiB). It cannot
touch the other 316 MiB, which is why packaging (ADR 0004) treats size as a
Phase 3 problem rather than a compiler-flag problem.

`cargo build --release` took 4 min 37 s from a `target/` that already had the
debug dependencies built, with LTO and 1 codegen unit. Incremental debug builds
are 8–40 s.

## Startup

| What | Time |
| --- | --- |
| Process start → window created, video argument, release build, warm file cache | **0.35 s** |
| Window created → first preview frame | not isolated; a capture taken immediately after the window appeared already showed frame 0 |

`libmpv-2.dll` is loaded lazily on the playback thread, so the window appears
before the player is ready. The UI shows the empty state until the first
`Loaded` event.

## Runtime cost

Measured on the release build with `fixtures/short-gop.mp4` (20 s, 640×360)
loaded, using the process's `CPU` counter over a 6 s window:

| State | CPU per 6 s | Share of one core | Working set |
| --- | ---: | ---: | ---: |
| Paused, preview visible | 0.11 s | ~1.8% | 102.1 MiB |
| Playing (30 fps source, 1280×720 preview) | 0.14 s | ~2.3% | 102.1 MiB |

Most of that is the deliberate 1 Hz full-window repaint that heals the
software renderer after the OS discards the window surface (ADR 0003). Without
that repaint the window can come back blank after a hide/show or a
remote-session reconnect; the idle cost was measured with it in place, and the
tick is kept because a blank window is a worse bug than 2% of a core.

The preview is rendered into an RGB24 buffer inside a 1280×720 box, sized to
the source aspect; playing a 640×360 source costs about the same as playing a
1080p one, because both are 16:9 and both render at 1280×720.

## Probe

`ffprobe -print_format json -show_format -show_streams`, warm process start,
same file:

| Input | Time |
| --- | ---: |
| 20 s 640×360 H.264/AAC | 30–40 ms |
| 30 s 1080p H.264/AAC | 36–38 ms |

The first invocation after a cold start measured 267 ms. Opening a file
therefore spends most of its time in process creation, not in analysis.

## Export

The engine is driven with exactly the arguments `build_args` produces
(`-ss` before `-i`, `-t`, `-map 0:v:0 -map 0:a:0?`,
`-avoid_negative_ts make_zero -movflags +faststart -f mp4`):

| Job | Mode | Wall clock | Output |
| --- | --- | ---: | ---: |
| 5 s from a 20 s 640×360 clip, cuts on keyframes | stream copy | **0.04–0.05 s** (first run 0.27 s, cold) | 655 KB |
| 15 s from a 30 s 1080p clip at 4.35 s (inside a 10 s GOP) | re-encode, `libx264 -preset veryfast -crf 20` + AAC 192k | **2.26–2.27 s** (~6.6× realtime) | 12.7 MiB |

A stream copy is dominated by process start-up. A re-encode on this CPU runs
at roughly 6.6× realtime for 1080p30, so a 10-minute 1080p cut takes about
90 seconds; that is the number to quote to users, not a fixed duration.

## Accuracy on the reference fixtures

Measured with `ffprobe -show_entries format=duration:stream=start_time,duration`.

| Request | Mode | Container duration | Video stream | Audio stream |
| --- | --- | ---: | ---: | ---: |
| 10.000 → 12.000 s (2.000 s) | stream copy | 2.152 s | 2.067 s, starts at 0.085 s | 2.090 s, starts at 0.000 s |
| 4.000 → 9.000 s (5.000 s) | stream copy | 5.152 s | 5.067 s, starts at 0.085 s | 5.085 s |

Both are inside the 200 ms copy tolerance. The consistent +0.15 s is AAC
priming plus the encoder delay carried by the source, not a shifted video
start: the video stream itself is +0.067 s. This is the documented
stream-copy limitation (README) and the reason cuts that are not on keyframes
re-encode instead, where the result lands within 250 ms of the request across
the integration suite's 15 real-media tests.

## Reproducing

```powershell
cargo build --release
Copy-Item target\debug\ffmpeg.exe,target\debug\ffprobe.exe,target\debug\libmpv-2.dll target\release\

# sizes
Get-ChildItem target\release\quicktrim.exe,target\release\ffmpeg.exe,`
  target\release\ffprobe.exe,target\release\libmpv-2.dll |
  Select-Object Name,Length

# idle cost (load a video first)
$p = Get-Process quicktrim
$c1 = $p.CPU; Start-Sleep 6; $p.Refresh(); $p.CPU - $c1

# export timing: same arguments the app builds
Measure-Command {
  .\target\release\ffmpeg.exe -hide_banner -nostdin -loglevel error -nostats `
    -y -ss 4 -i fixtures\short-gop.mp4 -t 5 -map 0:v:0 -map "0:a:0?" `
    -c copy -avoid_negative_ts make_zero -movflags +faststart -f mp4 "$env:TEMP\cut.mp4"
}
```

## Not measured yet

* macOS and Linux: nothing on this page applies to them.
* Peak memory while encoding: the FFmpeg child is a separate process and its
  working set was not recorded.
* Long jobs (a 2-hour 4K source): no fixture exists for it yet.
* Power draw, thermals, and behaviour on battery.
