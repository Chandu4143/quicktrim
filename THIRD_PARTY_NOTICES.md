# Third-party notices

QuickTrim is free software under the **GNU General Public License, version 3**
(`LICENSE`). It is built on other free software, and this file records what is
included, under which licence, and where its source can be obtained.

The source for QuickTrim itself is this repository, at the version recorded in
`Cargo.toml`. Every third-party binary listed below is shipped **unmodified**,
exactly as published by its upstream project.

## What a QuickTrim folder contains

| File | Component | Version | Licence | Role |
| --- | --- | --- | --- | --- |
| `quicktrim.exe` | QuickTrim | 0.1.0 | GPL-3.0-or-later (our source); conveyed as GPLv3 | the application |
| | Slint (statically linked) | 1.18.1 | GPL-3.0-only, used under that option | UI toolkit |
| | 227 Rust crates | see `docs/third-party-crates.md` | MIT / Apache-2.0 / BSD / ISC / Zlib / … | support code |
| `ffmpeg.exe` | FFmpeg | 9.0.2, gyan.dev "essentials" build | GPLv3 | probing and exporting |
| `ffprobe.exe` | FFprobe | 9.0.2, same build | GPLv3 | metadata probe |
| `libmpv-2.dll` | libmpv (mpv) | v0.41.0-1205-g01e2533f3 | GPLv2-or-later | playback |

On macOS the library is `libmpv.2.dylib`, on Linux `libmpv.so.2`. The
application resolves it at runtime (see ADR 0004); none of these helpers is
statically linked into `quicktrim.exe`.

## Slint 1.18.1

Copyright © SixtyFPS GmbH and contributors.

Slint is available under a choice of licences; QuickTrim uses the **GNU General
Public License v3** option. Because that option is GPL-3.0-*only*, the combined
binary is conveyed under GPLv3, while QuickTrim's own source remains available
under GPL-3.0-or-later.

* Project: <https://slint.dev>
* Source: <https://github.com/slint-ui/slint> (tag `v1.18.1`)
* Licence text: `LICENSE` in this repository.

The full GPLv3 terms apply to the binary. Slint's other options (the
royalty-free and software licences) are not used and are not granted here.

## FFmpeg 9.0.2 (gyan.dev essentials build)

Copyright © 2000–2026 the FFmpeg developers.

This is a **GPLv3** build: it was compiled with `--enable-gpl --enable-version3`
and includes GPL components such as libx264 and libx265. QuickTrim executes it
as a child process; it is not linked into `quicktrim.exe`, but it is
distributed alongside it, so its licence and source availability apply.

* Project: <https://ffmpeg.org>
* Source: <https://ffmpeg.org/download.html> — take the `n9.0.2` release, or
* Build provider: <https://www.gyan.dev/ffmpeg/builds/> (the `essentials_build`
  for 9.0.2, which is what was measured in `docs/measurements.md`)

The build configuration is recorded in the binary itself
(`ffmpeg -version`). It enables, among others: libx264 (GPLv2+), libx265
(GPLv2), libvpx/libaom (BSD), libmp3lame (LGPL-2.0+), libopus/libvorbis
(BSD), libass (ISC), FreeType (FTL or GPLv2), HarfBuzz (MIT), fontconfig
(MIT-style), GnuTLS (LGPL-2.1+), zlib, bzip2, liblzma, and SDL2 (Zlib). Those
components are aggregated into a single GPLv3 binary; their individual notices
are part of the FFmpeg source distribution.

## libmpv v0.41.0-1205-g01e2533f3

Copyright © 2000–2026 the mpv/MPlayer/mplayer2 projects.

mpv is licensed under the **GNU General Public License v2 or later** (some
optional parts are LGPL-2.1+). This build is used as a shared library, loaded
with `dlopen`/`LoadLibrary` via the `libloading` crate.

* Project: <https://mpv.io>
* Source: <https://github.com/mpv-player/mpv> (commit `01e2533f3`)
* Build: the official Windows x86_64 `libmpv` build corresponding to
  `v0.41.0-1205-g01e2533f3`
* Licence text: `LICENSES/GPL-2.0.txt` (mpv's "or later" option is satisfied by
  GPLv3, which the combined work is conveyed under).

## Rust crates

`docs/third-party-crates.md` is generated from the Cargo dependency graph and
lists all 227 crates in the build graph, with their licence expressions. They
are used under MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, BSL-1.0, 0BSD, CC0,
Unlicense, Unicode-3.0, or the Slint GPLv3 option — all compatible with
conveying the result under GPLv3.

The full text of each permissive licence ships inside its crate in the Cargo
registry, and is therefore present in any build environment. To collect them
for a redistribution:

```powershell
cargo vendor vendor\          # copies every crate's sources and licence files
Get-ChildItem vendor -Recurse -Include LICENSE*,COPYING*,NOTICE* |
    Copy-Item -Destination LICENSES\ -Force
```

`tempfile` is a dev-dependency only and is not part of the shipped binary.

## Operating-system components

QuickTrim calls the Windows API, and on other platforms the native windowing
and dialog APIs, through Rust crates. Those interfaces are "System Libraries"
in the sense of GPLv3 §1 and are not distributed with QuickTrim. No Microsoft,
Apple or Linux distribution code is included in the bundle.

## What this means if you redistribute QuickTrim

1. Keep `LICENSE`, `THIRD_PARTY_NOTICES.md`, `LICENSES/`, and the unmodified
   `ffmpeg.exe`, `ffprobe.exe` and `libmpv-2.dll` together with
   `quicktrim.exe`.
2. Make the Corresponding Source available. For QuickTrim that is this source
   tree; for FFmpeg, mpv and the Rust crates it is the upstream releases named
   above. Shipping the exact upstream versions (as QuickTrim does) means
   pointing at the upstream release is sufficient; modifying them would not be.
3. Do not add restrictions on top of the GPL, and do not relicense the combined
   binary.
4. There is **no warranty**, to the extent permitted by law (GPLv3 §§15–16).

## Not included

No proprietary code, no code under a licence incompatible with the GPL, no
binary blobs other than the three upstream helpers listed above. QuickTrim
contains no networking, telemetry, or auto-update component.
