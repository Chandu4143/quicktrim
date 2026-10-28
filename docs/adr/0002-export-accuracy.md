# 0002 — Export: copy only when provably safe, otherwise re-encode

* Status: accepted
* Date: 2026-10-27
* Phase: 0

## Context

Trimming looks trivial — `ffmpeg -ss A -to B -i in.mp4 out.mp4` — until you
promise accuracy. Three things go wrong in the obvious implementation:

1. **Stream copy snaps to keyframes.** `-c copy` can only start on a keyframe.
   Ask for 12.4 s on a 10 s GOP and you silently get 10.0 s, or a clip with a
   frozen first second.
2. **Some codecs are legal but unusable in MP4.** VP9/Opus in MP4 exists, but
   Windows and macOS players routinely refuse it. A "successful" trim that will
   not open is a failure.
3. **Cut points and containers disagree.** AAC priming, negative timestamps and
   timescale rounding can shift the result by tens of milliseconds. Assuming the
   result is right is not the same as checking.

The user's mental model is simple: the clip starts where I put the handle and
ends where I put the other handle. Anything else must be reported, not shipped.

## Decision

**Plan first, then export, then prove it.**

### Planning (`plan_to`)

* Output is always `.mp4`; only the first video and first audio stream are mapped
  (`-map 0:v:0 -map 0:a:0?`).
* Refuse an output that equals the input; refuse a missing folder.
* Choose **stream copy** only if:
  * the video codec is `h264`, `hevc`, `av1` or `mpeg4` **and** the audio codec is
    `aac`, `mp3`, `ac3`, `eac3` or `alac` (or there is no audio), **and**
  * both cut points are within **45 ms** of a real keyframe, found with
    `ffprobe -skip_frame nokey -show_entries frame=pts_time` in a ±0.6 s window
    (the start of the file and the end of the file count as aligned).
* Otherwise choose **re-encode**: `libx264 -preset veryfast -crf 20
  -pix_fmt yuv420p`, AAC 192 kbit/s.

VP9 was deliberately removed from the copy list (ADR 0004 covers the wider
compatibility stance): a VP9-in-MP4 file is legal but poorly supported by the
players this tool targets, so a WebM source is transcoded.

### Execution (`run`)

* Write to a hidden sibling temp file, never to the destination.
* `-ss` before `-i` (input seek) plus `-t` for the selection.
* Parse `-progress pipe:1` for a monotonic fraction; drain stderr so a chatty
  failure cannot deadlock the child.
* On cancel: `child.kill()`, `child.wait()`, join both pipe threads, delete the
  temp file, return `Cancelled`. No orphan, no partial file.
* On completion: probe the temp file and compare its duration with the request.
  * copy tolerance: **200 ms**;
  * encode tolerance: `max(250 ms, 5% of the selection)`.
* If a **copy** fails validation or fails to run, delete it and retry once as an
  **encode**. Errors that a retry cannot fix (cancel, disk full, permissions,
  missing tool, vanished input) are returned immediately.
* Publish with `finalize`: re-check the destination does not exist, then rename.
  A rename is atomic on the same volume, so there is no window in which the user
  can see a half-written file.

### Why these numbers

* **45 ms at 30 fps is 1.35 frames.** Keyframe timestamps from ffprobe are exact;
  the tolerance only absorbs rounding in the container's timescale.
* **200 ms for copies** accommodates AAC priming (~80 ms start offset plus a few
  audio frames) while still catching a copy that grabbed the wrong span. On the
  reference fixture a 5.000 s copy probes at 5.152 s — inside tolerance, and the
  extra time is the audio tail, not shifted video.
* **250 ms / 5% for encodes** covers container rounding without ever accepting a
  clip that is visibly longer or shorter.

## Consequences

* The fast path is genuinely fast (a 5 s cut is a few hundred milliseconds) and
  lossless.
* The slow path always matches the handles, at the cost of a re-encode.
* Users never receive a silently wrong file: either it is within tolerance, or
  it is re-encoded, or they get a plain-language error.
* Cost: the keyframe probe adds one short ffprobe call per export with a copy
  candidate, and every export costs one probe afterwards.

## Not done yet

* No two-pass encoding, no hardware encoder selection, no per-file quality
  setting. Those are product decisions for later, not correctness ones.
* No audio-only or subtitle-preserving export. Both are visible limitations in
  the README rather than silent data loss.
