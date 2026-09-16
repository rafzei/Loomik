# Verification record

Verified on 2026-09-16 on macOS 26.1, Apple Silicon.

The initial hardware captures below were made before the rename from Local
Loom to Loomik. Later sections describe subsequent Loomik builds and recordings.
Original artifact filenames and reports are preserved.
Commands below use the current application name; renaming does not represent a
new hardware recording test.

Rename verification: the `loomik` crate passes all 8 tests, formatting, and Clippy.
`target/release/bundle/Loomik.app` builds, passes signature verification, and
launches with the Loomik title. Its final UI smoke report is in
`artifacts/loomik-final/ui-smoke.json`: no startup error, FFmpeg ready, and Screen
Recording permission retained. The original system bundle identifier and signing
requirement are intentionally stable so a branding rename preserves macOS grants.

## Initial implementation checks

- `cargo test --all-targets`: 8 passing tests.
- `cargo fmt --check`: passes.
- `cargo clippy --all-targets -- -D warnings`: passes.
- Release application builds and `codesign --verify --deep --strict` passes.
- `plutil -lint packaging/Info.plist`: passes.

The tests encode and fully decode MP4, MOV, and MKV; verify H.264, dimensions,
frame counts, first/last frame colors, duration, and an AAC track; reject malformed
frames; and verify that an existing output file is never overwritten. Other tests
cover pause/resume timing through 100 hours, frame row padding, circular camera
cropping, mirroring, clipping, and negative desktop coordinates.

## Native release check

Command:

```sh
open -W -n "target/release/bundle/Loomik.app" --args \
  --reset-settings --verify-recording "$PWD/artifacts/release-native"
python3 scripts/check-native.py artifacts/release-native
```

The actual display, GENERAL WEBCAM, and GENERAL WEBCAM microphone were used.
The app recorded, paused for two wall-clock seconds, hid settings, moved/resized
the camera, restored settings, resumed, stopped, and captured a PNG.

| Requirement | Evidence |
| --- | --- |
| Real screen video | Release output is playable H.264, 1280×720, 15 fps, 76 frames |
| Stop/finalization | Complete 5.066667-second MP4, full FFmpeg decode succeeds |
| Pause removes elapsed time | Timer remained exactly 2.011984416 s throughout the pause; encoded duration is about 5 s despite about 7 s wall-clock capture |
| Microphone | 48 kHz mono AAC, matching duration, nonzero decoded audio peak 0.06779 |
| Floating controls excluded | Decoded native frames show desktop content at the settings/toolbar positions; neither app panel is in the recording |
| Camera circle included | Real camera image is circular without a square background |
| Move/resize camera | Native viewport moved from (60,520), diameter 200 points, to (588,308), diameter 270 points; decoded frames follow the change |
| Hidden settings | Verification continued while the root settings window was hidden and completed after reopening it |
| PNG screenshot | PNG decodes and dimensions match 1280×720; excluded panels remain absent |
| No duration limit | Encoder loops have no deadline/frame cap, video streams to fragmented MP4, initial implementation retained one latest frame per source and a bounded 32-chunk microphone queue; see current transport limits below |
| Reference-inspired UI | Actual settings, toolbar, and camera viewport renders reviewed; normal, expanded, collapsed, and 100-hour timer states fit without clipping |

Machine-readable results are in `artifacts/release-native/native-result.json` and
`artifacts/release-native/media-check.json`. UI renders are in `artifacts/final-ui`.
These local artifacts are gitignored because actual captures contain desktop and
camera content. The synthetic texture in UI-only smoke tests is never used in
normal recording or hardware verification.

## Practical limits

- The app targets macOS 13+, with hardware execution verified on macOS 26.1.
  Native Windows/Linux capture backends are not implemented; shared file-source
  code passes cross-target checks as detailed below. Mixed-DPI multi-monitor hardware
  was not tested.
- macOS can display its own direct-screen-capture confirmation in addition to
  Screen Recording permission. That system dialog can appear in captures while
  it is on screen; allow it before recording your final take. Loomik's own
  windows are excluded.
- Wireless Microphone was detected but supplied silence during its test. The
  webcam microphone supplied an active signal and was used for the final check.
- Disk space and CPU throughput remain physical limits. Finalization temporarily
  needs space for the working movie and final movie. System audio is not captured.
- Local keyboard shortcuts require a Loomik window to be focused.

The application bundle is locally signed for this machine, not notarized for
public distribution. FFmpeg remains an installed runtime dependency.

## Timestamp and performance update — 2026-09-16

