#!/bin/sh
# Builds gramm.app: Telegram Web (vite build) and the release binary, signed ad hoc,
# and gramm-<version>.zip of it to attach to a GitHub release (Check for Updates reads it).
# Usage: desktop/bundle.sh  →  desktop/target/gramm.app, desktop/target/gramm-<version>.zip
set -eu

DESKTOP=$(cd "$(dirname "$0")" && pwd)
TWEB=$(dirname "$DESKTOP")
APP="$DESKTOP/target/gramm.app"
WEB="$APP/Contents/Resources/tweb"
# The one place the version lives; the app compares it with the release tag
VERSION=$(plutil -extract version raw "$DESKTOP/tauri.conf.json")

echo "→ Telegram Web"
(cd "$TWEB" && pnpm exec vite build)

echo "→ gramm (release)"
(cd "$DESKTOP" && cargo build --release --features custom-protocol)

echo "→ gramm.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$WEB"
cp "$DESKTOP/target/release/tg-desktop" "$APP/Contents/MacOS/gramm"
cp "$DESKTOP/Info.plist" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" -c "Set :CFBundleVersion $VERSION" \
  "$APP/Contents/Info.plist"

# What the build does not copy (copyPublicDir: false): assets, wasm, the voice recorder
# workers. public also holds an old committed build, so its hashed chunks stay out;
# then the build itself goes over it. Source maps are left behind
rsync -a --exclude='*.map' --exclude='*-????????.js' --exclude='*-????????.css' \
  --exclude='index.html' --exclude='snapshot.html' --exclude='.DS_Store' \
  "$TWEB/public/" "$WEB/"
rsync -a --exclude='*.map' --exclude='.DS_Store' "$TWEB/dist/" "$WEB/"

# The icon is an Icon Composer document (icons/gramm.icon, edit it in Icon Composer):
# actool (Xcode) turns it into the Liquid Glass icon macOS 26 renders itself (Assets.car,
# found by CFBundleIconName) plus a flat gramm.icns for older systems
xcrun actool "$DESKTOP/icons/gramm.icon" --compile "$APP/Contents/Resources" \
  --platform macosx --minimum-deployment-target 14.0 --app-icon gramm \
  --output-partial-info-plist "$DESKTOP/target/icon-partial.plist" >/dev/null

# Ad hoc: no Apple account needed, but macOS asks again for camera, microphone and
# notifications after each rebuild, since the signature changes
codesign --force --deep --sign - "$APP"

ZIP="$DESKTOP/target/gramm-$VERSION.zip"
rm -f "$DESKTOP"/target/gramm-*.zip
ditto -c -k --keepParent "$APP" "$ZIP"

echo "✓ $APP ($(du -sh "$APP" | cut -f1))"
echo "✓ $ZIP ($(du -sh "$ZIP" | cut -f1)) — attach to a release tagged v$VERSION"
