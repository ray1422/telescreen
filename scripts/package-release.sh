#!/bin/sh
# Make the release files of this platform (docs/spec.md DIST-3).
#
#   scripts/package-release.sh [output-dir]      default: target/dist
#
# macOS: telescreen-macos-arm64.tar.gz. The signing rules are in
# scripts/make-macos-app.sh.
# Linux: TELESCREEN_VARIANT selects the archive. generic (default):
# telescreen-linux-x86_64.tar.gz, the dependency manifest
# telescreen-linux-x86_64.deps (DIST-10), and the applet file
# dev.telescreen.menu.plasmoid (DIST-7). arch: telescreen-arch-x86_64.tar.gz
# (DIST-6). Each Linux archive contains the applet directory.
set -eu

fail() {
  echo "telescreen: $*" >&2
  exit 1
}

initialize() {
  root=$(cd "$(dirname "$0")/.." && pwd)
  out=${1:-"$root/target/dist"}
  mkdir -p "$out" || fail "cannot create $out"
  out=$(cd "$out" && pwd)
  cd "$root"
  work=$(mktemp -d) || fail 'cannot create temporary directory'
  trap 'rm -rf "$work"' 0
  trap 'exit 1' HUP INT TERM
  mkdir -p "$work/telescreen/examples"
}

build_linux() {
  variant=${TELESCREEN_VARIANT:-generic}
  case "$variant" in
    generic) name=telescreen-linux-x86_64 ;;
    arch) name=telescreen-arch-x86_64 ;;
    *) fail "unknown TELESCREEN_VARIANT: $variant" ;;
  esac
  env -u CARGO_TARGET_DIR cargo build --release -p telescreen-app || fail 'Linux release build failed'
  cp target/release/telescreen "$work/telescreen/telescreen"
  # Local release builds retain debug info; the distribution does not.
  strip "$work/telescreen/telescreen" || fail 'cannot strip release binary'
  cp packaging/linux/dev.telescreen.Telescreen.desktop "$work/telescreen/"
  cp assets/linux/telescreen.png "$work/telescreen/dev.telescreen.Telescreen.png"
  mkdir -p "$work/telescreen/applet"
  cp -R applet/dev.telescreen.menu "$work/telescreen/applet/"
}

make_manifest() {
  readelf -d "$work/telescreen/telescreen" >"$work/dynamic" || fail 'cannot read binary dependencies'
  readelf -V "$work/telescreen/telescreen" >"$work/versions" || fail 'cannot read binary symbol versions'
  awk '/\(NEEDED\)/ { sub(/.*\[/, ""); sub(/\].*/, ""); print "lib " $0 }' "$work/dynamic" | sort -u >"$work/deps"
  # Ignore GLIBC_PRIVATE and unrelated version namespaces.
  awk '{
    line = $0
    while (match(line, /GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?/)) {
      print substr(line, RSTART + 6, RLENGTH - 6)
      line = substr(line, RSTART + RLENGTH)
    }
  }' "$work/versions" | sort -Vu >"$work/glibc"
  newest=$(tail -n 1 "$work/glibc")
  [ -n "$newest" ] || fail 'binary has no GLIBC version requirement'
  printf 'glibc %s\n' "$newest" >>"$work/deps"
  {
    awk '$1 == "import" && $2 != "dev.telescreen.ui" { print "qml " $2 }' crates/telescreen-ui-linux/qml/*.qml
    echo 'qml org.kde.desktop'
  } | sort -u >>"$work/deps"
  mv "$work/deps" "$out/telescreen-linux-x86_64.deps"
}

make_plasmoid() {
  (cd applet/dev.telescreen.menu && python3 -m zipfile -c "$out/dev.telescreen.menu.plasmoid" metadata.json contents) || fail 'cannot create plasmoid'
}

copy_resources() {
  cp LICENSE README.md "$work/telescreen/"
  cp examples/config.yaml "$work/telescreen/examples/"
  cp -R contrib "$work/telescreen/contrib"
}

main() {
  initialize "$@"
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
      name=telescreen-macos-arm64
      scripts/make-macos-app.sh "$work/telescreen" || fail 'macOS release build failed'
      ;;
    Linux-x86_64)
      build_linux
      if [ "$variant" = generic ]; then
        make_manifest
        make_plasmoid
      fi
      ;;
    *) fail "no release for $(uname -s) $(uname -m)" ;;
  esac
  copy_resources
  tar -czf "$out/$name.tar.gz" -C "$work" telescreen || fail 'cannot create release archive'
  echo "made $out/$name.tar.gz"
}

main "$@"
