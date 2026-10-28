## What changed

<!-- One or two sentences. Link the issue if there is one. -->

## Why

<!-- The problem this solves. If it changes a design decision, say which ADR it touches. -->

## How it was verified

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo build --release`

<!--
Not every check can run everywhere. Say which ones did not, and why, instead of
leaving the box unticked. For engine changes, name the media you tested with.
-->

## Notes for the reviewer

<!--
- Does this keep the "one job" rule?
- Does it touch the safety rules (read-only source, temp file + rename,
  never overwrite, cancel kills the child)?
- Does a number in docs/measurements.md need re-measuring?
- Is CHANGELOG.md updated for anything a user would notice?
-->
