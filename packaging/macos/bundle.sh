#!/usr/bin/env bash
# Build "FEC Viewer.app" and register it with LaunchServices so double-clicking
# a .fec file in Finder opens it. Undo with:
#   lsregister -u "target/release/FEC Viewer.app"
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build --release
APP="target/release/FEC Viewer.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp target/release/fec-viewer "$APP/Contents/MacOS/fec-viewer"
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP"
echo "Built $APP"
