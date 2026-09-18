# Loomik on Ubuntu

Native Ubuntu 24.04 CI builds and installs the `.deb`, runs the automated media
tests and GUI smoke check, and exercises isolated X11 capture under Xvfb.
Wayland portal, physical camera/microphone and compositor behavior remain
unverified; CI evidence and its scope are recorded in
[VERIFICATION.md](../VERIFICATION.md).

## Build and install

The initial target is **Ubuntu 24.04 LTS, x86_64**, Rust 1.88 or newer:

```sh
sudo apt update
sudo apt install build-essential pkg-config clang libclang-dev linux-libc-dev \
  libpipewire-0.3-dev libspa-0.2-dev libasound2-dev libegl1-mesa-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libxinerama-dev ffmpeg dpkg-dev desktop-file-utils \
  xdg-desktop-portal
cargo build --release --locked
./target/release/loomik
```

Install the portal backend for your desktop as well (normally preinstalled):
`xdg-desktop-portal-gnome` for GNOME, `xdg-desktop-portal-kde` for KDE Plasma.
A portal that supports only monitor capture cannot provide Loomik's isolated
window recording; file backgrounds are still available.

```sh
bash scripts/bundle-linux.sh --skip-build
sudo apt install ./target/release/bundle/Loomik-1.1.0-Ubuntu-24.04-amd64.deb
```

The `.deb` installs the app, desktop launcher and icon. APT installs FFmpeg and
native runtime dependencies. Do not copy the binary to older distributions and
assume ABI compatibility. The release download is the package built and checked
on Ubuntu by the native CI workflow.

## Recording sources

- **Wayland:** click **Choose window…**, then choose another application's window
  in the system sharing dialog. Loomik hides its windows while the chooser is
  open. The portal grants access to that single window; the PipeWire session
  stays alive until another source replaces it or Loomik exits. To change windows,
  reopen the chooser. Dismissal and portal failures are shown in the app.
- **X11:** select a visible application window. XComposite reads its redirected
  pixmap, independently of windows over it. Loomik's own windows are filtered
  from the list. Closing/minimizing the source ends capture with an error.
- **Image/video files:** use Background studio as on other platforms. Files do
  not require a portal or access to another application's pixels.

Whole-monitor capture is unavailable: Linux compositors do not offer a universal
API to exclude floating recorder controls. The cursor is currently omitted on
Linux. The Wayland chooser is controlled by the desktop; never choose a Loomik
window if your compositor offers hidden windows in its list.

## Camera, microphone and timing

Recording studio previews the selected window once capture starts. Drag the
camera circle inside this canvas; resize with its slider or the mouse wheel.
Positions belong to the output image and survive quality changes. The studio
receives the latest native frame directly, independently of encoding. Controls
and studio chrome are not composited into the saved video.

Wayland does not promise absolute floating-window positions or always-on-top
behavior; window placement, countdown placement and dragging must be verified
per compositor. The camera lives inside the studio on Linux, not in a separate
globally positioned overlay window.

Camera capture uses V4L2 with a three-buffer mmap stream, requests 640×480/30 fps,
and accepts MJPEG or YUYV (BT.601/709, full or limited range). Drivers choose the actual supported format.
Devices must provide monotonic timestamps. Check camera access/session ACLs and
close other camera consumers if opening fails. Microphone capture uses CPAL/ALSA;
choose the desktop's default input or the desired exposed ALSA device. System
audio capture is not implemented.

PipeWire presentation timestamps and V4L2 acquisition timestamps map to the Linux
monotonic clock. ALSA's separate stream timeline is recalibrated periodically;
audio is resampled to the shared recording timeline. X11 lacks native acquisition
timestamps in this path, so it uses the synchronous pixmap read midpoint as an
estimate. That estimate, device exposure latency and physical lip sync need real
measurement; no zero-latency guarantee is made.

## Remaining hardware validation

- Install the release `.deb` on the target desktop; automated CI covers build,
  Clippy, media tests and package installation on Ubuntu 24.04.
- GNOME/KDE Wayland chooser, cancellation, re-selection, portal shutdown,
  mapped-memory negotiation, HiDPI geometry and exclusion of floating controls.
- X11 window isolation under occlusion/movement, source resize/minimize/closure.
- Camera and microphone selection/disconnection, pause/resume, long takes,
  1080p30/60 load, recorded flash/tone markers and physical lip sync.
- Image/video backgrounds, all three output containers, PNG, countdown and quit.

CI uses native Ubuntu runners, without Docker. Xvfb checks cover a virtual X11
display; they do not establish behavior on every physical desktop/compositor.
