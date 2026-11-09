#!/bin/sh
# Exercise the installer through its CLI with a real local release and PATH stubs.
set -eu

fail() {
  echo "telescreen: install test: $*" >&2
  if [ -f "$test_root/output" ]; then cat "$test_root/output" >&2; fi
  exit 1
}

assert_contains() {
  awk -v text="$2" 'index($0, text) { found = 1 } END { exit !found }' "$1" || fail "expected $2 in $1"
}

assert_absent() {
  if awk -v text="$2" 'index($0, text) { found = 1 } END { exit !found }' "$1"; then
    fail "unexpected $2 in $1"
  fi
}

make_tools() {
  tools=$test_root/tools
  mkdir -p "$tools"
  # An isolated PATH makes the absence of sudo, FUSE and optional tools real.
  for utility in awk cat chmod cp curl cut head ln mkdir mktemp mv rm sed sh sort tail tar tr sha256sum shasum unzip python3; do
    found=$(command -v "$utility" 2>/dev/null || :)
    if [ -n "$found" ]; then ln -s "$found" "$tools/$utility"; fi
  done
  real_curl=$(command -v curl)
  export real_curl
  rm "$tools/curl"
  cat >"$tools/curl" <<'STUB'
#!/bin/sh
printf 'curl %s\n' "$*" >>"$T_LOG"
exec "$real_curl" "$@"
STUB
  cat >"$tools/uname" <<'STUB'
#!/bin/sh
printf 'uname %s\n' "$*" >>"$T_LOG"
case "$1" in -s) echo "${T_OS:-Linux}" ;; -m) echo "${T_ARCH:-x86_64}" ;; esac
STUB
  cat >"$tools/id" <<'STUB'
#!/bin/sh
printf 'id %s\n' "$*" >>"$T_LOG"
echo "${T_UID:-1000}"
STUB
  cat >"$tools/pacman" <<'STUB'
#!/bin/sh
printf 'pacman %s\n' "$*" >>"$T_LOG"
case "$1" in
  -Q) [ "${T_CURRENT_NATIVE:-0}" = 1 ] ;;
  -T)
    if [ -n "${T_MISSING_PACKAGE:-}" ]; then echo "$T_MISSING_PACKAGE"; exit 127; fi
    ;;
  -U)
    if [ "${T_PACMAN_FAIL:-0}" = 1 ]; then exit 1; fi
    case "$*" in *--noconfirm*) ;; *) IFS= read -r reply || exit 1; echo "pacman answer $reply" >>"$T_LOG" ;; esac
    ;;
esac
STUB
  cat >"$tools/ldconfig" <<'STUB'
#!/bin/sh
printf 'ldconfig %s\n' "$*" >>"$T_LOG"
for library in libQt6Core.so.6 libGL.so.1 libEGL.so.1 libfontconfig.so.1 libfreetype.so.6 libxkbcommon.so.0 libdbus-1.so.3 libglib-2.0.so.0; do
  if [ "$library" != "${T_MISSING_LIB:-}" ]; then echo " $library (libc6,x86-64) => /lib/$library"; fi
done
STUB
  cat >"$tools/ldd" <<'STUB'
#!/bin/sh
printf 'ldd %s\n' "$*" >>"$T_LOG"
if [ "${T_LDD_FAIL:-0}" = 1 ]; then echo 'liblate.so => not found'; exit 1; fi
echo 'libQt6Core.so.6 => /lib/libQt6Core.so.6'
STUB
  cat >"$tools/getconf" <<'STUB'
#!/bin/sh
printf 'getconf %s\n' "$*" >>"$T_LOG"
echo "glibc ${T_GLIBC:-2.39}"
STUB
  cat >"$tools/qtpaths6" <<'STUB'
#!/bin/sh
printf 'qtpaths6 %s\n' "$*" >>"$T_LOG"
echo "$T_QML"
STUB
  cat >"$tools/pgrep" <<'STUB'
