# Loomik implementation plan

Project name: **Loomik**. Repository: `https://github.com/rafzei/Loomik.git`.
This document was renamed from `local-loom-plan.md` with the application.

## Objective

Build a native Rust recorder for macOS, Windows, and Linux, matching the supplied
compact floating settings panel and dark recording toolbar. The recording source
can be a display, an application window, an image file, or a video file. A loaded
image or video acts as the recording background: the user can record webcam video
and microphone narration over it without recording their desktop. Recordings have
no application-imposed duration limit. The toolbar and settings never appear in
captured video. An optional live camera circle can be moved and resized and is
composited into the recording.

The completed initial implementation supports macOS 13 or later (verified on
macOS 26 on Apple Silicon). Steps 1–8 below describe that delivered version.
Shared source interfaces and media-file backgrounds (steps 9–10) are implemented
and verified on macOS. Windows capture code and packaging are implemented, but
native Windows validation, native Linux validation and runtime delivery remain in steps
11–13. Windows and
Linux must each support building from source and running on that operating system;
cross-compilation from macOS alone does not satisfy this requirement.

## Formats

- **MP4 / H.264**: default, broad compatibility and small files.
- **MOV / H.264**: convenient for QuickTime and editing on macOS.
- **MKV / H.264**: an open container, useful for long recordings and editing tools.

FFmpeg handles streaming encoding and finalization. It must be available locally;
the app checks for it and reports an actionable error when absent. No upload,
account, subscription, or recording time limit is involved.

## Implementation sequence

1. [x] **Foundation** — Cargo project, modules, error handling, configuration,
   README, macOS application bundle and privacy usage descriptions.
2. [x] **Recording model** — explicit idle/starting/recording/paused/stopping/error
   transitions, elapsed active time, output naming, bounded memory, no time limit.
3. [x] **Native capture** — discover displays and windows; request Screen Recording
   permission; use ScreenCaptureKit filters that exclude this application even
   when floating windows are created or moved after recording starts.
4. [x] **Encoder** — stream frames to FFmpeg, MP4/MOV/MKV, correct frame pacing,
   pause/resume without recorded gaps, graceful stop, errors surfaced to the UI,
   recoverable intermediate files, microphone recording and mute selection.
5. [x] **Camera** — discover connected video devices, select/disable preview,
   capture frames, render a floating circular preview, move/resize it, mirror it,
   composite only the camera pixels into the saved screen recording.
6. [x] **Floating UI** — reference-inspired settings, screen/window and device
   selectors, format/quality/output settings, orange Start button; dark floating
   toolbar with stop, timer, pause/resume, restart, discard, and collapse controls.
   Confirm destructive restart/discard inside the app. Keep controls responsive
   while capture and encoding run on workers.
7. [x] **Verification** — meaningful tests of timing, frame composition, encoder
   output, pause/resume, finalization, and failure paths; cargo fmt/check/clippy;
   FFprobe validation of each format; native launch and visual review; verify
   screen-window exclusion and webcam/microphone with available hardware and
   macOS permissions. Record any verification that requires user interaction.
8. [x] **Delivery** — release build, launchable .app, usage/setup instructions,
   keyboard shortcuts, limitations, and final requirement-by-requirement audit.
9. [x] **Shared source and platform interfaces** — extend source selection to
   Display / Window / Image / Video; separate the recording canvas and its
   coordinates from global desktop coordinates. Define common frame-source,
   camera, microphone, permission, and window-management interfaces with native
   implementations selected through Cargo target dependencies and `cfg` gates.
   Remove unconditional macOS imports, framework links, and the non-macOS
   compile error. Keep the working macOS path covered while introducing these
   abstractions; media-file recording must not require screen-capture permission.
10. [x] **Image/video backgrounds and recording over media** — implement the
    media workflow detailed below: file selection, preview canvas, image/video
    decoding, camera placement relative to the canvas, microphone narration,
    playback controls, audio mixing, and export through the existing encoder.
    The output must come directly from media frames plus overlays, so unrelated
    desktop content and editor controls cannot enter the recording.
11. [ ] **Windows implementation and native build** — support Windows 11
    on x86_64. Implement display/window capture using
    Windows Graphics Capture, native camera discovery/capture using Media
    Foundation or an equivalent maintained Rust adapter, and microphone input.
    Implement floating transparent windows, dragging/resizing, DPI handling, and
    exclusion of recorder windows using the supported Windows capture-exclusion
    APIs. Handle denied permissions and disconnected devices. Provide Windows
    FFmpeg discovery, output paths, file dialogs, and open-in-Explorer actions.
    Build with Rust's MSVC toolchain on Windows and produce a runnable `.exe`
    distribution with dependency/setup instructions; verify it on Windows.
