#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
profile="${1:-release}"
if [ "$profile" = "release" ]; then
  cargo build --release --locked
else
  cargo build --locked
fi
bundle="target/$profile/bundle/Loomik.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/loomik" "$bundle/Contents/MacOS/loomik"
cp packaging/Info.plist "$bundle/Contents/Info.plist"
if [ -f packaging/AppIcon.icns ]; then
  cp packaging/AppIcon.icns "$bundle/Contents/Resources/AppIcon.icns"
fi
# Preserve the installed app's identity and privacy grants through the rename.
codesign --force --deep --sign - --identifier com.local-loom.recorder \
  --requirements '=designated => identifier "com.local-loom.recorder"' "$bundle"
printf '%s\n' "$bundle"
