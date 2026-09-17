# Release 1.0.0

The 1.0.0 release is authorized after automated build/package verification. The
user will perform Windows and Ubuntu camera/microphone/compositor checks after
publication. Those checks remain explicitly unverified in release notes;
macOS hardware verification is recorded in VERIFICATION.md.

## Downloads

| Target | Assets |
| --- | --- |
| macOS 13+, Apple Silicon | `Loomik-1.0.0-macOS-arm64.zip` and `.dmg` |
| macOS 13+, Intel | `Loomik-1.0.0-macOS-x86_64.zip` and `.dmg` |
| Windows 11 x64 | `Loomik-1.0.0-Windows-x64.zip` |
| Ubuntu 24.04 LTS amd64 | `Loomik-1.0.0-Ubuntu-24.04-amd64.deb` |

macOS/Windows packages include FFmpeg/FFprobe, licenses, corresponding source
archives and build recipes. Ubuntu installs runtime dependencies through APT.
Rust dependency notices are included. macOS packages are ad-hoc signed, not
notarized; Windows packages are unsigned. Installation instructions describe the
resulting OS prompts.

## Publication gates

- Native GitHub Actions matrix passes on Apple Silicon, Intel, Windows MSVC
  and Ubuntu, using the exact commit to tag.
- Rust formatting, Clippy and tests pass. macOS/Windows tests use bundled media
  tools. ZIP extraction preserves executable modes/signatures and succeeds from
  paths containing spaces. Packaged executables export and decode H.264/AAC in
  MP4/MOV/MKV with only system directories on PATH.
- Ubuntu builds/installs the `.deb`, runs GUI smoke checks under Xvfb and verifies
  isolated XComposite window pixels beneath occluding controls. These tests do
  not substitute for Wayland portal or physical device checks.
- Archive SHA-256 checksums match downloaded Actions artifacts. Attach those
  exact tested archives to GitHub Release `v1.0.0`, with a combined checksum file.
- Release notes describe actual verification and known limits: Linux window-only
  capture/no cursor, compositor-dependent window positioning, unsigned/notarized
  status, and outstanding Windows/Ubuntu hardware measurements.

Actions artifacts are temporary outputs; GitHub Releases hosts the permanent
application downloads. The build workflow has read-only repository permissions
and never publishes automatically. Publication happens after the gates above.
No Docker is used.
