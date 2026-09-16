#!/usr/bin/env bash
# Run natively on Ubuntu 24.04 amd64. No containers or cross-build assumptions.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo 'Build this package on x86_64 Ubuntu 24.04.' >&2
  exit 1
fi
if [[ "${1:-}" != --skip-build ]]; then
  cargo build --release --locked
fi
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" && -x target/release/loomik ]]
command -v dpkg-deb >/dev/null
command -v dpkg-shlibdeps >/dev/null
mkdir -p target/release/bundle
package_root="$(mktemp -d target/release/bundle/loomik-deb.XXXXXX)"
trap 'rm -rf -- "$package_root"' EXIT
install -Dm755 target/release/loomik "$package_root/usr/bin/loomik"
install -Dm644 packaging/linux/io.github.rafzei.Loomik.desktop "$package_root/usr/share/applications/io.github.rafzei.Loomik.desktop"
install -Dm644 docs/assets/loomik-logo.svg "$package_root/usr/share/icons/hicolor/scalable/apps/loomik.svg"
install -Dm644 LICENSE "$package_root/usr/share/doc/loomik/copyright"
install -Dm644 docs/linux.md "$package_root/usr/share/doc/loomik/README.md"
python3 scripts/rust-notices.py "$package_root/usr/share/doc/loomik/rust-notices"
mkdir -p "$package_root/DEBIAN" "$package_root/debian"
cat > "$package_root/debian/control" <<'CONTROL'
Source: loomik
Section: video
Priority: optional
Maintainer: Loomik contributors <rafzei@users.noreply.github.com>

Package: loomik
Architecture: amd64
Description: Private screen and media recorder
CONTROL
# Resolve linked ABI minimums on the build OS; dlopen libraries are listed below.
dependencies="$(cd "$package_root" && dpkg-shlibdeps -O -eusr/bin/loomik | sed -n 's/^shlibs:Depends=//p')"
[[ -n "$dependencies" ]]
cat > "$package_root/DEBIAN/control" <<CONTROL
Package: loomik
Version: $version
Section: video
Priority: optional
Architecture: amd64
Maintainer: Loomik contributors <rafzei@users.noreply.github.com>
Homepage: https://github.com/rafzei/Loomik
Depends: $dependencies, ffmpeg, libegl1, libgl1, libxkbcommon0, libwayland-client0, libx11-6, libxcursor1, libxi6, libxrandr2, xdg-utils, xdg-desktop-portal
Recommends: xdg-desktop-portal-gnome | xdg-desktop-portal-kde | xdg-desktop-portal-gtk
Description: Private screen and media recorder
 Record an isolated application window or an image/video background with
 a movable camera circle and microphone narration. Saves MP4, MOV and MKV
 locally without an account, upload, or recording-duration limit.
CONTROL
# Debian's build metadata is not part of the installed filesystem.
rm -r -- "$package_root/debian"
artifact="target/release/bundle/Loomik-$version-Ubuntu-24.04-amd64.deb"
dpkg-deb --root-owner-group --build "$package_root" "$artifact"
dpkg-deb --info "$artifact"
(cd target/release/bundle && sha256sum "${artifact##*/}" > "${artifact##*/}.sha256")
printf '%s\n' "$artifact"