#!/bin/sh
printf 'pgrep %s\n' "$*" >>"$T_LOG"
case "$2" in plasmashell) exit 0 ;; telescreen) [ "${T_RUNNING:-0}" = 1 ] ;; esac
STUB
  chmod +x "$tools/"*
}

make_sudo() {
  cat >"$tools/sudo" <<'STUB'
#!/bin/sh
printf 'sudo %s\n' "$*" >>"$T_LOG"
exec "$@"
STUB
  chmod +x "$tools/sudo"
}

make_release() {
  release=$test_root/release
  source=$test_root/source/telescreen
  mkdir -p "$release" "$source/examples" "$source/contrib" "$source/applet"
  cp -R "$root/applet/dev.telescreen.menu" "$source/applet/"
  cp "$root/packaging/linux/dev.telescreen.Telescreen.desktop" "$source/"
  echo icon >"$source/dev.telescreen.Telescreen.png"
  echo config >"$source/examples/config.yaml"
  echo helper >"$source/contrib/helper"
  cat >"$source/telescreen" <<'BINARY'
#!/bin/sh
printf 'binary %s platform=%s\n' "$*" "${QT_QPA_PLATFORM:-}" >>"$T_LOG"
[ "$1" = self-check ] && [ "${T_SELF_FAIL:-0}" = 0 ]
BINARY
  chmod +x "$source/telescreen"
  tar -czf "$release/telescreen-linux-x86_64.tar.gz" -C "$test_root/source" telescreen
  cp "$release/telescreen-linux-x86_64.tar.gz" "$release/telescreen-arch-x86_64.tar.gz"
  echo package >"$release/telescreen-arch-x86_64.pkg.tar.zst"
  cat >"$release/telescreen-linux-x86_64.deps" <<'DEPS'
lib libQt6Core.so.6
glibc 2.34
qml QtQuick
qml org.kde.desktop
DEPS
  cat >"$release/Telescreen-x86_64.AppImage" <<'APPIMAGE'
#!/bin/sh
printf 'appimage %s\n' "$*" >>"$T_LOG"
[ "$1" = --appimage-extract ] || exit 1
case "$2" in
  usr/share/applications/dev.telescreen.Telescreen.desktop)
    mkdir -p squashfs-root/usr/share/applications
    cat >"squashfs-root/$2" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Telescreen
Comment=Control computers that share a display with one keyboard and mouse
Exec=telescreen
Terminal=false
Categories=Utility;
Icon=dev.telescreen.Telescreen
StartupWMClass=dev.telescreen.Telescreen
DESKTOP
    ;;
  usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png)
    mkdir -p squashfs-root/usr/share/icons/hicolor/256x256/apps
    echo icon >"squashfs-root/$2"
    ;;
  *) exit 1 ;;
esac
APPIMAGE
  (cd "$root/applet/dev.telescreen.menu" && python3 -m zipfile -c "$release/dev.telescreen.menu.plasmoid" metadata.json contents)
  make_checksums
}

make_checksums() {
  (cd "$release" && if command -v sha256sum >/dev/null 2>&1; then
    sha256sum telescreen-* Telescreen-* dev.telescreen.menu.plasmoid
  else
    shasum -a 256 telescreen-* Telescreen-* dev.telescreen.menu.plasmoid
  fi) >"$release/sha256sums.txt"
}

reset_case() {
  rm -rf "$test_root/prefix"
  rm -f "$tools/sudo"
  : >"$T_LOG"
  : >"$test_root/output"
  printf 'ID=other\n' >"$TELESCREEN_OS_RELEASE"
  printf 'yes\nyes\nyes\n' >"$test_root/tty"
  TELESCREEN_TTY=$test_root/tty
  T_MISSING_LIB='' T_MISSING_PACKAGE='' T_GLIBC=2.39 T_LDD_FAIL=0 T_SELF_FAIL=0
  T_CURRENT_NATIVE=0 T_UID=1000 T_PACMAN_FAIL=0 T_RUNNING=0 T_OS=Linux T_ARCH=x86_64
  TELESCREEN_PREFIX=$test_root/prefix
  export TELESCREEN_PREFIX TELESCREEN_TTY T_MISSING_LIB T_MISSING_PACKAGE T_GLIBC T_LDD_FAIL T_SELF_FAIL T_CURRENT_NATIVE T_UID T_PACMAN_FAIL T_RUNNING T_OS T_ARCH
}