12. [ ] **Linux implementation and native build** — support native x86_64 Linux
    builds, initially using Ubuntu LTS as the reference environment. Implement
    Wayland capture through XDG Desktop Portal / PipeWire and an X11 capture
    backend; support camera devices through V4L2 or an equivalent maintained
    adapter and microphone input through an appropriate audio backend. Verify
    floating-window placement, resizing, scaling, permissions, and device access
    separately on Wayland and X11. Investigate control-window exclusion and
    positioning restrictions per compositor before claiming feature parity:
    where exclusion is unavailable, provide an explicit safe capture mode or
    capability limitation instead of silently recording the controls. Media
    backgrounds must work independently of desktop-capture restrictions. Provide
    Linux FFmpeg discovery, desktop integration, system-package prerequisites,
    and a runnable distribution. Build and launch it on Linux itself.
13. [ ] **Cross-platform verification and delivery** — add native macOS, Windows,
    and Linux CI jobs for formatting, checks, tests, and release builds with
    explicit FFmpeg/native dependencies. Exercise all source types, exports,
    pause/resume, long-running recording, camera geometry, microphone/source
    audio, and errors. Run real capture/UI checks on Windows and on Linux Wayland
    and X11; a headless build or CI test alone is not proof of hardware capture.
    Publish local build instructions and packaged artifacts for each supported
    OS, document verified versions and backend limitations, and update the
    requirement-by-requirement verification record.

14. [x] **macOS performance and synchronization** — use native screen/camera/audio
    acquisition timestamps on one host clock, compensate device-clock drift,
    clip audio at pause boundaries, timestamp controls independently of encoding,
    keep bounded source history, select historical frames after encoder stalls,
    and stop on sustained overload. Use hardware H.264 with a verified software
    fallback, cache camera composition geometry, and repaint preview on arrival.
    Validate 1080p60 with real screen/camera/microphone, timestamp/pause/drift tests,
    decoded flash/tone alignment, and a release throughput benchmark. Capture
    measured limits in `VERIFICATION.md`; do not promise zero sensor latency.

## Media-background workflow and acceptance criteria

- **Load a background:** choose Image or Video in the source selector, then pick
  a local file. Initially support PNG/JPEG images and MP4/MOV/MKV/WebM video
  inputs where the installed decoder supports their codecs. Show a useful error
  for missing, corrupt, unsupported, or unreadable media. Input formats and export
  formats are separate choices.
- **Prepare the canvas:** preview the background before recording. Choose output
  dimensions/aspect ratio and Fit or Fill behavior; preserve media proportions
  and handle image orientation, transparency, and video rotation metadata. Keep
  the existing compact floating settings and controls.
- **Record over the background:** an image stays static; a video supplies its
  decoded frames. Add the live camera circle and microphone narration as selected.
  Users can move/resize the circle on the preview canvas, and exported placement
  must match. This mode must work with desktop capture disabled and must never
  include unrelated screen content or canvas editor controls.
- **Control video playback:** provide preview play/pause and seek to a starting
  position. Recording pause freezes the source-video playhead and recording clock;
  resume continues both without inserting a gap. Offer explicit end-of-video
  behavior: hold the final frame (default) or loop. Reaching the end of a short
  background video must not impose a recording-duration limit.
- **Choose audio:** allow the source video's audio to be muted or mixed with
  microphone narration, with separate levels. Keep audio synchronized through
  pause/resume, looping, and differing source/output frame rates. A background
  without audio must still work with microphone-only or silent output.
- **Stream and export:** decode long videos incrementally with bounded buffering,
  use timestamps for variable-frame-rate media, and reuse MP4/MOV/MKV export and
  interrupted-session recovery. Verify portrait/landscape images, rotated video,
  short looping clips, clips without audio, and recordings longer than the source.
- **Prove the result:** decode exported media and check background pixels, camera
  position/size, duration, audio timing, and absence of UI/desktop pixels. Test
  media-file recording on macOS, Windows, and Linux.

## Architecture

- `src/app.rs`, `src/ui/`: egui/eframe multi-viewport native floating windows.
- `src/model.rs`: settings, recording state, monotonic pause-aware timer.
- `src/capture/`: target-selected device interface, macOS ScreenCaptureKit and
  AVFoundation under `macos/`, Windows WGC/MediaCapture/WASAPI under `windows/`,
  and portal/PipeWire, XComposite, V4L2 and ALSA under `linux/`. Linux
  currently exposes window capture and a canvas-relative camera studio.
- `src/media/`: file probing, oriented image loading, bounded video decoding,
  seek/loop/fit options; preview and canvas-relative placement in
  `src/ui/media_canvas.rs`.
