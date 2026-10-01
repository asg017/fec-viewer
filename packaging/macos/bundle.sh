#!/usr/bin/env bash
# Build "FEC Viewer.app" (and a .dmg) with cargo-bundle, ad-hoc sign it, and
# register it with LaunchServices so double-clicking a .fec file in Finder
# opens it. Needs `cargo install cargo-bundle`. Undo the registration with:
#   lsregister -u "target/release/bundle/osx/FEC Viewer.app"
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo bundle --release
APP="target/release/bundle/osx/FEC Viewer.app"
codesign --force --sign - "$APP"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP"
echo "Built $APP"
