#!/bin/sh
# Build a signed Telescreen.app (docs/spec.md DIST-1).
#
#   scripts/make-macos-app.sh [output-dir]      default: ~/Library/Caches/telescreen-dev.noindex
#
# The default is not in target/ and has the .noindex suffix: Spotlight does not
# index it, so the launcher does not show a second Telescreen (DIST-5).
#
# Start it with `open`, so that macOS gives the permissions to the app itself
# (not to the terminal):
#
#   open -a ~/Library/Caches/telescreen-dev.noindex/Telescreen.app
#
# macOS keys Accessibility and Input Monitoring to the code signature. An ad-hoc
# signature changes at each build, so macOS can ask for the permissions again.
# A signing identity keeps the permissions across builds. The script uses the
# identity in TELESCREEN_SIGN_IDENTITY (default "Telescreen Dev") if the keychain
# has it, else an ad-hoc signature. A self-signed code signing certificate is
# sufficient (docs/test-plan.md).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-"$HOME/Library/Caches/telescreen-dev.noindex"}
app="$out/Telescreen.app"

cd "$root"
# TELESCREEN_DEBUG=1 adds the debug protocol (docs/debug-protocol.md). Never
# for a release.
features=
if [ "${TELESCREEN_DEBUG:-}" = 1 ]; then
  features="--features debug-protocol"
fi
# shellcheck disable=SC2086
env -u CARGO_TARGET_DIR cargo build --release -p telescreen-app $features

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/telescreen "$app/Contents/MacOS/telescreen"
# The app icon; tools/make_icon.py makes it from the mockup canvas.
cp assets/macos/Telescreen.icns "$app/Contents/Resources/Telescreen.icns"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>dev.telescreen.Telescreen</string>
  <key>CFBundleName</key><string>Telescreen</string>
  <key>CFBundleExecutable</key><string>telescreen</string>
  <key>CFBundleIconFile</key><string>Telescreen</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>27.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Telescreen connects to your other computers on the local network to forward the keyboard and the mouse.</string>
  <key>NSBonjourServices</key>
  <array><string>_telescreen._udp</string></array>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>dev.telescreen.Telescreen</string>
      <key>CFBundleURLSchemes</key><array><string>telescreen</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
identity=${TELESCREEN_SIGN_IDENTITY:-Telescreen Dev}
if ! security find-identity -p codesigning | grep -qF "\"$identity\""; then
  echo "no signing identity \"$identity\": ad-hoc signature" >&2
  identity=-
fi

# The Control Center button (UI-10, spike 8).
appex="$app/Contents/PlugIns/TelescreenControl.appex"
scripts/make-macos-control.sh "$app" "$version" "$identity"
codesign --force --sign "$identity" "$app"
# A running extension answers with its old code.
pkill -f "$appex/Contents/MacOS/TelescreenControl" || true
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f -R "$app"
echo "built $app"