The current implementation replaces CPAL audio callback-time estimates with
AVFoundation capture PTS. Screen/camera/audio are mapped to one host clock.
Audio writes run independently of the video pipe; every buffer is resampled
against converted timestamps and clipped to active recording intervals. Camera
composition caches crop/mirror/edge geometry; preview repaints on capture arrival.
VideoToolbox is selected only after an actual hardware probe; software fallback
uses x264 zero-latency tuning. Both disable B-frames.

Current automated checks: 14 passing tests, plus one explicitly ignored release
throughput benchmark. The tests include a simulated 100 ppm device-clock drift
for ten minutes, audio callbacks spanning start/pause/resume/stop, timestamped
history exhaustion, and an actual H.264/AAC flash/tone export. The decoded marker
onsets differ by less than 2 ms after a pause and delayed audio delivery. This
proves the synthetic timestamp/mux path, not physical webcam lip sync.

Native artifacts: `artifacts/performance-camera-60/native-result.json`,
`media-check.json` and the movie's `.performance.json` sidecar. Real 1920×1080
screen capture, GENERAL WEBCAM and its microphone produced 301 frames at 60 fps,
5.016667 seconds, H.264 without B-frames and 48 kHz mono AAC. Full decode passes;
audio signal is nonzero (peak 0.1138), the pause timer stays fixed, and PNG
size matches. Decoded frames confirm the moved circular camera and absent app
controls. These artifacts are local and gitignored.

| Current native 1080p60 measurement | Mean | p95 upper bound | Maximum |
| --- | ---: | ---: | ---: |
| Screen timestamp → callback copy | 0.95 ms | 2 ms | 2.61 ms |
| Camera timestamp → callback copy, including startup | 67.96 ms | 81 ms | 448.30 ms |
| Audio buffer end timestamp → writer processing | 1.81 ms | 3 ms | 3.90 ms |
| CPU composition | 0.63 ms | 2 ms | 1.71 ms |
| Write raw frame to encoder pipe | 1.61 ms | 4 ms | 10.18 ms |
| Selected camera frame age at output slot | 21.20 ms | 41 ms | 42.28 ms |

The output assembly budget is 100 ms, independent of immediate preview. Camera
cadence on this setup was about 24 unique frames/s (181 repeated camera frames
in 301 output slots), so the 60 fps screen movie cannot contain 60 distinct
webcam images per second. No camera frames were omitted. Three screen-history
misses held previous content instead of pulling future frames backward. Static
screen age is not capture latency: ScreenCaptureKit need not emit new pixels
for an unchanged display. Audio inserted four silence samples and skipped one
overlap sample while aligning timestamped buffers (at 48 kHz).

The release motion-pattern benchmark (`tests/performance.rs`, 180 frames at
1920×1080 with a camera circle) achieved **186.87 frames/s** with hardware H.264,
including pattern generation and video finalization. Encoder write p95 was
4 ms; one startup write reached 71.32 ms. This is one machine's throughput
measurement, not a universal deadline or complete capture/preview benchmark.

Memory: each source retains at most 32 historical frames / 300 ms / 128 MiB,
with a two-frame minimum; audio transport has 32 buffers, and latency histograms
are fixed-size. Recording duration does not grow media queues. The timeline
stores one small interval per pause/resume, not per frame. No hour-long physical
capture, 4K performance run, Windows/Linux run, or externally measured physical
sensor-to-display / clap-test lip sync is claimed by these checks.

Reproduction:

```sh
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
cargo test --release --test performance --locked -- --ignored --nocapture
bash scripts/bundle.sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --verify-recording "$PWD/artifacts/performance-60" --verify-fps 60
python3 scripts/check-native.py artifacts/performance-60
```

Final default-mode check: `artifacts/performance-final-30/media-check.json` verifies
1920×1080 at 30 fps, 151 frames / 5.033333 seconds, active AAC audio, pause/resume,
PNG dimensions and zero missing camera frames. This run also demonstrates real
scheduling variability: encoder pipe write maximum 49.15 ms, audio writer delivery
maximum 55.59 ms, while timestamp alignment inserted only three samples and
skipped one. Webcam delivery averaged 80.21 ms. The previous 60 fps measurements
are not a promise that any individual callback will meet a fixed deadline.

An additional failure-path integration test forces the hardware probe to fail,
then encodes and fully decodes software H.264 through the actual fallback path.

## Select controls update — 2026-09-16

All six selection fields share `src/ui/select.rs`: full-row device fields,
compact format/quality/fps fields, outlined chevrons, visible keyboard focus,
rounded white popup menus, full-width 36-point option rows and a selected check.
Long window/device names truncate inside the panel and remain available in
hover tooltips. Disabled controls are visibly dimmed and cannot open menus.

