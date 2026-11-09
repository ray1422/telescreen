#!/bin/sh
# Run the packaging CLI in a fixture tree; cargo/strip/readelf are stubs.
set -eu

fail() {
  echo "telescreen: package release test: $*" >&2
  exit 1
}

make_fixture() {
  mkdir -p "$fixture/scripts" "$fixture/packaging/linux" "$fixture/assets/linux" "$fixture/examples" "$fixture/contrib" "$fixture/applet" "$fixture/crates/telescreen-ui-linux/qml" "$tools"
  cp "$root/scripts/package-release.sh" "$fixture/scripts/"
  cp "$root/packaging/linux/dev.telescreen.Telescreen.desktop" "$fixture/packaging/linux/"
  cp -R "$root/applet/dev.telescreen.menu" "$fixture/applet/"
  echo license >"$fixture/LICENSE"
  echo readme >"$fixture/README.md"
  echo icon >"$fixture/assets/linux/telescreen.png"
  echo config >"$fixture/examples/config.yaml"
  echo helper >"$fixture/contrib/helper"
  cat >"$fixture/crates/telescreen-ui-linux/qml/App.qml" <<'QML'
import QtQuick
import QtQuick.Controls 6.4 as QQC2
import org.kde.kirigami as Kirigami
import dev.telescreen.ui
QML
  echo 'import QtQuick' >"$fixture/crates/telescreen-ui-linux/qml/Other.qml"
  cat >"$tools/cargo" <<'STUB'
#!/bin/sh
mkdir -p target/release
printf '#!/bin/sh\nexit 0\n' >target/release/telescreen
STUB
  cat >"$tools/strip" <<'STUB'
#!/bin/sh
# Leave a marker so readelf can verify it sees the stripped copy.
printf '# stripped\n' >>"$1"
STUB
  cat >"$tools/readelf" <<'STUB'
#!/bin/sh
case "$(tail -n 1 "$2")" in '# stripped') ;; *) exit 1 ;; esac
case "$1" in
  -d)
    echo ' 0x0000000000000001 (NEEDED) Shared library: [libQt6Core.so.6]'
    echo ' 0x0000000000000001 (NEEDED) Shared library: [libc.so.6]'
    ;;
  -V)
    echo 'Name: GLIBC_2.9 Flags: none'
    echo 'Name: GLIBC_2.34 Flags: none'
    echo 'Name: GLIBC_2.3.4 Flags: none'
    echo 'Name: GLIBC_2.34.1 Flags: none'
    echo 'Name: GLIBC_PRIVATE Flags: none'
    ;;
  *) exit 1 ;;
esac
STUB
  cat >"$tools/uname" <<'STUB'
#!/bin/sh
case "$1" in -s) echo Linux ;; -m) echo x86_64 ;; esac
STUB
  # Only the stubs: chmod follows a symbolic link to a system tool.
  for stub in "$tools/"*; do
    [ -L "$stub" ] || chmod +x "$stub"
  done
}

check_generic() {
  PATH="$tools:$PATH" "$test_shell" "$fixture/scripts/package-release.sh" "$work/generic"
  cat >"$work/expected" <<'DEPS'
lib libQt6Core.so.6
lib libc.so.6
glibc 2.34.1
qml QtQuick
qml QtQuick.Controls
qml org.kde.desktop
qml org.kde.kirigami
DEPS
  cmp "$work/expected" "$work/generic/telescreen-linux-x86_64.deps" || fail 'incorrect manifest'
  python3 - "$work/generic/dev.telescreen.menu.plasmoid" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    names = archive.namelist()
    assert 'metadata.json' in names
    assert 'contents/ui/main.qml' in names
    assert all(name == 'metadata.json' or name.startswith('contents/') for name in names)
PY
  mkdir -p "$work/extracted"
  tar -xzf "$work/generic/telescreen-linux-x86_64.tar.gz" -C "$work/extracted"
  [ -f "$work/extracted/telescreen/applet/dev.telescreen.menu/metadata.json" ] || fail 'archive has no applet'
  [ -f "$work/extracted/telescreen/contrib/helper" ] || fail 'archive has no contrib'
}

check_arch() {
  PATH="$tools:$PATH" TELESCREEN_VARIANT=arch "$test_shell" "$fixture/scripts/package-release.sh" "$work/arch"
  [ -f "$work/arch/telescreen-arch-x86_64.tar.gz" ] || fail 'missing Arch archive'
  [ ! -e "$work/arch/telescreen-linux-x86_64.deps" ] || fail 'Arch generated a generic manifest'
  [ ! -e "$work/arch/dev.telescreen.menu.plasmoid" ] || fail 'Arch generated a plasmoid'
  mkdir -p "$work/extracted-arch"
  tar -xzf "$work/arch/telescreen-arch-x86_64.tar.gz" -C "$work/extracted-arch"
  diff -r "$work/extracted/telescreen" "$work/extracted-arch/telescreen" || fail 'variants have different contents'
}

main() {
  root=$(cd "$(dirname "$0")/../.." && pwd)
  test_shell=/bin/sh
  case "$(ps -p "$$" -o comm= 2>/dev/null || :)" in *dash*) test_shell=/bin/dash ;; esac
  work=$(mktemp -d)
  trap 'rm -rf "$work"' 0
  trap 'exit 1' HUP INT TERM
  fixture=$work/repo
  tools=$work/tools
  unset TELESCREEN_VARIANT
  make_fixture
  check_generic
  check_arch
  echo 'package release tests passed'
}

main "$@"
