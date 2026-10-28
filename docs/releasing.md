# Releasing QuickTrim

A release is one tag, one workflow run and one portable ZIP. The goal is that a
user on a clean Windows machine can download, extract, double-click and trim a
video without installing anything.

## Versioning

QuickTrim is pre-1.0. The version in `Cargo.toml` is the version that ships, and
the git tag must be `v` + that exact string:

```
Cargo.toml   version = "0.1.0"
git tag      v0.1.0
```

`release.yml` refuses to build if the tag and `Cargo.toml` disagree, so a typo
cannot produce an artifact labelled with the wrong number.

While the project is pre-1.0, a minor bump (0.1 → 0.2) may change behaviour; a
patch bump (0.1.0 → 0.1.1) is a fix. Once 1.0 ships, ordinary semver applies.

## Checklist

1. **Record the change.** Move the `Unreleased` section of
   [`CHANGELOG.md`](../CHANGELOG.md) into a dated `## [x.y.z]` section.
2. **Bump the version** in `Cargo.toml`. Nothing else hard-codes it.
3. **Run the checks** on a machine with the helpers in `target\debug\`:

   ```powershell
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo build --release
   ```

4. **Smoke test the release build** (see below). The integration suite covers
   the engine; this covers the window, the dialogs and the files on disk.
5. **Commit and tag**:

   ```bash
   git commit -am "Release 0.1.0"
   git tag -a v0.1.0 -m "QuickTrim 0.1.0"
   git push origin main
   git push origin v0.1.0
   ```

6. **Check the draft release.** The workflow opens it as a draft with the ZIP
   and checksum attached and generated notes. Add a short summary for the
   version, confirm the asset sizes and hashes, then publish.

## What the workflow does

[`.github/workflows/release.yml`](../.github/workflows/release.yml), on a `v*`
tag:

1. Builds `quicktrim.exe` with `cargo build --release --locked` on
   `windows-latest`.
2. Downloads the exact FFmpeg and libmpv builds below and copies the helpers
   into `target\release\`.
3. Runs [`scripts/package.ps1`](../scripts/package.ps1), which assembles the
   folder, writes `SHA256SUMS.txt` and zips it.
4. Attaches the ZIP and `.zip.sha256` to a **draft** GitHub release.

The downloads are pinned deliberately. FFmpeg and mpv are shipped unmodified,
so the corresponding source is the upstream release named here and in
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).

| Component | Pinned build | Source |
| --- | --- | --- |
| FFmpeg 9.0.2 | gyan.dev `essentials_build` | <https://www.gyan.dev/ffmpeg/builds/> |
| libmpv | `mpv-dev-x86_64-20261018-git-01e2533f31.7z` | <https://sourceforge.net/projects/mpv-player-windows/files/libmpv/> |

Updating either one is a code change: bump the URL in the workflow, re-run the
smoke test, and update the version in `THIRD_PARTY_NOTICES.md`,
`docs/measurements.md` and this table.

## Building the same artifact locally

```powershell
cargo build --release

# helpers beside the app
Copy-Item target\debug\ffmpeg.exe,target\debug\ffprobe.exe,`
          target\debug\libmpv-2.dll target\release\

powershell -ExecutionPolicy Bypass -File scripts/package.ps1
```

Output:

```
dist\quicktrim-0.1.0-win-x64.zip          the download
dist\quicktrim-0.1.0-win-x64.zip.sha256   hash of the ZIP
dist\quicktrim-0.1.0-win-x64\             the unzipped folder
```

The folder contains the app, the three helpers, `VERSION.txt`, `README.txt`,
`CHANGELOG.md`, `LICENSE`, `LICENSES/GPL-2.0.txt`, `THIRD_PARTY_NOTICES.md`,
`docs/third-party-crates.md` and a `SHA256SUMS.txt` with one hash per file.

Users verify with:

```powershell
Get-FileHash .\quicktrim-0.1.0-win-x64.zip -Algorithm SHA256
Get-Content .\quicktrim-0.1.0-win-x64.zip.sha256
```

## Smoke test before publishing

On a clean Windows 11 machine (no Rust, no FFmpeg installed):

1. Extract the ZIP somewhere with spaces in the path.
2. Double-click `quicktrim.exe` with a video passed on the command line.
3. Play, pause, seek (`Space`, arrows), set `I` and `O`.
4. Save with `Ctrl+S`; confirm the notice names the file.
5. Probe the result and confirm the duration is within 250 ms of the selection.
6. Hash the source before and after; it must be identical.
7. Save again without changing the name; confirm `(2)` is created and nothing
   is overwritten.
8. Start a long export and press `Esc`; confirm "Cancel export" reports, no
   `*.qt-part-*` file remains, and the source is still intact.
9. Unplug the network (the app must not care), lock and unlock the screen
   (the preview must repaint, see ADR 0003).

## Platforms

Only Windows x64 artifacts are published. There is no macOS or Linux download,
because neither platform has been run end to end. Adding one means adding a job
that builds, packages and tests that platform — not just a job that compiles.

When macOS comes up, the bundle will need a `.app` layout, the `.dylib` beside
the executable, and a decision about signing and notarisation. Linux will need
a tarball or AppImage plus an `rpath`/`LD_LIBRARY_PATH` story for libmpv. Both
are Phase 2 in the roadmap, and both should be written down in an ADR before
the first build is published.

## If a release is wrong

Do not move the tag; someone may already have downloaded it. Fix forward with a
patch release (`0.1.1`), note the problem in `CHANGELOG.md`, and edit the broken
release notes to point at the new one. Deleting a published release is reserved
for a security or licensing problem with the attached binaries.
