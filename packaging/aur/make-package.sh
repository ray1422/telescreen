#!/bin/sh
# Build the native Arch package from its release archive, as a non-root user.
set -eu

fail() {
  echo "telescreen: $*" >&2
  exit 1
}

initialize() {
  [ "$#" = 3 ] || fail 'usage: make-package.sh <arch-archive> <version> <output-dir>'
  [ "$(id -u)" != 0 ] || fail 'makepkg must run as a non-root user'
  [ -r /etc/arch-release ] || fail 'make-package.sh must run on Arch Linux'
  template=$(cd "$(dirname "$0")" && pwd)/PKGBUILD.in
  archive=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
  [ -f "$archive" ] || fail "archive not found: $archive"
  version=$2
  case "$version" in ''|*[!0-9A-Za-z.+_-]*) fail "invalid version: $version" ;; esac
  mkdir -p "$3" || fail "cannot create $3"
  out=$(cd "$3" && pwd)
  work=$(mktemp -d) || fail 'cannot create temporary directory'
  trap 'rm -rf "$work"' 0
  trap 'exit 1' HUP INT TERM
}

write_pkgbuild() {
  qt=$(pacman -Q qt6-base) || fail 'qt6-base is not installed'
  qt_min=$(printf '%s\n' "$qt" | awk '{
    sub(/^.*:/, "", $2)
    split($2, v, ".")
    if (v[1] ~ /^[0-9]+$/ && v[2] ~ /^[0-9]+$/) print v[1] "." v[2]
  }')
  [ -n "$qt_min" ] || fail "cannot read Qt major/minor from $qt"
  sha=$(sha256sum "$archive") || fail 'cannot checksum Arch archive'
  sha=${sha%% *}
  sed -e "s/@VERSION@/$version/g" -e "s/@SHA256@/$sha/g" -e "s/@QT_MIN@/$qt_min/g" "$template" >"$out/PKGBUILD" || fail 'cannot write PKGBUILD'
}

# The archive gets the local source name of the PKGBUILD, so that makepkg does
# not download it. Only the PKGBUILD and the package go to the output directory.
build_package() {
  cp "$out/PKGBUILD" "$work/" || fail 'cannot stage PKGBUILD'
  cp "$archive" "$work/telescreen-$version-arch-x86_64.tar.gz" || fail 'cannot stage source archive'
  (cd "$work" && makepkg --nodeps --clean --force) || fail 'makepkg failed'
  set -- "$work"/telescreen-bin-*.pkg.tar.zst
  [ "$#" = 1 ] && [ -f "$1" ] || fail 'makepkg did not produce exactly one telescreen-bin package'
  cp "$1" "$out/telescreen-arch-x86_64.pkg.tar.zst" || fail 'cannot copy native package'
}

main() {
  initialize "$@"
  write_pkgbuild
  build_package
}

main "$@"
