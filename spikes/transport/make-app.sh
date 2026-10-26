#!/bin/sh
# Build SpikeTransport.app (ad-hoc signed). Pass "lan" to add NSLocalNetworkUsageDescription.
set -eu
cd "$(dirname "$0")"
env -u CARGO_TARGET_DIR cargo build --release
app=target/SpikeTransport.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/spike-transport "$app/Contents/MacOS/SpikeTransport"
lan=""
[ "${1:-}" = lan ] && lan='<key>NSLocalNetworkUsageDescription</key><string>Telescreen connects to the computers in your config on the local network.</string>'
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.telescreen.spike.transport</string>
  <key>CFBundleName</key><string>SpikeTransport</string>
  <key>CFBundleExecutable</key><string>SpikeTransport</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSUIElement</key><true/>
  $lan
</dict></plist>
PLIST
codesign --force --sign - "$app"
echo "built $app (${1:-no-lan})"
