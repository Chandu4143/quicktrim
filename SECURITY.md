# Security policy

QuickTrim is a local desktop application. It has no network code, no telemetry,
no auto-update and no account system, and it never writes to the video it opens.
That removes most of the usual attack surface, but it does not remove all of it.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem.

Use GitHub's private reporting:
[Security → Report a vulnerability](https://github.com/quicktrim/quicktrim/security/advisories/new).

Include the version, the platform, a description of the impact, and the
smallest file or steps that reproduce it. If a malformed media file is
involved, say whether the crash is in `quicktrim.exe` or in the bundled
`ffmpeg.exe` / `libmpv-2.dll`, because that decides where the fix lands.

You can expect an acknowledgement within seven days. This is a volunteer
project, so a fix may take longer; you will get an honest status update rather
than silence.

## Supported versions

| Version | Supported |
| --- | --- |
| 0.1.x | yes — the only released line |
| older | no |

Fixes are published as a new release with a `CHANGELOG.md` entry. There are no
backports before 1.0.

## In scope

* The source in this repository: `src/`, `ui/`, `build.rs`, and the packaging
  scripts.
* Argument construction and process handling for the FFmpeg child.
* File naming, temporary-file handling, and the "never overwrite" rules.
* Dynamic library resolution (`libmpv`) on each platform.
* A crash or memory-safety bug caused by QuickTrim's own code.

## Out of scope

* Bugs in FFmpeg, libmpv, Slint or the Rust crates. Report those upstream; if
  QuickTrim needs to pin or update a version in response, open a normal issue.
* Denial of service from a deliberately malformed video file, unless it escapes
  the FFmpeg child process or corrupts files.
* Anything that requires an attacker to already have code execution or write
  access to the user's machine.
* "The bundle is large" and other packaging complaints — those are tracked in
  the roadmap, not a vulnerability.

## What the app already guarantees

These are design rules, and a report that one of them is false is in scope:

* The input file is opened read-only and is never modified.
* Output is written to a hidden temporary sibling and renamed only after it
  validates.
* An existing file is never overwritten.
* Helper binaries are looked up in a fixed order: environment variable,
  executable directory, then the system search path. There is no working
  directory in that list.
* No network access, no telemetry, no shell invocation. FFmpeg is started with
  an argument vector, not through a command string.
