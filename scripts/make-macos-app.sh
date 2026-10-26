#!/bin/sh
# Build a signed Telescreen.app (docs/spec.md DIST-1).
#
#   scripts/make-macos-app.sh [output-dir]      default: target/release
#
# Start it with `open`, so that macOS gives the permissions to the app itself
# (not to the terminal):
#
#   open -a target/release/Telescreen.app --args run --config "$PWD/config.yaml" --computer mac
#
# macOS keys Accessibility and Input Monitoring to the code signature. An ad-hoc
# signature changes at each build, so macOS can ask for the permissions again.
# A signing identity keeps the permissions across builds. The script uses the
# identity in TELESCREEN_SIGN_IDENTITY (default "Telescreen Dev") if the keychain
# has it, else an ad-hoc signature. A self-signed code signing certificate is
# sufficient (docs/test-plan.md).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-"$root/target/release"}
app="$out/Telescreen.app"

cd "$root"
env -u CARGO_TARGET_DIR cargo build --release -p telescreen-app

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/telescreen "$app/Contents/MacOS/telescreen"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>dev.telescreen.Telescreen</string>
  <key>CFBundleName</key><string>Telescreen</string>
  <key>CFBundleExecutable</key><string>telescreen</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>27.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Telescreen connects to your other computers on the local network to forward the keyboard and the mouse.</string>
</dict>
</plist>
PLIST
identity=${TELESCREEN_SIGN_IDENTITY:-Telescreen Dev}
if security find-identity -p codesigning | grep -qF "\"$identity\""; then
  codesign --force --sign "$identity" "$app"
else
  echo "no signing identity \"$identity\": ad-hoc signature" >&2
  codesign --force --sign - "$app"
fi
echo "built $app"
