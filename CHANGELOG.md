# Changelog

All notable changes to QuickTrim are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it reaches 1.0.
Until then, minor versions may change behaviour.

## [Unreleased]

## [0.1.0] — 2026-10-27

The first public preview: one window, one job, Windows 11 x64.

### Added

- Embedded libmpv preview with seek, play/pause, mute and 60 fps frame delivery.
- Single-track timeline with draggable IN/OUT handles and a playhead.
- Frame-accurate export via FFmpeg, with stream copy when both cuts land on
  keyframes and `libx264 -crf 20` + AAC otherwise.
- Verification of every output before it is published; copies outside tolerance
  fall back to an encode.
- Atomic writes through a hidden temporary sibling, with no-overwrite naming
  (`clip_trimmed.mp4`, `clip_trimmed (2).mp4`, …).
- Cancellable export that kills and reaps the FFmpeg child and removes partial
  files.
- Drag and drop, native open/save dialogs, "show in folder", and a file argument
  on the command line.
- Keyboard shortcuts with visible button equivalents and screen-reader labels.
- Integration tests against real media: copy, re-encode, MOV/MKV/WebM inputs,
  portrait and silent video, source immutability, cancellation and temp-file
  cleanup.
- Release workflow that builds the Windows portable bundle from pinned FFmpeg
  and libmpv versions.

### Known gaps

- macOS and Linux have not been run; no builds are published for them.
- HDR is not tone mapped.
- The bundle is ~325 MB until the FFmpeg/libmpv slimming work lands.
- Drag and drop is not yet automated.

[Unreleased]: https://github.com/quicktrim/quicktrim/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/quicktrim/quicktrim/releases/tag/v0.1.0
