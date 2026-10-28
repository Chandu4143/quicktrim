# 0004 — Packaging and licensing: ship the helpers, accept GPLv3

* Status: accepted
* Date: 2026-10-27
* Phase: 0

## Context

The promise is "download it, double-click it, trim a video". That rules out
asking the user to install FFmpeg, mpv, a C++ runtime, or a package manager.
It also means every third-party licence has to be one we can live with when
the three large binaries are copied into the same folder as the app.

The pieces that must ship:

| Piece | What it is | Licence |
| --- | --- | --- |
| `quicktrim.exe` | our Rust + Slint binary | GPL-3.0-or-later (our choice) |
| Slint 1.18.1 | statically linked UI toolkit | GPL-3.0-only *or* commercial |
| `ffmpeg.exe` | probe/export engine, invoked as a child process | GPL (gyan.dev essentials build; includes libx264) |
| `ffprobe.exe` | metadata probe, invoked as a child process | GPL (same build) |
| `libmpv-2.dll` | playback engine, loaded with `libloading` | GPLv2-or-later (mpv v0.41.0-1205) |

## Decision

### Licence: GPL-3.0-or-later for our source, GPLv3 for the combined binary

Slint's open-source option is GPLv3, FFmpeg's common builds are GPL, and mpv is
GPLv2+. A permissively licensed build would need a paid Slint licence plus
LGPL builds of FFmpeg and mpv, which is a different project. The honest choice
is to be GPL and say so:

* the repository is offered under **GPL-3.0-or-later** (`LICENSE`,
  `Cargo.toml`);
* the shipped binary is conveyed under **GPLv3**, because Slint is used under
  its GPL-3.0-only option and GPL-3.0-only code cannot be relicensed as "or
  later". Downstream users may still take our own source under GPLv3;
* `THIRD_PARTY_NOTICES.md` lists every bundled component, its licence and where
  its corresponding source lives; `docs/third-party-crates.md` is the generated
  inventory of all 227 crates in the Cargo build graph;
* FFmpeg and mpv are **shipped unmodified**, exactly as published upstream, so
  their corresponding source is the upstream release for the exact version in
  the notices. Nothing in QuickTrim patches or relinks them.

Invoking FFmpeg as a child process is not "linking" in the copyright sense, but
it does not matter here: QuickTrim is GPL anyway, and shipping an unmodified
GPL FFmpeg only requires that its source be available, which it is.

### Layout: a folder, not an installer

```
dist\quicktrim\
  quicktrim.exe
  ffmpeg.exe
  ffprobe.exe
  libmpv-2.dll
  LICENSE
  THIRD_PARTY_NOTICES.md
```

No installer, no registry writes, no PATH changes, no services. Delete the
folder and it is gone. The executable looks for its helpers in a fixed order:

| Helper | Order |
| --- | --- |
| `ffmpeg` / `ffprobe` | `QUICKTRIM_FFMPEG` / `QUICKTRIM_FFPROBE` → executable directory → `PATH` |
| libmpv | `QUICKTRIM_LIBMPV` → executable directory (`libmpv-2.dll`, `libmpv.2.dylib`, `libmpv.so.2`) → system names |

The environment overrides exist for development (the integration tests point
them at `target/debug`) and for distributions that prefer system libraries.
They are also the escape hatch if a user already has a working FFmpeg.

### Size: accepted for Phase 0, unacceptable for release

Measured on the Windows 11 reference machine (method in
[`docs/measurements.md`](../measurements.md)):

| File | Size |
| --- | --- |
| `quicktrim.exe` (release, LTO, stripped) | 8.7 MB |
| `ffmpeg.exe` | 100.5 MB |
| `ffprobe.exe` | 100.3 MB |
| `libmpv-2.dll` | 115.4 MB |
| **Total** | **~325 MB** |

For comparison the debug executable is 69.4 MB (no LTO, no stripping, but
`debug = 0`), which shows how much the release profile matters for the part we
own. It does not help with the 316 MB that is not ours.

This is the correct trade-off *now*: it proves playback and export with
official, well-tested builds and zero build-system work. It is the wrong
trade-off for a download.

### The shrink plan (Phase 3)

1. **Shared FFmpeg.** Build with `--enable-shared`: `ffmpeg.exe` becomes a few
   hundred KB and the codec libraries (avcodec, avformat, avutil, swscale,
   swresample) are shared with `ffprobe`. The two 100 MB executables collapse
   into roughly one copy of the libraries.
2. **Trim the FFmpeg feature set.** Drop encoders/muxers/demuxers the app can
   never use, drop `ffplay`/`ffserver` artifacts, drop static libx264 in favour
   of whatever the export path actually needs.
3. **Trim libmpv.** Build without Lua/JS scripting, without ytdl, without
   every video output except `libmpv`, and without the optional ffmpeg
   components we do not call.
4. **No UPX.** It would shrink the download further but trips antivirus
   heuristics on unsigned binaries, and it breaks signing.

Target: under 120 MB for the whole folder.

## Consequences

* Users get one folder with no installation step and no network access. That
  is the product promise, and it survives the size problem.
* The project cannot be relicensed permissively without first replacing Slint,
  FFmpeg and mpv. That is written down here so nobody discovers it later.
* Antivirus false positives are a real risk for any unsigned, freshly built
  Windows executable that spawns FFmpeg. Phase 3 needs code signing; until
  then the README says the binaries are unsigned.
* macOS and Linux packaging (`.app` bundle, `AppImage`/tarball, `rpath` and
  `LD_LIBRARY_PATH` for libmpv) is not designed yet, because neither platform
  has been tested. Deciding it now would be fiction.
