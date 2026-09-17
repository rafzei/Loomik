# Security

Loomik records locally, without accounts, uploads, telemetry or an updater.
Screen, camera and microphone access use the operating system's permissions.
Recordings are not encrypted by the app; protect the output folder using OS
permissions and disk encryption when needed.

## Media and recovery files

- Video backgrounds accept local MP4/MOV/MKV/WebM containers. Probing, playback
  and source-audio export restrict FFmpeg input protocols and demuxers, rejecting
  playlists that could read other files. These restrictions are not a sandbox.
- Image dimensions are checked again before decoding, even if a file changes
  after selection. The maximum is 60 megapixels.
- On macOS/Linux, new recovery directories use mode `0700`. On Windows they
  inherit the output folder's ACLs. Recovery files can contain raw microphone
  audio, source paths and incomplete video; failed exports retain them.
- `LOOMIK_FFMPEG`, `LOOMIK_FFPROBE`, adjacent executables and PATH are trusted
  executable sources. Only use media tools and installation folders you trust.

## Dependency checks

The dependency audit workflow checks `Cargo.lock` against the current RustSec
database on dependency changes and weekly. Dependabot proposes Cargo and Actions
updates. Action pins must also match the repository's GitHub allowlist.

```sh
cargo install cargo-audit --version 0.22.2 --locked
cargo audit
```

The 2026-09-17 audit found no RustSec vulnerabilities. It reported
[RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192.html):
the transitive `ttf-parser` 0.25.1 dependency is unmaintained. This advisory is
not suppressed; replacement depends on the GUI/font dependency chain.

FFmpeg is outside Cargo's dependency graph. Its source archives and hashes are
pinned in `packaging/media-sources.env`; check [upstream security fixes](https://ffmpeg.org/security.html)
before each release. Source builds now use 8.0.3. Published Loomik 1.0.0 packages
still contain 8.0.1 and require a new release to receive the fixes. Ubuntu's
FFmpeg security updates come through APT. See [media tools](docs/media-tools.md).

## GitHub settings checked on 2026-09-17

Secret scanning and push protection are enabled. Dependabot alerts, automatic
security updates and private vulnerability reporting are disabled. The active
`main` ruleset has no named required CI checks. These server-side settings are
separate from the versioned workflows and Dependabot configuration.

Review diagnostic logs and performance reports before sharing them. Keep private
recordings, device identifiers, credentials and personal paths out of public issues.
