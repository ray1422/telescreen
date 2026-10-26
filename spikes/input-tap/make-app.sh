#!/bin/sh
# Build SpikeInputTap.app (ad-hoc signed) so that macOS grants permissions to this
# bundle only, not to the terminal or the agent app.
set -eu
cd "$(dirname "$0")"
env -u CARGO_TARGET_DIR cargo build --release
app=target/SpikeInputTap.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/spike-input-tap "$app/Contents/MacOS/SpikeInputTap"
cat >"$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.telescreen.spike.inputtap</string>
  <key>CFBundleName</key><string>SpikeInputTap</string>
  <key>CFBundleExecutable</key><string>SpikeInputTap</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSUIElement</key><true/>
</dict></plist>
PLIST
codesign --force --sign - "$app"
echo "built $app"
