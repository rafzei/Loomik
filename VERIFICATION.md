# Verification

## Published release

[Loomik 1.0.0](https://github.com/rafzei/Loomik/releases/tag/v1.0.0) was published
on 2026-09-17 from commit `1efdc45af81cd3d227831c82b86ce84e1e3434ff`.
All four native jobs passed in
[build run 35202831277](https://github.com/rafzei/Loomik/actions/runs/35202831277).
The release contains six installers/archives, package reports and SHA-256 checksums.
These results apply to that commit; later source changes require fresh checks.

| Platform | Automated evidence | Hardware evidence |
| --- | --- | --- |
| macOS ARM64 and Intel | Build, Clippy, Rust/Python tests, ZIP/DMG and standalone media checks | Screen, camera and microphone tested on macOS 26.1 / Apple Silicon |
| Windows x64 | MSVC build on Windows Server 2025, tests, ZIP and standalone media checks | Windows 11 capture, devices, GUI and mixed DPI remain unverified |
| Ubuntu 24.04 amd64 | Build, tests, `.deb` installation, GUI smoke and isolated X11 capture under Xvfb | Wayland, physical devices and compositor behavior remain unverified |

macOS/Windows packages run with only system directories on PATH, locate bundled
FFmpeg/FFprobe, and export/decode H.264/AAC in MP4, MOV and MKV. Checks cover frame
count, colors, dimensions, duration and nonzero audio. Dependency checks reject
unbundled libraries. macOS checks also verify architecture, ad-hoc signatures
before/after extraction and disk-image integrity. Windows packages are unsigned;
macOS packages are not notarized.

The release workflow verifies source-run provenance, commit ancestry, every native
job, package reports and checksums before publishing the exact tested archives.
See [releasing](docs/releasing.md) for the procedure.

## macOS hardware observations — 2026-09-16

Physical screen, camera and microphone checks produced playable 1920×1080 H.264
recordings at 30 and 60 output fps, with 48 kHz mono AAC. Full decoding, PNG
dimensions, countdown cancellation, 3–2–1–0 and pause/resume checks passed.
Decoded frames showed excluded recorder controls and a circular camera overlay
following movement/resizing. Image/video backgrounds were also tested, including
a looping one-second video with source audio and microphone narration.

The final desktop and media checks each produced 301 frames / 5.016667 seconds at
60 fps, with nonzero audio and no missing camera frames. Camera cadence was lower
than output cadence, so repeated webcam frames were expected.

| Measured 1080p60 operation | Desktop mean | Media-background mean |
| --- | ---: | ---: |
| CPU composition | 0.63 ms | 0.56 ms |
| Encoder pipe write | 1.61 ms | 2.67 ms |
| Camera timestamp to callback, including startup | 67.96 ms | 78.33 ms |

These are observations from one Mac, not performance guarantees or physical
lip-sync measurements. A separate synthetic motion/composition/export benchmark
achieved 186.87 fps with VideoToolbox. No hour-long physical capture, 4K load,
mixed-DPI setup or external clap/flash latency measurement was performed.

Recordings and reports stay in gitignored `artifacts/` because they contain
desktop/camera content. The README illustration uses synthetic demo content.

## Reproduce automated checks

Local source validation on 2026-09-17 (macOS ARM64, Rust 1.99 nightly,
source-built FFmpeg 8.0.3): **33 Rust tests and 25 Python tests passed**; formatting
and Clippy passed. One throughput benchmark was intentionally ignored. Regression
checks cover disguised playlists, replaced oversized images and private recovery
directory permissions. The FFmpeg source checksum, ARM64 architecture and
system-only binary dependencies were checked. This is not a new hardware test
or a verification of updated Windows/Linux release packages.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Media tests require FFmpeg and FFprobe on PATH. They cover all export containers,
collision-safe saving, image orientation/transparency, Fit/Fill, video rotation,
variable frame rates, seek/hold/loop, audio mixing and worker pause/resume.
Synthetic timing tests cover delayed callbacks, ten minutes of simulated device
drift, pause boundaries and decoded flash/tone alignment. The release throughput
benchmark is intentionally ignored in normal test runs:

```sh
cargo test --release --test performance --locked -- --ignored --nocapture
```

## Reproduce hardware checks

Camera-only source validation on 2026-09-18 (macOS ARM64): **37 Rust tests passed**,
with formatting, Clippy, and a local release bundle build passing. Four camera
regressions cover acquisition timestamps, full-frame resizing/mirroring, decoded
movie edges, pause/resume, countdown cancellation, and recovery after disconnect.
A GUI smoke check verified the camera-only settings and rectangular preview using
synthetic frames. This does not establish physical camera/microphone behavior in
the new mode or verify Windows/Linux builds.

On macOS, use a fresh output directory and grant capture permissions:

```sh
bash scripts/bundle.sh
open -W -n "target/release/bundle/Loomik.app" --args --reset-settings \
  --verify-recording "$PWD/artifacts/native-check" --verify-fps 60
python3 scripts/check-native.py artifacts/native-check
```

Append `--media /absolute/path/to/background.mp4 --media-loop` to exercise a file
background, or `--camera-only` for a full camera frame without desktop capture.
Inspect the saved video and `native-result.json` to confirm which
devices were used, control exclusion, camera placement and audio. UI smoke tests
use a synthetic camera texture and do not establish capture behavior.

Remaining acceptance checks are listed in the [Windows](docs/windows.md) and
[Ubuntu](docs/linux.md) guides. System/desktop audio capture is not implemented.
