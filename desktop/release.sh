#!/bin/sh
# Releases a new gramm: bumps the version, builds gramm.app and its zip (bundle.sh),
# commits desktop/, pushes, and publishes a GitHub release that Check for Updates finds.
# Usage: desktop/release.sh 0.1.1
set -eu

DESKTOP=$(cd "$(dirname "$0")" && pwd)
TWEB=$(dirname "$DESKTOP")
REPO=arturious/tweb

VERSION=${1:?usage: desktop/release.sh <version>, e.g. 0.1.1}
VERSION=${VERSION#v}
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "✗ version must look like 0.1.1"; exit 1; }

CURRENT=$(plutil -extract version raw "$DESKTOP/tauri.conf.json")
NEWEST=$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | tail -1)
if [ "$VERSION" = "$CURRENT" ] || [ "$NEWEST" != "$VERSION" ]; then
  echo "✗ $VERSION must be newer than the current $CURRENT"
  exit 1
fi
gh auth status >/dev/null 2>&1 || { echo "✗ gh is not signed in: run gh auth login"; exit 1; }

echo "→ $CURRENT → $VERSION"
sed -i '' "s/\"version\": \"$CURRENT\"/\"version\": \"$VERSION\"/" "$DESKTOP/tauri.conf.json"
sed -i '' "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" "$DESKTOP/Cargo.toml"

"$DESKTOP/bundle.sh"

echo "→ git"
git -C "$TWEB" add desktop
git -C "$TWEB" commit -m "gramm $VERSION"
git -C "$TWEB" push origin HEAD

echo "→ GitHub release v$VERSION"
gh release create "v$VERSION" "$DESKTOP/target/gramm-$VERSION.zip" \
  --repo "$REPO" --target "$(git -C "$TWEB" rev-parse HEAD)" --title "gramm $VERSION" \
  --notes "Update from gramm → Check for Updates…, or download gramm-$VERSION.zip, unzip it and move gramm.app to Applications."

echo "✓ gramm $VERSION released: https://github.com/$REPO/releases/tag/v$VERSION"
