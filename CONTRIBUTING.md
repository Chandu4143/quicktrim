# Contributing to QuickTrim

Thanks for taking a look. QuickTrim is deliberately small, and the fastest way
to get a change merged is to keep it that way.

## The two rules

1. **Keep it one job.** Open a video, pick a start and an end, save a copy.
   Features that turn it into an editor (a multi-track timeline, effects,
   project files, batch queues) are out of scope, however useful they sound.
2. **Prove the claim.** If it affects playback, export behaviour, accuracy or
   the safety rules, it needs a test or a measurement. A screenshot is not
   evidence that a file was written correctly.

## Before you start

For anything larger than a bug fix — a new dependency, a new platform, a change
to the export strategy, a UI restructure — open an issue or a draft pull request
first. The decision records in [`docs/adr/`](docs/adr/) explain why the current
design looks the way it does, and a change that contradicts one should say so
and (if accepted) update it.

Small fixes, typo corrections, better error messages and test additions can go
straight to a pull request.

## Setting up

You need Rust 1.85 or newer and the helpers beside the test binary.

```powershell
# Windows, GNU toolchain used by the reference machine
# (MSVC works too; nothing in the build depends on GNU)
# Put these in target\debug\ and target\release\:
#   ffmpeg.exe  ffprobe.exe  libmpv-2.dll
cargo test
cargo run --release -- "C:\path\to\clip.mp4"
```

The integration tests generate their own fixtures with FFmpeg under
`target/it-fixtures`, so the first run is slower than later ones. They never
touch the fixtures in `fixtures/`.

## Checks that must pass

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

CI runs the same four on Windows. If a check cannot run on your machine, say so
in the pull request rather than skipping it silently.

## Style

- The existing code favours small modules and explicit error types over clever
  abstractions. Match it.
- User-facing strings are sentences: what happened, and what to do next. Never
  surface a raw FFmpeg log or a stack trace in the interface.
- Comments explain *why*. The code already says what.
- Keep `unsafe` and FFI confined to the modules that need them, and document
  the invariant that makes the call sound.

## Commits and pull requests

- One logical change per commit; a clear subject line in the imperative mood.
- In the pull request, say what you changed, why, and what you ran. If the
  change moves a number in `docs/measurements.md`, re-measure and update it.
- Add an entry to [`CHANGELOG.md`](CHANGELOG.md) under `Unreleased` for anything
  a user would notice.

## Licence

By contributing you agree that your contribution is licensed under
**GPL-3.0-or-later**, the licence of the project. There is no CLA and no
copyright assignment. Do not paste code you cannot license this way.
