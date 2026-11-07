#!/bin/sh
# Builds the extension and puts it into the dev Telescreen.app, then signs
# the extension and the app again. Run scripts/make-macos-app.sh first.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=${1:-"$HOME/Library/Caches/telescreen-dev.noindex/Telescreen.app"}
identity=${TELESCREEN_SIGN_IDENTITY:-Telescreen Dev}
appex="$app/Contents/PlugIns/TelescreenControl.appex"
rm -rf "$appex"
mkdir -p "$appex/Contents/MacOS"
swiftc -O -parse-as-library -application-extension \
  -target arm64-apple-macos27.0 -module-name TelescreenControl \
  -Xlinker -e -Xlinker _NSExtensionMain \
  "$here/Control.swift" -o "$appex/Contents/MacOS/TelescreenControl"
cp "$here/Info.plist" "$appex/Contents/Info.plist"
# The App Intents metadata, written by hand (no Xcode).
mkdir -p "$appex/Contents/Resources"
cp -R "$here/Metadata.appintents" "$appex/Contents/Resources/"
codesign --force --sign "$identity" --entitlements "$here/Control.entitlements" "$appex"
# The app gets the URL scheme of the button.
/usr/libexec/PlistBuddy -c "Delete :CFBundleURLTypes" "$app/Contents/Info.plist" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :CFBundleURLTypes array" \
  -c "Add :CFBundleURLTypes:0 dict" \
  -c "Add :CFBundleURLTypes:0:CFBundleURLName string dev.telescreen.Telescreen" \
  -c "Add :CFBundleURLTypes:0:CFBundleURLSchemes array" \
  -c "Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string telescreen" \
  "$app/Contents/Info.plist"
codesign --force --sign "$identity" "$app"
echo "built $appex"