- `src/recording/`: bounded timestamped frame history, circular camera composition,
  streaming hardware/software FFmpeg worker, dedicated timestamped audio writer,
  shared desktop/media frame sources, canvas composition,
  background-audio/microphone mixing, finalization and recovery.
- `scripts/`: reproducible app packaging and runtime verification helpers,
  including MSVC Windows ZIP and native Ubuntu `.deb` packaging scripts.
- `src/platform.rs`: platform-specific file reveal.
- `.github/workflows/`: configured native build/test matrix for all three
  systems; workflow execution and native Windows/Linux validation still pending.
- `tests/`: tests that inspect actual encoded media in addition to pure logic.

## Invariants and acceptance evidence

- No duration cap and no accumulation of the whole movie in RAM: inspect worker
  loops, queue bounds, and file-writing strategy; exercise repeated pause/resume.
- Timer excludes paused time and continues through hours: deterministic tests.
- Capture excludes all Loomik windows: native filter inspection and a real
  recording with the toolbar/settings over known screen content.
- Camera circle is visible in output, follows its on-screen position/size, and
  has no square background: image tests plus hardware recording when permitted.
- Stop produces playable files in all three formats: FFprobe/decode checks.
- Settings, toolbar and camera remain floating, draggable, and usable: native
  runtime check; capture errors and permission denials are visible and recoverable.
- No claim of hardware or permission verification without direct evidence.
- Media backgrounds work without Screen Recording permission and without any
  dependency on the desktop's current contents: test source isolation and inspect
  the exported background/camera composition.
- A background video's duration does not cap the output recording: exercise both
  end-frame holding and looping, including pause/resume and synchronized audio.
- Windows and Linux support requires successful source builds, launchable release
  artifacts, and runtime checks performed on those operating systems. Verify Linux
  Wayland and X11 separately and record any compositor-specific restrictions.

## Progress and decisions

- Renamed the application, Rust crate, executable, documentation, and plan to
  Loomik. Git origin for fetch/push is `https://github.com/rafzei/Loomik.git`.
  The native system identity remains stable to preserve macOS privacy grants;
  the rebuilt `Loomik.app` passed launch and signature checks.
- 2026-09-16: Inspected the empty workspace and confirmed Rust, FFmpeg, FFprobe,
  and macOS 26.1 are available. Created this plan before implementation.
- Native egui is the UI library. The web landing-page design skill is outside this
  app's scope; the user's screenshots are the design authority.
- Implemented the Rust UI, ScreenCaptureKit adapter, native AVFoundation camera,
  CPAL microphone, streaming encoder, recovery files, and three export formats.
- Eight automated tests pass, including actual H.264/AAC exports and full decode.
  Clippy passes with warnings denied. Fixed the Swift runtime rpath discovered
  by running the native test executables.
- Rendered and inspected real settings, toolbar, and camera windows. UI smoke
  fixtures are explicitly separate from hardware recordings.
- User enabled macOS permissions. Two hardware recordings succeeded on the
  1920×1080 display with GENERAL WEBCAM. The pause timer remained unchanged for
  the full pause; saved video is 5.0667 s. Decoded frames confirm excluded app
  windows and a camera circle moved from (60, 520), 200 px, to (588, 308), 270 px.
- Wireless Microphone supplied silence. GENERAL WEBCAM microphone supplied real
  audio (−29.9 dB mean, −10.4 dB peak), encoded as 48 kHz mono AAC.
- Release build and signature checks pass. Final release hardware verification
  also covers hidden settings and PNG capture. Final normal/expanded/collapsed/
  100-hour layouts were rendered and inspected. All acceptance evidence and
  practical limits are recorded in `VERIFICATION.md`.
- Scope extension requested after the initial macOS delivery: add image/video
  files as recording backgrounds and native build/run support for Windows and
  Linux. Steps 9–10 are now implemented; steps 11–13 remain open.
- Shared `RecordingSource` and frame-source contracts isolate desktop coordinates
  from media canvas coordinates. macOS native modules are target-gated; media
  recording never creates a desktop capture stream or requests its permission.
- Background studio supports PNG/JPEG and MP4/MOV/MKV/WebM, oriented previews,
  aspect/Fit/Fill controls, seeking, hold/loop, canvas camera movement/resizing,
  microphone narration, and optional source audio with independent levels.
  Preview is silent; source audio is decoded at export, so the source must remain
  available until saving finishes. Decoding uses a bounded two-frame queue.
- 29 automated tests pass on macOS, including decoded file-source exports,
  transparency/EXIF/rotation/VFR, hold/loop, and audio/video pause alignment.
  Cross-target `cargo check --locked` passes for `x86_64-pc-windows-gnu` and
  `x86_64-unknown-linux-musl`. These checks are not native builds or runtime tests.
  The three-OS CI workflow is present but has not been executed.
