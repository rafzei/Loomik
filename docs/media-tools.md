# Bundled FFmpeg and FFprobe

macOS/Windows distribution archives include FFmpeg and FFprobe beside the
application executable. Users do not need Rust, Homebrew, Chocolatey or a separate
FFmpeg installation. Ubuntu's `.deb` uses FFmpeg from APT instead.

The source build uses FFmpeg 8.0.1 and a fixed x264 commit; exact upstream URLs
and SHA-256 checksums are in `packaging/media-sources.env`. Windows additionally
includes NVIDIA codec headers 12.1.14.0 for NVENC. These are separate processes
with static codec libraries and system-only dynamic dependencies. Network
protocols are disabled in bundled FFmpeg. Local media decoding, filters, H.264
encoding and AAC audio remain enabled.

macOS includes VideoToolbox and x264. The initial Windows bundle includes NVENC
and x264. QSV/AMF probes remain available with an external FFmpeg build containing
those encoders; the bundle does not claim Intel/AMD hardware encoding. Driver
compatibility and performance require native hardware measurements.

## Build on macOS

Install Xcode Command Line Tools and `pkg-config`. Intel builds also need `nasm`
for assembly; Apple Silicon uses Clang's assembler. Python 3.11+ is required.

```sh
bash scripts/build-media-tools.sh
python3 scripts/package-desktop.py
```

`LOOMIK_BUILD_JOBS` overrides parallelism. Build Apple Silicon and Intel archives
on their respective native runners. Binaries target macOS 13.0. The packager
checks architectures/dependencies, signs the app and tools ad hoc, creates ZIP/DMG,
verifies signatures after extraction to a path with spaces, tests synthetic
recording and emits SHA-256 checksums. `--skip-build` reuses a Rust release build;
`--no-dmg` creates only ZIP. Ad-hoc signing is not Apple notarization.

## Build on Windows

Use Rust MSVC and Visual Studio C++ Build Tools/Windows SDK. The repository enables
static MSVC CRT linkage. Build FFmpeg in an **MSYS2 MINGW64** shell:

```sh
pacman -S --needed make diffutils tar curl nasm mingw-w64-x86_64-gcc mingw-w64-x86_64-pkgconf
bash scripts/build-media-tools.sh
```

Then in an x64 Visual Studio developer PowerShell with Python 3.11+ and Rust:

```powershell
./scripts/bundle-windows.ps1
```

`dumpbin` must be on PATH for dependency checks. The packager uses fresh staging,
bundles tools/licenses, creates a ZIP, extracts it to a path with spaces, then
runs the packaged app with a system-only PATH. Native capture and physical
synchronization remain separate hardware checks.

## Corresponding source and licenses

Distributions include `media-tools/sources` and `media-tools/licenses` (under
`Loomik.app/Contents/Resources` on macOS). Archives contain the complete unmodified
source used for the binaries. The recipe, manifest, generated configuration and
compiler information accompany them. FFmpeg with x264 is GPL-2.0-or-later;
applicable LGPL/GPL texts and attribution are included. NVIDIA headers carry
their permissive license in the shipped header/source. Loomik is MIT licensed
and communicates with the separate executables through pipes.

To rebuild from a distribution without downloading the source again, create:

```text
scripts/build-media-tools.sh       ← from media-tools/sources
packaging/media-sources.env        ← from media-tools/sources
packaging/MEDIA-NOTICE.txt         ← from media-tools/sources
target/media-sources/*.tar.*       ← from media-tools/sources
```

Run the included recipe on the target OS. Archives are validated before
extraction; optional codecs from Homebrew/MSYS2 are not automatically linked.
`LOOMIK_FFMPEG` and `LOOMIK_FFPROBE` can select user-supplied replacement tools.

## Package verification

`Loomik --verify-package NEW_DIRECTORY` runs before GUI creation or permission
requests. It insists that tools resolve beside the executable, writes synthetic
red/blue frames and an audio tone in MP4/MOV/MKV, checks H.264/AAC metadata and
duration, decodes every frame, checks colors/audio, and writes `package-check.json`.
It does not record the screen, camera or microphone or establish hardware parity.
