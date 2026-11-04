#!/bin/sh
# Make the release archive of this platform (docs/spec.md DIST-3).
#
#   scripts/package-release.sh [output-dir]      default: target/dist
#
# macOS: telescreen-macos-arm64.tar.gz. Linux: telescreen-linux-x86_64.tar.gz.
# The macOS signing rules are in scripts/make-macos-app.sh.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-"$root/target/dist"}
cd "$root"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$out" "$work/telescreen/examples"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    name=telescreen-macos-arm64
    scripts/make-macos-app.sh "$work/telescreen"
    ;;
  Linux-x86_64)
    name=telescreen-linux-x86_64
    env -u CARGO_TARGET_DIR cargo build --release -p telescreen-app
    cp target/release/telescreen "$work/telescreen/telescreen"
    ;;
  *)
    echo "no release for $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

cp LICENSE README.md "$work/telescreen/"
cp examples/config.yaml "$work/telescreen/examples/"
cp -R contrib "$work/telescreen/contrib"
tar -czf "$out/$name.tar.gz" -C "$work" telescreen
echo "made $out/$name.tar.gz"
