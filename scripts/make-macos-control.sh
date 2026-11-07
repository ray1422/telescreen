#!/bin/sh
# Build the Control Center button (spec UI-10, docs/spikes.md spike 8) into an
# app bundle. scripts/make-macos-app.sh calls this script before it signs the
# app.
#
#   scripts/make-macos-control.sh <Telescreen.app> <version> <signing identity>
#
# The extension needs App Intents metadata (Metadata.appintents). With Xcode,
# this script makes it with appintentsmetadataprocessor, with the arguments
# that xcodebuild uses (swift-build: AppIntentsMetadata.xcspec,
# AppIntentsMetadataCompiler.swift, SwiftCompiler.swift). Without Xcode, it
# copies the hand-written fallback and prints a warning.
#
# TELESCREEN_REQUIRE_XCODE=1 (CI): fail if the Xcode tool is not available.
set -eu

app=$1
version=$2
identity=$3

root=$(cd "$(dirname "$0")/.." && pwd)
control="$root/crates/telescreen-ui-macos/control"
appex="$app/Contents/PlugIns/TelescreenControl.appex"
module=TelescreenControl
bundle_id=dev.telescreen.Telescreen.Control
deployment=27.0
triple="arm64-apple-macos$deployment"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

processor=$(xcrun --find appintentsmetadataprocessor 2>/dev/null || true)
if [ -z "$processor" ] && [ "${TELESCREEN_REQUIRE_XCODE:-}" = 1 ]; then
  echo "TELESCREEN_REQUIRE_XCODE=1, but appintentsmetadataprocessor is not available (no Xcode)" >&2
  exit 1
fi

mkdir -p "$appex/Contents/MacOS" "$appex/Contents/Resources"

# The protocols whose conformances swiftc writes to the const values file
# (SWIFT_EMIT_CONST_VALUE_PROTOCOLS of swift-build, Plugin.swift).
cat >"$work/protocols.json" <<'JSON'
["AppIntent","EntityQuery","AppEntity","TransientEntity","AppEnum","AppShortcutProviding","AppShortcutsProvider","AnyResolverProviding","AppIntentsPackage","DynamicOptionsProvider","_IntentValueRepresentable","_AssistantIntentsProvider","_GenerativeFunctionExtractable","IntentValueQuery","Resolver","AppExtension","ExtensionPointDefining"]
JSON

# The extension must start at _NSExtensionMain. The const values and the
# linker dependency info are inputs of appintentsmetadataprocessor.
swiftc -O -wmo -parse-as-library -application-extension -module-name "$module" \
  -target "$triple" \
  -Xlinker -e -Xlinker _NSExtensionMain \
  -Xlinker -dependency_info -Xlinker "$work/dependency_info.dat" \
  -emit-const-values-path "$work/$module.swiftconstvalues" \
  -Xfrontend -const-gather-protocols-file -Xfrontend "$work/protocols.json" \
  "$control/Control.swift" -o "$appex/Contents/MacOS/$module" 2>"$work/swiftc.log" ||
  {
    cat "$work/swiftc.log" >&2
    exit 1
  }
cat "$work/swiftc.log" >&2

cat >"$appex/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleSupportedPlatforms</key><array><string>MacOSX</string></array>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundleName</key><string>$module</string>
  <key>CFBundleDisplayName</key><string>Telescreen</string>
  <key>CFBundleExecutable</key><string>$module</string>
  <key>CFBundlePackageType</key><string>XPC!</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$(date +%s)</string>
  <key>LSMinimumSystemVersion</key><string>$deployment</string>
  <key>NSExtension</key>
  <dict>
    <key>NSExtensionPointIdentifier</key><string>com.apple.widgetkit-extension</string>
  </dict>
</dict>
</plist>
PLIST

if [ -n "$processor" ]; then
  printf '%s\n' "$control/Control.swift" >"$work/sources.txt"
  printf '%s\n' "$work/$module.swiftconstvalues" >"$work/constvalues.txt"
  : >"$work/metadata.txt"
  : >"$work/static-metadata.txt"
  "$processor" \
    --toolchain-dir "$(xcode-select -p)/Toolchains/XcodeDefault.xctoolchain" \
    --module-name "$module" \
    --sdk-root "$(xcrun --sdk macosx --show-sdk-path)" \
    --xcode-version "$(xcodebuild -version | sed -n 's/^Build version //p')" \
    --platform-family macOS \
    --deployment-target "$deployment" \
    --bundle-identifier "$bundle_id" \
    --output "$appex/Contents/Resources" \
    --target-triple "$triple" \
    --binary-file "$appex/Contents/MacOS/$module" \
    --dependency-file "$work/dependency_info.dat" \
    --stringsdata-file "$work/ExtractedAppShortcutsMetadata.stringsdata" \
    --source-file-list "$work/sources.txt" \
    --metadata-file-list "$work/metadata.txt" \
    --static-metadata-file-list "$work/static-metadata.txt" \
    --swift-const-vals-list "$work/constvalues.txt" \
    --compile-time-extraction \
    --deployment-aware-processing \
    --no-app-shortcuts-localization
  generated="$appex/Contents/Resources/Metadata.appintents/extract.actionsdata"
  if [ ! -f "$generated" ]; then
    echo "appintentsmetadataprocessor wrote no $generated" >&2
    exit 1
  fi
  # The fallback must describe the same actions as the generated file.
  fallback="$control/fallback/Metadata.appintents/extract.actionsdata"
  if ! python3 -I -c '
import json, sys
a, b = (json.load(open(p))["actions"] for p in sys.argv[1:3])
sys.exit(0 if a == b else 1)
' "$generated" "$fallback"; then
    echo "warning: $fallback differs from the generated App Intents metadata; update it" >&2
    if [ -n "${GITHUB_ACTIONS:-}" ]; then
      echo "::warning::the fallback App Intents metadata differs from the generated metadata"
    fi
  fi
else
  echo "warning: no Xcode: using the hand-written App Intents metadata" >&2
  cp -R "$control/fallback/Metadata.appintents" "$appex/Contents/Resources/"
fi

codesign --force --sign "$identity" --entitlements "$control/Control.entitlements" "$appex"