run_ok() {
  PATH=$tools "$test_shell" "$root/scripts/install.sh" "$@" >"$test_root/output" 2>&1 || fail "install failed: $*"
}

run_bad() {
  if PATH=$tools "$test_shell" "$root/scripts/install.sh" "$@" >"$test_root/output" 2>&1; then fail "install unexpectedly passed: $*"; fi
}

assert_user_install() {
  method=$(cat "$TELESCREEN_PREFIX/.local/share/telescreen/install-method")
  [ "$method" = "$1" ] || fail "expected $1, got $method"
  [ -x "$TELESCREEN_PREFIX/.local/bin/telescreen" ] || fail 'missing binary'
  [ -f "$TELESCREEN_PREFIX/.local/share/plasma/plasmoids/dev.telescreen.menu/metadata.json" ] || fail 'missing applet'
  [ -f "$TELESCREEN_PREFIX/.local/share/applications/dev.telescreen.Telescreen.desktop" ] || fail 'missing launcher'
  [ -f "$TELESCREEN_PREFIX/.local/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png" ] || fail 'missing icon'
  assert_contains "$test_root/output" 'systemctl --user restart plasma-plasmashell'
}

assert_no_install() {
  [ ! -e "$TELESCREEN_PREFIX/.local" ] || fail 'unexpected user installation'
  assert_absent "$T_LOG" 'pacman -U'
}

arch_case() {
  printf 'ID=derivative\nID_LIKE="other arch"\n' >"$TELESCREEN_OS_RELEASE"
}

test_native() {
  reset_case
  arch_case
  make_sudo
  run_ok
  assert_contains "$T_LOG" 'sudo pacman -U --needed '
  assert_absent "$T_LOG" --noconfirm
  assert_contains "$T_LOG" 'pacman answer yes'
  reset_case
  arch_case
  make_sudo
  run_ok --yes
  assert_contains "$T_LOG" 'sudo pacman -U --needed --noconfirm '
  reset_case
  arch_case
  T_UID=0
  run_ok --yes
  assert_contains "$T_LOG" 'pacman -U --needed --noconfirm '
  assert_absent "$T_LOG" 'sudo pacman'
  reset_case
  arch_case
  make_sudo
  TELESCREEN_TTY=$test_root/no-tty
  run_bad
  assert_contains "$test_root/output" --yes
  assert_contains "$test_root/output" 'Command not run:'
  assert_no_install
}

test_native_user() {
  reset_case
  arch_case
  run_ok --yes
  assert_user_install native-user
  assert_contains "$T_LOG" 'pacman -T qt6-base qt6-declarative qt6-wayland'
  assert_contains "$T_LOG" '/telescreen-arch-x86_64.tar.gz'
  reset_case
  arch_case
  T_MISSING_PACKAGE=qt6-base
  run_bad --native --user --yes
  assert_contains "$test_root/output" 'sudo pacman -S --needed qt6-base'
  assert_no_install
  run_ok --yes
  assert_user_install generic
  reset_case
  arch_case
  make_sudo
  run_ok --user --yes
  assert_user_install native-user
  assert_absent "$T_LOG" 'sudo pacman -U'
}