Two egui interaction tests verify pointer selection, Enter activation, Escape,
click-away dismissal, disabled fields and bounded long-label layout. The native
UI-only check renders every popup plus the selected microphone and disabled
states without capturing screen, camera or microphone data:

```sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --ui-smoke "$PWD/artifacts/selects-final" --select-smoke
```

Screenshots and the startup report are in `artifacts/selects-final` (gitignored).
The UI-only fixture never persists its device selections or settings.

## Window movement and countdown — 2026-09-16

Settings, expanded/collapsed controls, the camera and the countdown share a
background drag handler. Confirmation dialog headings also move their host
window. On macOS the handler samples the global cursor and updates the window
position from a fixed press anchor, avoiding a blocking AppKit drag loop and
window-local coordinate feedback. Child buttons/selects/sliders retain their
input priority. Toolbar position survives collapsing/expanding; camera movement
continues updating the recorded circle placement.

The worker prepares screen/audio streams, then waits for an explicit Begin
command. The UI presents 3, 2, 1 for one second each and 0 for 300 ms before
starting the shared media clock. Pre-start audio is excluded by the empty
timeline. Escape, Cancel, Stop, or closing the countdown cancels the pending
session. Own-app exclusion also covers the countdown viewport.

Validation: **24 tests pass**, with the existing throughput benchmark explicitly
ignored; formatting and Clippy with warnings denied pass. New checks exercise
digit timing, UI stalls, pre-roll clipping, cancellation/device failure, display
selection with negative coordinates, button/select hit testing, and viewport
movement/release/focus loss with injected pointer input. Those drag tests do not
automate a physical mouse gesture or test mixed-DPI monitor hardware.

Native check: `artifacts/countdown-native/native-result.json` records the exact
sequence `[3, 2, 1, 0]`, a zero media clock throughout the countdown, start after
zero, and cancellation of an earlier countdown without a movie. The subsequent
real 1920×1080/60 fps camera-and-microphone recording fully decodes: 301 H.264
frames, 5.016667 seconds, active AAC audio, a fixed pause timer, matching PNG
dimensions and no missing camera frames. Native countdown renders for 3 and 0
were inspected. Artifacts remain local and gitignored.
The final signed bundle also completed the UI-only select check with no startup
error; settings and toolbar renders in `artifacts/drag-ui-final` were inspected.

```sh
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
bash scripts/bundle.sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --verify-recording "$PWD/artifacts/countdown-native-new" --verify-fps 60
python3 scripts/check-native.py artifacts/countdown-native-new
```

## Toolbar hover stability — 2026-09-16

Native reproduction found a visual footprint change on hover: the record button
painted at `[18, 38, 59, 79]` when idle and `[17, 37, 60, 80]` when hovered.
The OS window itself stayed at `[24, 160, 78, 354]` in all 617 baseline samples.
The toolbar now applies zero expansion and no outside stroke directly to its
local widget styles, including the style selected by its native viewport theme.
Hover/press feedback changes fill color only. The builder owns size changes;
ordinary redraws no longer resend size or feed rounded OS positions back into
the toolbar builder. Explicit dragging still owns position changes.

`artifacts/hover-verified/hover-result.json` confirms identical idle/hover button
bounds and unchanged native geometry across 595 samples. The UI-only fixture
injects hover input inside egui without posting OS mouse events or recording
screen/camera/audio. Screenshots were reviewed, all 24 tests and Clippy pass,
and the release bundle was rebuilt.

```sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --ui-smoke "$PWD/artifacts/hover-check" --hover-smoke
```

## Shared sources and media backgrounds — 2026-09-16

Steps 9–10 are implemented. `RecordingSource` selects native desktop capture or
a direct image/video reader; `FrameSource` exposes source bounds and frames to
the existing recording worker. Native adapters now live in `src/capture/macos/`.
Windows/Linux select an explicit unsupported-device implementation, allowing
common UI/media code to compile without claiming native capture support.

Background studio handles PNG/JPEG and FFmpeg-readable MP4/MOV/MKV/WebM inputs,
Original/16:9/9:16/1:1 canvases, Fit/Fill, EXIF/rotation, transparency, seek,
hold/loop, canvas-relative camera placement and independent source/microphone
levels. Preview is silent. Recording consumes normalized media presentation
frames only for active output slots, so pausing does not advance the source.
Source audio uses the same seek/loop options during finalization; the original
file must remain available until export completes.

