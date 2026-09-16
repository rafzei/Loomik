# Loomik on Windows

The Windows backend is implemented and checked by cross-target compilation from
macOS. **A native Windows build, launch, capture test, and performance measurement
have not yet been performed.** This is a development build path, not a verified
Windows release. Linux native capture remains a separate plan milestone.

## Build on Windows

Requires Windows 10 version 2004 (build 19041) or Windows 11, x64; Rust 1.88+
with the MSVC target; Visual Studio C++ Build Tools and Windows SDK; and FFmpeg
with FFprobe and libx264 on PATH. Installing FFmpeg via Chocolatey is one option:

```powershell
choco install ffmpeg -y
rustup target add x86_64-pc-windows-msvc
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked --target x86_64-pc-windows-msvc
```

Release packaging uses [source-built media tools](media-tools.md) and
`scripts/bundle-windows.ps1` from an x64 Visual Studio developer shell. It creates
`target/release/bundle/Loomik-1.0.0-Windows-x64.zip`, including FFmpeg/FFprobe,
licenses and corresponding source. Extract the entire folder and run Loomik.exe;
no separate FFmpeg or Visual C++ runtime installation is required. The package
is unsigned. User-supplied tools can be selected with `LOOMIK_FFMPEG` /
`LOOMIK_FFPROBE`.

## Capture and permissions

- Displays and windows use Windows Graphics Capture. Minimized/closed source
  windows end recording with a recoverable error. Windows may draw its system
  capture border; Loomik leaves the OS setting intact.
- Camera uses Windows MediaCapture/MediaFrameReader, selects a fast mode up to
  60 fps when available, converts to BGRA with a bounded-size realtime preview,
  and requests only video. Microphone capture is independent through WASAPI.
- In Windows Settings → Privacy (Windows 10) or Privacy & security (Windows 11),
  enable Camera and Microphone access, including access for desktop apps.
  If a device is busy, close any application using it exclusively and refresh.
- Every Loomik top-level window is marked `WDA_EXCLUDEFROMCAPTURE` before it is
  shown. The recorder checks that visible app windows remain protected before
  publishing a captured frame and reports an error if protection is unavailable.
- Media backgrounds bypass desktop capture and its window-exclusion checks.
  Camera placement in the studio uses canvas coordinates on every platform.
- X at the bottom of the floating controls quits the whole app. During a take,
  **Save & quit** finishes export first. X in settings only hides that panel.

WGC, camera and WASAPI acquisition timestamps map from QPC to the same monotonic
clock. The existing bounded history, 100 ms assembly allowance, pause clipping,
audio resampling, and overload reporting are shared with macOS. CPU frame scaling
uses SIMD. NVIDIA NVENC, Intel QSV and AMD AMF are probed with actual frames and
low-latency options; libx264 is the fallback. Physical latency and actual hardware
cadence require measurement on each Windows system.

## Native acceptance check still required

From the repository root, with a fresh output directory:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc
./target/x86_64-pc-windows-msvc/release/loomik.exe --reset-settings --verify-recording "$PWD/artifacts/windows-native" --verify-fps 60
python scripts/check-native.py artifacts/windows-native
```

The verifier records a short local take, cancels one countdown, verifies the next
3–2–1–0 sequence, pauses, moves/resizes the camera, resumes, exports and takes a
PNG. It uses available devices; inspect `native-result.json` to confirm both
camera and microphone were actually selected. Append `--media C:\path\clip.mp4
--media-loop` to verify a looping background with source audio.

Check exported frames for excluded controls, camera geometry, audio and motion.
Test permissions denied, device unplug, source closure/minimization, display
changes, 100%/150%/200% DPI, mixed-DPI monitors, physical camera/microphone sync,
all three export formats, and long recordings. A successful compilation or CI
run alone does not satisfy these hardware acceptance checks.