test_generic() {
  reset_case
  run_ok --yes
  assert_user_install generic
  assert_contains "$T_LOG" 'binary self-check platform=offscreen'
  count=$(awk '/curl .*telescreen-linux-x86_64.tar.gz/ { n++ } END { print n+0 }' "$T_LOG")
  [ "$count" = 1 ] || fail 'generic archive downloaded more than once'
  [ -f "$TELESCREEN_PREFIX/.local/share/telescreen/examples/config.yaml" ] || fail 'missing examples'
  reset_case
  T_MISSING_LIB=libQt6Core.so.6
  run_ok --yes
  assert_user_install appimage
  assert_absent "$T_LOG" '/telescreen-linux-x86_64.tar.gz'
  assert_contains "$TELESCREEN_PREFIX/.local/share/applications/dev.telescreen.Telescreen.desktop" 'Exec=env APPIMAGE_EXTRACT_AND_RUN=1 '
  reset_case
  T_MISSING_LIB=libQt6Core.so.6
  run_bad --generic
  assert_contains "$test_root/output" 'missing library libQt6Core.so.6'
  assert_no_install
  reset_case
  T_LDD_FAIL=1
  run_bad --generic
  assert_contains "$test_root/output" 'liblate.so => not found'
  assert_no_install
  reset_case
  T_SELF_FAIL=1
  run_bad --generic
  assert_contains "$test_root/output" 'offscreen self-check failed'
  assert_no_install
  reset_case
  TELESCREEN_PREFIX="$test_root/prefix with spaces"
  run_ok --generic --yes
  assert_user_install generic
  assert_contains "$TELESCREEN_PREFIX/.local/share/applications/dev.telescreen.Telescreen.desktop" "Exec=\"$TELESCREEN_PREFIX/.local/bin/telescreen\""
  rm -rf "$TELESCREEN_PREFIX"
}

test_all_failed() {
  reset_case
  T_GLIBC=2.20
  printf 'appimage\n' >"$TELESCREEN_TTY"
  run_ok
  assert_contains "$test_root/output" 'glibc 2.20 is older'
  assert_user_install appimage
  reset_case
  T_GLIBC=2.20
  TELESCREEN_TTY=$test_root/no-tty
  run_bad
  assert_contains "$test_root/output" 'select a method with --native, --generic, or --appimage'
  assert_no_install
  reset_case
  T_GLIBC=2.20
  run_bad --yes
  assert_no_install
  reset_case
  T_GLIBC=2.20
  printf 'generic\n' >"$TELESCREEN_TTY"
  run_ok
  assert_user_install generic
  assert_absent "$T_LOG" 'binary self-check'
}

test_appimage_unpackers() {
  reset_case
  if [ -e "$tools/unzip" ]; then
    mv "$tools/unzip" "$test_root/unzip"
    run_ok --appimage --yes
    assert_user_install appimage
    mv "$test_root/unzip" "$tools/unzip"
  fi
  reset_case
  if [ -e "$tools/unzip" ]; then mv "$tools/unzip" "$test_root/unzip"; fi
  mv "$tools/python3" "$test_root/python3"
  run_ok --appimage --yes
  assert_contains "$test_root/output" 'skipping applet'
  [ -f "$TELESCREEN_PREFIX/.local/share/telescreen/install-method" ] || fail 'missing method when applet is skipped'
  [ ! -e "$TELESCREEN_PREFIX/.local/share/plasma/plasmoids/dev.telescreen.menu" ] || fail 'unexpected skipped applet'
  mv "$test_root/python3" "$tools/python3"
  if [ -e "$test_root/unzip" ]; then mv "$test_root/unzip" "$tools/unzip"; fi
}

test_change() {
  reset_case
  run_ok --appimage --yes
  : >"$T_LOG"
  printf 'no\n' >"$TELESCREEN_TTY"
  run_ok
  assert_user_install appimage
  assert_contains "$test_root/output" 'Change the current appimage install to generic?'
  reset_case
  run_ok --appimage --yes
  TELESCREEN_TTY=$test_root/no-tty
  run_bad
  method=$(cat "$TELESCREEN_PREFIX/.local/share/telescreen/install-method")
  [ "$method" = appimage ] || fail 'changed installation without a TTY'
  run_ok --yes
  assert_user_install generic
}