Decoding is incremental with a two-frame channel and bounded preview history.
Images decode once per reader. Static previews stop their worker after delivery;
gain slider changes do not restart video decoding. Moving/resizing a studio
window does not change canvas coordinates. Studio controls remain accessible
through a scrollable area at reduced window sizes; default portrait and landscape
renders were inspected in `artifacts/media-ui-final` and `media-ui-polished`.
No claim of physical mouse automation or mixed-DPI validation is made.

Automated verification: **29 passing tests**, one explicitly ignored release
throughput benchmark; formatting and Clippy with warnings denied pass. The five
new media integration tests inspect actual decoded output:

- Image Fit/Fill, transparent pixels, JPEG EXIF orientation, missing/corrupt media.
- Video seek, final-frame holding beyond source duration, looping, rotation,
  and variable-frame-rate conversion using presentation timestamps.
- Actual image-source worker export without desktop capture; paused clock stays
  fixed and every decoded output pixel remains in the expected color range.
- Source audio/microphone gains, looping, and decoded picture/tone onsets.
- Actual video-source worker with a 400 ms pause: the picture marker stays at
  output frame 15 (0.5 s at 30 fps) and the tone begins within 10 ms of 0.5 s.

Native media check: `artifacts/media-native-final/native-result.json` and
`media-check.json`. A one-second 1920×1080/30 fps test-pattern movie with sound
was looped into a **1920×1080/60 fps, 301-frame, 5.016667-second** H.264/AAC
recording using GENERAL WEBCAM and its microphone. Full decode passes; AAC
has a nonzero peak of 0.2362. Countdown cancellation and 3–2–1–0, the fixed pause
timer, and PNG output all pass. Decoded frames were visually inspected: only the
file background and circular camera appear; the moved/resized camera matches
the final canvas placement `(998.4, 475.2)`, diameter `378` pixels.

| Media 1080p60 measurement | Mean | p95 upper bound | Maximum |
| --- | ---: | ---: | ---: |
| Receive prefetched background frame | 0.004 ms | 1 ms | 0.022 ms |
| CPU camera composition | 0.56 ms | 1 ms | 4.23 ms |
| Encoder pipe write | 2.67 ms | 4 ms | 28.88 ms |
| Camera timestamp → callback copy, including startup | 78.33 ms | 90 ms | 596.54 ms |

The frame-receive metric measures queue retrieval, not full decoder work.
Hardware VideoToolbox was used. There were zero missing camera frames and
205 repeated camera frames across 301 output slots. Screen capture delivery and
screen age both have zero samples, consistent with the direct file-source path.
This hardware test ran with existing screen permission; it did not revoke the
permission. Source isolation is additionally covered by code and the image
worker integration test. These are processing measurements, not physical lip sync.

Desktop regression on the final signed build:
`artifacts/source-desktop-final/media-check.json` verifies 1920×1080/60 fps,
301 frames, 5.016667 seconds, active AAC (peak 0.0918), countdown cancellation,
pause/resume and matching PNG dimensions. Real GENERAL WEBCAM and its microphone
were used; zero missing camera frames and zero screen-history misses. Composition
averaged 0.41 ms and encoder pipe writes averaged 2.28 ms (maximum 3.92 ms).
An earlier regression run exposed a startup discovery race: the app cleared its
busy flag after receiving screens, before camera/microphone enumeration completed.
The flag now clears after the device result, and the final run includes both
devices. The 29-test suite and Clippy were rerun after that fix; bundle signature
verification and formatting pass. The studio UI hover fixture also retained one
unchanged toolbar window rectangle across 636 samples.
Decoded desktop frames were inspected: recorder panels are excluded and the
circular camera follows the changed position and size.

Portable checks run on this Mac:

```sh
cargo check --all-targets --target x86_64-pc-windows-gnu --locked
cargo check --all-targets --target x86_64-unknown-linux-musl --locked
```

Both pass, including compilation of test targets. They do not link or launch a
native Windows/Linux release. `.github/workflows/portable-builds.yml` defines
native macOS, Windows MSVC, and Ubuntu jobs with FFmpeg dependencies, formatting,
Clippy, tests, release builds, and artifacts. **The workflow has not executed.**
Windows and Linux hardware capture, native builds, and runtime verification
remain steps 11–13; Wayland and X11 must be checked separately.

Reproduce the media hardware test with a fresh artifact directory:

```sh
bash scripts/bundle.sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --verify-recording "$PWD/artifacts/media-check-new" --verify-fps 60 \
  --media /absolute/path/to/background.mp4 --media-loop
python3 scripts/check-native.py artifacts/media-check-new
```

All hardware recordings, screenshots, and diagnostic reports remain local in
gitignored `artifacts/`. The README graphic continues to use demo content.
