#!/usr/bin/env bash
# Native macOS or MSYS2 MINGW64 build. No system FFmpeg libraries are packaged.
set -euo pipefail
cd "$(dirname "$0")/.."
repo_root="$PWD"
source packaging/media-sources.env
case "$(uname -s)" in
  Darwin)
    platform="macos-$(uname -m)"
    compiler=clang
    jobs="${LOOMIK_BUILD_JOBS:-$(sysctl -n hw.logicalcpu)}"
    cflags='-O3 -mmacosx-version-min=13.0'
    ldflags='-mmacosx-version-min=13.0'
    ff_options=(--enable-videotoolbox)
    ;;
  MINGW64_NT-*|MSYS_NT-*)
    [[ "${MSYSTEM:-}" == MINGW64 ]] || { echo 'Use the MSYS2 MINGW64 shell.' >&2; exit 1; }
    platform=windows-x64
    compiler=gcc
    jobs="${LOOMIK_BUILD_JOBS:-4}"
    cflags='-O3'
    ldflags='-static -static-libgcc'
    ff_options=(--target-os=mingw32 --arch=x86_64 --enable-ffnvcodec --enable-nvenc)
    ;;
  *) echo 'Build media tools natively on macOS or Windows/MSYS2; Ubuntu uses APT FFmpeg.' >&2; exit 1 ;;
esac
[[ "$jobs" =~ ^[1-9][0-9]*$ ]]
command -v "$compiler" >/dev/null
command -v pkg-config >/dev/null
if [[ "$(uname -m)" == x86_64 ]]; then command -v nasm >/dev/null; fi
archive_dir="$repo_root/target/media-sources"
build_root="$repo_root/target/media-build/$platform"
output="$repo_root/target/media-tools/$platform"
prefix="$build_root/prefix"
mkdir -p "$archive_dir" "$build_root" "$prefix" "$output/bin" "$output/sources" "$output/licenses"
digest() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi; }
download() {
  local filename="$1" url="$2" expected="$3" actual
  if [[ ! -f "$archive_dir/$filename" ]]; then
    curl -fL --retry 3 --output "$archive_dir/$filename.download" "$url"
    mv "$archive_dir/$filename.download" "$archive_dir/$filename"
  fi
  actual="$(digest "$archive_dir/$filename")"
  [[ "${actual%% *}" == "$expected" ]] || { echo "Checksum mismatch: $filename" >&2; exit 1; }
  cp "$archive_dir/$filename" "$output/sources/"
}
download "$LOOMIK_FFMPEG_ARCHIVE" "$LOOMIK_FFMPEG_URL" "$LOOMIK_FFMPEG_SHA256"
download "$LOOMIK_X264_ARCHIVE" "$LOOMIK_X264_URL" "$LOOMIK_X264_SHA256"
if [[ "$platform" == windows-x64 ]]; then
  download "$LOOMIK_NVCODEC_ARCHIVE" "$LOOMIK_NVCODEC_URL" "$LOOMIK_NVCODEC_SHA256"
  tar -xf "$archive_dir/$LOOMIK_NVCODEC_ARCHIVE" -C "$build_root"
  make -C "$build_root/nv-codec-headers-n12.1.14.0" PREFIX="$prefix" install
  # The permissive NVIDIA license is embedded in each distributed header.
  cp "$build_root/nv-codec-headers-n12.1.14.0/include/ffnvcodec/nvEncodeAPI.h" "$output/licenses/nvEncodeAPI.h"
fi
tar -xf "$archive_dir/$LOOMIK_X264_ARCHIVE" -C "$build_root"
tar -xf "$archive_dir/$LOOMIK_FFMPEG_ARCHIVE" -C "$build_root"
# Isolate pkg-config from Homebrew/MSYS2 optional codecs. Only x264 is linked.
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export MACOSX_DEPLOYMENT_TARGET=13.0
(
  cd "$build_root/x264-$LOOMIK_X264_REVISION"
  CC="$compiler" ./configure --prefix="$prefix" --enable-static --enable-pic \
    --disable-cli --disable-opencl --disable-lavf --disable-swscale \
    --extra-cflags="$cflags" --extra-ldflags="$ldflags"
  make -j "$jobs"
  make install-lib-static
)
(
  cd "$build_root/ffmpeg-$LOOMIK_FFMPEG_VERSION"
  ./configure --prefix="$prefix" --cc="$compiler" --disable-autodetect \
    --disable-shared --enable-static --enable-pic --enable-gpl --enable-libx264 \
    --disable-network --disable-doc --disable-debug --disable-ffplay \
    --extra-cflags="$cflags" --extra-ldflags="$ldflags" --pkg-config-flags=--static \
    "${ff_options[@]}"
  make -j "$jobs" ffmpeg ffprobe
  suffix=''; [[ "$platform" != windows-x64 ]] || suffix=.exe
  cp "ffmpeg$suffix" "ffprobe$suffix" "$output/bin/"
  cp config.h config_components.h ffbuild/config.mak "$output/sources/"
  cp COPYING.GPLv2 COPYING.LGPLv2.1 LICENSE.md "$output/licenses/"
)
cp "$build_root/x264-$LOOMIK_X264_REVISION/COPYING" "$output/licenses/x264-COPYING"
cp "$build_root/x264-$LOOMIK_X264_REVISION/config.h" "$output/sources/x264-config.h"
cp "$build_root/x264-$LOOMIK_X264_REVISION/config.mak" "$output/sources/x264-config.mak"
cp scripts/build-media-tools.sh packaging/media-sources.env packaging/MEDIA-NOTICE.txt "$output/sources/"
cp packaging/MEDIA-NOTICE.txt "$output/NOTICE.txt"
"$output/bin/ffmpeg" -version > "$output/build-info.txt"
"$output/bin/ffmpeg" -buildconf >> "$output/build-info.txt" 2>&1
"$compiler" --version >> "$output/build-info.txt"
printf '\nMedia tools: %s\n' "$output"