- Native media and desktop regression recordings both pass at 1920×1080/60 fps
  with a real camera/microphone, pause/resume, countdown cancellation and PNG
  output. The media recording has no screen-capture stream. Fixed a discovery
  race that could mark the initial device refresh complete before cameras and
  microphones arrived. Full results are in `VERIFICATION.md`.
- Windows implementation progress (step 11 remains open until native validation):
  WGC display/window capture, MediaCapture camera, WASAPI microphone, QPC clock
  mapping, bounded frame/audio transport, SIMD resize, physical-pixel camera
  coordinates, native cursor dragging, window-exclusion hook/verification,
  NVENC/QSV/AMF encoder probes with x264 fallback, MSVC packaging script, and
  Windows CI packaging are implemented. Native Windows build, launch, device,
  exclusion, mixed-DPI and physical synchronization checks are still required.
- Moved whole-app quit to X on the floating controls (expanded and collapsed),
  removed Quit from settings, and kept save/cancel behavior during active takes.
- The next implementation milestones are Windows native MSVC build/validation (11),
  Linux Wayland/X11/device capture/native build (12), then the runtime and release
  matrix (13). Cross-platform media acceptance remains part of that final matrix.

## Realtime requirements for every source and OS

- Preview consumes the newest available camera frame and must not wait for the
  recording assembly buffer. Video, microphone and future media-background audio
  use native acquisition/presentation timestamps mapped to one monotonic clock.
- Keep camera/screen/audio timing through start, pause, resume, stop, delayed
  callbacks, encoder stalls and long-running device drift. Do not assign the
  current image to old output slots when catching up after overload.
- Use bounded queues, hardware encoders where available, cached/GPU composition,
  and explicit overload reporting. Measure 1080p30/60 on each target platform;
  measure 4K separately before claiming it is realtime.
- Report capture delivery, composition, encoder submission, source cadence and
  synchronization separately. A 60 fps file does not imply a 60 fps webcam.
- Validate exported flash/tone markers and physical clap/flash lip sync on each
  hardware setup. Sub-millisecond physical alignment is not a universal acceptance
  promise; source frame intervals and device timestamp accuracy are real limits.
- Windows capture code is implemented but awaits native verification; Linux
  capture code and `.deb` packaging are implemented but native verification is
  deferred. Media backgrounds are implemented
  and measured on macOS; their cross-platform runtime and hardware verification
  remain part of step 13.


## Linux implementation and 1.0.0 delivery update

- Implemented Wayland XDG ScreenCast window selection with PipeWire mapped video
  and acquisition PTS; each consumer opens its own portal remote connection.
- Implemented XComposite window-pixmap capture on X11. Full-monitor capture is
  deliberately unavailable because compositor-independent exclusion of Loomik
  controls cannot be guaranteed. Linux currently omits the cursor.
- Added V4L2 camera capture, CPAL/ALSA microphone input, explicit clock mapping,
  bounded buffers, cancellation and device/portal errors. X11 pixmap-read timing
  is an estimate and requires physical synchronization measurement.
- Linux Recording studio positions/resizes the camera in output coordinates;
  its preview consumes the newest native frame independently of the encoder.
  Wayland global placement/always-on-top behavior remains compositor-dependent.
- Added Ubuntu `.deb` packaging, desktop launcher/icon, runtime dependencies and
  native build prerequisites/packaging in GitHub Actions. CI has not run yet.
- At the user's request, do not start Docker or perform Ubuntu runtime testing
  during this stage. Step 12 stays open until native Linux build and Wayland/X11
  hardware/UI checks are completed. Prior musl checks predate the native backend.
- Release 1.0.0 will be published on GitHub with macOS, Windows and Ubuntu
  downloads only after verification. Native builds, clean-machine installation,
  distribution archives/dependencies, signing decisions and release checksums
  are tracked in `docs/releasing.md`. Keep version 0.1.0 until those gates pass.

## Authorized release scope update

- The user will verify Windows/Ubuntu hardware capture after publication and
  explicitly authorized push plus GitHub release 1.0.0 after automated checks.
  This supersedes the earlier requirement to wait for those physical devices.
- Added source-built, bundled FFmpeg/FFprobe with source archives/licenses,
  native macOS ZIP/DMG and Windows ZIP verification, static MSVC CRT, Rust notices,
  package smoke checks and checksums. Version is now 1.0.0 in preparation.
- Native CI targets Apple Silicon, Intel, Windows MSVC and Ubuntu. Ubuntu also
  runs an Xvfb test of XComposite isolation and an installed-package GUI smoke.
  Docker is not used. Release notes retain unverified hardware/compositor limits.