test_cleanup() {
  reset_case
  run_ok --generic --yes
  mkdir -p "$TELESCREEN_PREFIX/.config/telescreen" "$TELESCREEN_PREFIX/.local/share/other"
  echo keep >"$TELESCREEN_PREFIX/.config/telescreen/config.yaml"
  echo keep >"$TELESCREEN_PREFIX/.local/share/other/keep"
  echo keep >"$TELESCREEN_PREFIX/.local/bin/other"
  arch_case
  make_sudo
  printf 'yes\nyes\nyes\n' >"$TELESCREEN_TTY"
  run_ok
  assert_contains "$test_root/output" 'Remove the old user install files'
  for removed in .local/bin/telescreen .local/share/applications/dev.telescreen.Telescreen.desktop .local/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png .local/share/plasma/plasmoids/dev.telescreen.menu .local/share/telescreen; do
    [ ! -e "$TELESCREEN_PREFIX/$removed" ] || fail "did not remove $removed"
    assert_contains "$test_root/output" "Removed $TELESCREEN_PREFIX/$removed"
  done
  for kept in .config/telescreen/config.yaml .local/bin/other .local/share/other/keep; do
    [ -f "$TELESCREEN_PREFIX/$kept" ] || fail "removed $kept"
  done
  reset_case
  run_ok --generic --yes
  arch_case
  make_sudo
  run_ok --native --yes
  [ ! -e "$TELESCREEN_PREFIX/.local/bin/telescreen" ] || fail '--yes did not clean up'
}

test_errors() {
  reset_case
  run_bad --native --generic
  assert_no_install
  run_bad --generic --user
  assert_no_install
  # A CLI method overrides TELESCREEN_METHOD.
  TELESCREEN_METHOD=appimage
  export TELESCREEN_METHOD
  run_ok --generic --yes
  assert_user_install generic
  unset TELESCREEN_METHOD
  reset_case
  cp "$release/sha256sums.txt" "$test_root/checksums"
  echo corrupt >>"$release/telescreen-linux-x86_64.deps"
  run_bad --generic --yes
  assert_contains "$test_root/output" 'checksum failed'
  assert_no_install
  mv "$test_root/checksums" "$release/sha256sums.txt"
  # Restore the original manifest for a binary-checksum failure as well.
  sed '$d' "$release/telescreen-linux-x86_64.deps" >"$test_root/deps"
  mv "$test_root/deps" "$release/telescreen-linux-x86_64.deps"
  cp "$release/telescreen-linux-x86_64.tar.gz" "$test_root/archive"
  echo corrupt >>"$release/telescreen-linux-x86_64.tar.gz"
  run_bad --generic --yes
  assert_contains "$test_root/output" 'checksum failed for telescreen-linux-x86_64.tar.gz'
  assert_no_install
  mv "$test_root/archive" "$release/telescreen-linux-x86_64.tar.gz"
  reset_case
  T_OS=Darwin T_ARCH=arm64
  run_bad --generic
  assert_contains "$test_root/output" 'not supported on macOS'
  assert_no_install
}

main() {
  root=$(cd "$(dirname "$0")/../.." && pwd)
  # Run the child installer under the same interpreter as this test invocation.
  test_shell=$(command -v sh)
  if [ -n "${BASH_VERSION:-}" ]; then test_shell=/bin/sh; fi
  case "$(ps -p "$$" -o comm= 2>/dev/null || :)" in *dash*) test_shell=/bin/dash ;; esac
  test_root=$(mktemp -d)
  trap 'rm -rf "$test_root"' 0
  trap 'exit 1' HUP INT TERM
  T_LOG=$test_root/log
  T_QML=$test_root/qml
  TELESCREEN_OS_RELEASE=$test_root/os-release
  TELESCREEN_BASE_URL=file://$test_root/release
  export T_LOG T_QML TELESCREEN_OS_RELEASE TELESCREEN_BASE_URL
  unset TELESCREEN_METHOD TELESCREEN_USER TELESCREEN_YES TELESCREEN_VERSION
  mkdir -p "$T_QML/QtQuick" "$T_QML/org/kde/desktop"
  : >"$T_QML/QtQuick/qmldir"
  : >"$T_QML/org/kde/desktop/qmldir"
  make_tools
  make_release
  test_native
  test_native_user
  test_generic
  test_all_failed
  test_appimage_unpackers
  test_change
  test_cleanup
  test_errors
  echo 'install tests passed'
}

main "$@"
