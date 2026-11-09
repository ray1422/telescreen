#!/bin/sh
# Install a verified release (DIST-3, DIST-4, DIST-7 through DIST-11).
# curl -fsSL https://github.com/ray1422/telescreen/releases/latest/download/install.sh | sh
# TELESCREEN_VERSION, TELESCREEN_BASE_URL and TELESCREEN_PREFIX select the release/root.
# TELESCREEN_METHOD, TELESCREEN_USER=1 and TELESCREEN_YES=1 supply option defaults.
set -eu

fail() {
  echo "telescreen: $*" >&2
  exit 1
}

usage() {
  cat <<'HELP'
Usage: install.sh [--native | --generic | --appimage] [--user] [--yes] [--help]
  --native    Install the Arch package (requires root or sudo).
  --user      Use the Arch archive for a native user install.
  --generic   Install the generic Linux archive for the current user.
  --appimage  Install the AppImage for the current user.
  --yes       Accept changes and removals; pass --noconfirm to pacman.
Environment: TELESCREEN_METHOD, TELESCREEN_USER=1, TELESCREEN_YES=1,
             TELESCREEN_VERSION, TELESCREEN_BASE_URL, TELESCREEN_PREFIX.
HELP
}

parse_options() {
  cli_method=
  user=${TELESCREEN_USER:-0}
  yes=${TELESCREEN_YES:-0}
  for option in "$@"; do
    case "$option" in
      --native|--generic|--appimage)
        next=${option#--}
        [ -z "$cli_method" ] || [ "$cli_method" = "$next" ] || fail 'two different methods were requested'
        cli_method=$next
        ;;
      --user) user=1 ;;
      --yes) yes=1 ;;
      --help) usage; exit 0 ;;
      *) fail "unknown option: $option (use --help)" ;;
    esac
  done
  requested=${cli_method:-${TELESCREEN_METHOD:-}}
  case "$requested" in ''|native|generic|appimage) ;; *) fail "invalid TELESCREEN_METHOD: $requested" ;; esac
  if [ "$user" = 1 ]; then
    case "$requested" in
      generic|appimage) fail '--user is only valid with the native method' ;;
    esac
  fi
}

initialize() {
  repo=https://github.com/ray1422/telescreen
  base=${TELESCREEN_BASE_URL:-}
  if [ -z "$base" ]; then
    if [ -n "${TELESCREEN_VERSION:-}" ]; then
      base=$repo/releases/download/$TELESCREEN_VERSION
    else
      base=$repo/releases/latest/download
    fi
  fi
  prefix=${TELESCREEN_PREFIX:-$HOME}
  # Launcher Exec paths must be absolute, including with a relative prefix.
  case "$prefix" in /*) ;; *) prefix=$PWD/$prefix ;; esac
  bin=$prefix/.local/bin
  share=$prefix/.local/share/telescreen
  apps=$prefix/.local/share/applications
  desktop=$apps/dev.telescreen.Telescreen.desktop
  icons=$prefix/.local/share/icons/hicolor/256x256/apps
  icon=$icons/dev.telescreen.Telescreen.png
  applet=$prefix/.local/share/plasma/plasmoids/dev.telescreen.menu
  tty=${TELESCREEN_TTY:-/dev/tty}
  tty_open=0
  applet_installed=0
  os=$(uname -s)
  arch=$(uname -m)
  case "$os-$arch" in
    Darwin-arm64)
      [ -z "$requested" ] && [ "$user" != 1 ] || fail 'Linux install methods and --user are not supported on macOS'
      ;;
    Linux-x86_64) ;;
    *) fail "no release for $os $arch. Build from source." ;;
  esac
  work=$(mktemp -d) || fail 'cannot create a temporary directory'
  trap 'rm -rf "$work"' 0
  trap 'exit 1' HUP INT TERM
}

# Cache only verified files. A corrupt download is fatal, including during selection.
download() {
  [ ! -f "$work/$1" ] || return 0
  if [ ! -f "$work/sha256sums.txt" ]; then
    curl -fsSL "$base/sha256sums.txt" -o "$work/sha256sums.txt" || fail 'cannot download sha256sums.txt'
  fi
  curl -fsSL "$base/$1" -o "$work/download" || fail "cannot download $1"
  expected=$(awk -v name="$1" '$2 == name || $2 == "*" name { print $1 }' "$work/sha256sums.txt")
  [ -n "$expected" ] || fail "sha256sums.txt has no entry for $1"
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$work/download") || fail "cannot checksum $1"
  else
    actual=$(shasum -a 256 "$work/download") || fail "cannot checksum $1"
  fi
  actual=${actual%% *}
  [ "$actual" = "$expected" ] || fail "checksum failed for $1"
  mv "$work/download" "$work/$1" || fail "cannot keep $1"
}

extract_archive() {
  download "$1"
  mkdir -p "$work/$2" || fail 'cannot create extraction directory'
  tar -xzf "$work/$1" -C "$work/$2" || fail "cannot extract $1"
  source_dir=$work/$2/telescreen
}

reason() {
  printf 'telescreen: %s: %s\n' "$checking" "$*" >>"$work/reasons-$checking"
}

is_arch() {
  [ -r "${TELESCREEN_OS_RELEASE:-/etc/os-release}" ] || return 1
  # Read just the two fields; do not execute os-release as shell code.
  awk -F= '
    $1 == "ID" || $1 == "ID_LIKE" {
      value = substr($0, index($0, "=") + 1)
      gsub(/[\047\042]/, "", value)
      if ($1 == "ID" && value == "arch") found = 1
      if ($1 == "ID_LIKE") {
        n = split(value, words, /[ \t]+/)
        for (i = 1; i <= n; i++) if (words[i] == "arch") found = 1
      }
    }
    END { exit !found }
  ' "${TELESCREEN_OS_RELEASE:-/etc/os-release}"
}

check_native() {
  is_arch || reason 'requires Arch Linux (ID=arch or arch in ID_LIKE)'
  if ! command -v pacman >/dev/null 2>&1; then
    reason 'pacman is not available'
  elif [ "$checking" = native-user ]; then
    if ! missing=$(pacman -T qt6-base qt6-declarative qt6-wayland kirigami qqc2-desktop-style libxkbcommon wayland 2>/dev/null); then
      [ -n "$missing" ] || reason 'cannot query native dependencies with pacman -T'
    fi
    if [ -n "$missing" ]; then
      packages=$(printf '%s\n' "$missing" | tr '\n' ' ')
      printf '%s\n' "$missing" >"$work/missing-packages"
      while IFS= read -r package; do
        reason "missing package $package; run: sudo pacman -S --needed ${packages% }"
      done <"$work/missing-packages"
    fi
  fi
  if [ "$checking" = native ] && [ "$(id -u)" != 0 ] && ! command -v sudo >/dev/null 2>&1; then
    reason 'requires root, sudo, or --native --user'
  fi
}

read_libraries() {
  : >"$work/libraries"
  for loader in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    if command -v "$loader" >/dev/null 2>&1 && "$loader" -p >"$work/libraries" 2>/dev/null; then
      return 0
    fi
  done
}

has_library() {
  awk -v name="$1" '$1 == name { found = 1 } END { exit !found }' "$work/libraries"
}

check_glibc() {
  installed=$(getconf GNU_LIBC_VERSION 2>/dev/null) || installed=
  case "$installed" in
    'glibc '*) installed=${installed#glibc } ;;
    *) reason "cannot determine glibc (requires $1 or newer)"; return 0 ;;
  esac
  newest=$(printf '%s\n%s\n' "$1" "$installed" | sort -V | tail -n 1)
  [ "$newest" = "$installed" ] || reason "glibc $installed is older than $1"
}

qml_directories() {
  queried=
  if command -v qtpaths6 >/dev/null 2>&1; then
    queried=$(qtpaths6 --query QT_INSTALL_QML 2>/dev/null) || queried=
  fi
  if [ -z "$queried" ] && command -v qtpaths >/dev/null 2>&1; then
    queried=$(qtpaths --query QT_INSTALL_QML 2>/dev/null) || queried=
  fi
  if [ -z "$queried" ] && command -v qmake6 >/dev/null 2>&1; then
    queried=$(qmake6 -query QT_INSTALL_QML 2>/dev/null) || queried=
  fi
  printf '%s\n' "$queried"
  printf '%s\n' "${QML_IMPORT_PATH:-}" | tr ':' '\n'
  printf '%s\n' /usr/lib/qt6/qml /usr/lib/x86_64-linux-gnu/qt6/qml /usr/lib64/qt6/qml
}

check_qml() {
  module_path=$(printf '%s' "$1" | tr '.' '/')
  while IFS= read -r qml_dir; do
    if [ -n "$qml_dir" ] && [ -f "$qml_dir/$module_path/qmldir" ]; then
      return 0
    fi
  done <"$work/qml-directories"
  reason "missing QML module $1"
}

check_generic_manifest() {
  download telescreen-linux-x86_64.deps
  read_libraries
  qml_directories >"$work/qml-directories"
  while IFS=' ' read -r kind value; do
    case "$kind" in
      lib) has_library "$value" || reason "missing library $value" ;;
      glibc) check_glibc "$value" ;;
      qml) check_qml "$value" ;;
      *) reason "invalid dependency manifest line: $kind $value" ;;
    esac
  done <"$work/telescreen-linux-x86_64.deps"
}

check_generic_binary() {
  extract_archive telescreen-linux-x86_64.tar.gz generic
  if command -v ldd >/dev/null 2>&1; then
    ldd "$source_dir/telescreen" >"$work/ldd" 2>&1 || :
    while IFS= read -r line; do
      case "$line" in *'not found'*) reason "$line" ;; esac
    done <"$work/ldd"
  fi
  if ! QT_QPA_PLATFORM=offscreen "$source_dir/telescreen" self-check >"$work/self-check" 2>&1; then
    reason 'offscreen self-check failed'
    while IFS= read -r line; do reason "$line"; done <"$work/self-check"
  fi
}

check_appimage() {
  read_libraries
  check_glibc 2.34
  for library in libGL.so.1 libEGL.so.1 libfontconfig.so.1 libfreetype.so.6 libxkbcommon.so.0 libdbus-1.so.3 libglib-2.0.so.0; do
    has_library "$library" || reason "missing library $library"
  done
}

check_method() {
  checking=$1
  : >"$work/reasons-$checking"
  case "$checking" in
    native|native-user) check_native ;;
    generic)
      check_generic_manifest
      if [ ! -s "$work/reasons-generic" ]; then check_generic_binary; fi
      ;;
    appimage) check_appimage ;;
  esac
  [ ! -s "$work/reasons-$checking" ]
}

current_method() {
  current=
  if command -v pacman >/dev/null 2>&1 && pacman -Q telescreen-bin >/dev/null 2>&1; then
    current=native
  elif [ -f "$share/install-method" ]; then
    current=$(cat "$share/install-method")
    case "$current" in native-user|generic|appimage) ;; *) current= ;; esac
  fi
}

priority() {
  case "$1" in native) echo 1 ;; native-user) echo 2 ;; generic) echo 3 ;; appimage) echo 4 ;; *) echo 5 ;; esac
}

open_tty() {
  [ "$tty_open" = 0 ] || return 0
  if ! ( : <"$tty" ) 2>/dev/null; then
    fail "cannot read $tty; use --yes. Command not run: $pending_command"
  fi
  exec 3<"$tty"
  tty_open=1
}

ask() {
  if [ "$yes" = 1 ]; then answer=yes; return 0; fi
  open_tty
  printf '%s ' "$1" >&2
  IFS= read -r answer <&3 || fail "no answer from $tty; use --yes. Command not run: $pending_command"
}

choose_unchecked() {
  for failed in native native-user generic appimage; do
    if [ -f "$work/reasons-$failed" ]; then cat "$work/reasons-$failed" >&2; fi
  done
  [ "$yes" != 1 ] || fail 'all checks failed; select a method with --native, --generic, or --appimage'
  ( : <"$tty" ) 2>/dev/null || fail "all checks failed and $tty cannot be read; select a method with --native, --generic, or --appimage"
  choices='generic, appimage, or stop'
  if is_arch; then choices="native, $choices"; fi
  ask "All checks failed. Choose $choices:"
  case "$answer" in
    native) is_arch || fail 'native is only available on Arch'; selected=$native_method ;;
    generic|appimage) selected=$answer ;;
    stop) fail 'installation stopped' ;;
    *) fail "invalid method: $answer" ;;
  esac
}

select_method() {
  native_method=native
  if [ "$user" = 1 ] || { [ "$(id -u)" != 0 ] && ! command -v sudo >/dev/null 2>&1; }; then
    native_method=native-user
  fi
  if [ -n "$requested" ]; then
    selected=$requested
    if [ "$requested" = native ] && [ "$user" = 1 ]; then selected=native-user; fi
    if ! check_method "$selected"; then
      cat "$work/reasons-$selected" >&2
      exit 1
    fi
    return 0
  fi
  selected=
  for candidate in "$native_method" generic appimage; do
    if check_method "$candidate"; then selected=$candidate; break; fi
  done
  if [ -z "$selected" ]; then choose_unchecked; return 0; fi
  if [ -n "$current" ] && [ "$(priority "$current")" -gt "$(priority "$selected")" ]; then
    pending_command="sh install.sh --${selected%-user}"
    if [ "$selected" = native-user ]; then pending_command="$pending_command --user"; fi
    ask "Change the current $current install to $selected? [y/N]"
    case "$answer" in
      y|Y|yes|YES) ;;
      *)
        if check_method "$current"; then selected=$current; else
          cat "$work/reasons-$current" >&2
          fail "cannot reinstall $current"
        fi
        ;;
    esac
  fi
}

copy_resources() {
  mkdir -p "$bin" "$share" || fail 'cannot create user install directories'
  rm -rf "$share/examples" "$share/contrib" || fail 'cannot replace resources'
  cp -R "$source_dir/examples" "$source_dir/contrib" "$share/" || fail 'cannot copy examples and contrib'
}

install_binary() {
  mkdir -p "$bin" || fail "cannot create $bin"
  cp "$1" "$bin/.telescreen.new" || fail 'cannot copy binary'
  chmod 755 "$bin/.telescreen.new" || fail 'cannot make binary executable'
  mv "$bin/.telescreen.new" "$bin/telescreen" || fail 'cannot install binary'
}

write_desktop() {
  mkdir -p "$apps" || fail 'cannot create launcher directory'
  # Escape Desktop Entry string/Exec quoting, independent of awk replacement syntax.
  exec_path=$(printf '%s' "$bin/telescreen" | awk '{gsub(/\\/, "\\\\\\\\"); gsub(/"/, "\\\\\\\""); gsub(/`/, "\\\\`"); gsub(/\$/, "\\\\$"); printf "%s", $0}')
  while IFS= read -r line; do
    case "$line" in
      Exec=*) printf 'Exec=%s"%s"\n' "$exec_env" "$exec_path" ;;
      *) printf '%s\n' "$line" ;;
    esac
  done <"$1" >"$desktop" || fail 'cannot write launcher'
}

install_icon() {
  if [ -f "$1" ]; then
    mkdir -p "$icons" || fail 'cannot create icon directory'
    cp "$1" "$icon" || fail 'cannot install icon'
  fi
}

install_applet_directory() {
  mkdir -p "${applet%/*}" || fail 'cannot create applet directory'
  rm -rf "$applet" || fail 'cannot remove old applet'
  cp -R "$1" "$applet" || fail 'cannot install applet'
  applet_installed=1
}

record_method() {
  mkdir -p "$share" || fail 'cannot create install record directory'
  printf '%s\n' "$selected" >"$share/install-method" || fail 'cannot record install method'
}

install_archive() {
  case "$selected" in
    native-user) extract_archive telescreen-arch-x86_64.tar.gz native ;;
    generic)
      if [ -d "$work/generic/telescreen" ]; then source_dir=$work/generic/telescreen; else
        extract_archive telescreen-linux-x86_64.tar.gz generic
      fi
      ;;
  esac
  copy_resources
  install_binary "$source_dir/telescreen"
  exec_env=
  write_desktop "$source_dir/dev.telescreen.Telescreen.desktop"
  install_icon "$source_dir/dev.telescreen.Telescreen.png"
  install_applet_directory "$source_dir/applet/dev.telescreen.menu"
  record_method
}

extract_appimage_resources() {
  mkdir -p "$work/appimage" || fail 'cannot create AppImage extraction directory'
  (cd "$work/appimage" && "$work/Telescreen-x86_64.AppImage" --appimage-extract "$1" >/dev/null) || fail "cannot extract $1 from AppImage"
}

install_plasmoid() {
  if command -v unzip >/dev/null 2>&1; then
    mkdir -p "$work/plasmoid" || fail 'cannot create plasmoid extraction directory'
    unzip -q "$work/dev.telescreen.menu.plasmoid" -d "$work/plasmoid" || fail 'cannot unpack plasmoid'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -m zipfile -e "$work/dev.telescreen.menu.plasmoid" "$work/plasmoid" || fail 'cannot unpack plasmoid'
  elif command -v bsdtar >/dev/null 2>&1; then
    # Arch has bsdtar with pacman (libarchive). It reads zip files.
    mkdir -p "$work/plasmoid" || fail 'cannot create plasmoid extraction directory'
    bsdtar -xf "$work/dev.telescreen.menu.plasmoid" -C "$work/plasmoid" || fail 'cannot unpack plasmoid'
  else
    echo 'telescreen: skipping applet: install unzip, python3, or bsdtar to unpack the plasmoid' >&2
    return 0
  fi
  install_applet_directory "$work/plasmoid"
}

install_appimage() {
  download Telescreen-x86_64.AppImage
  download dev.telescreen.menu.plasmoid
  chmod 755 "$work/Telescreen-x86_64.AppImage" || fail 'cannot make AppImage executable'
  extract_appimage_resources usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png
  extract_appimage_resources usr/share/applications/dev.telescreen.Telescreen.desktop
  install_binary "$work/Telescreen-x86_64.AppImage"
  exec_env=
  if ! command -v fusermount >/dev/null 2>&1 && ! command -v fusermount3 >/dev/null 2>&1; then
    exec_env='env APPIMAGE_EXTRACT_AND_RUN=1 '
  fi
  write_desktop "$work/appimage/squashfs-root/usr/share/applications/dev.telescreen.Telescreen.desktop"
  install_icon "$work/appimage/squashfs-root/usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png"
  install_plasmoid
  record_method
}

prepare_native_command() {
  set -- pacman -U --needed
  if [ "$yes" = 1 ]; then set -- "$@" --noconfirm; fi
  set -- "$@" "$work/telescreen-arch-x86_64.pkg.tar.zst"
  if [ "$(id -u)" != 0 ]; then set -- sudo "$@"; fi
  pending_command=
  for argument in "$@"; do
    escaped=$(printf '%s' "$argument" | sed "s/'/'\\\\''/g")
    pending_command="$pending_command '$escaped'"
  done
  echo "$pending_command"
}

confirm_cleanup() {
  cleanup=0
  case "$current" in
    native-user|generic|appimage)
      ask 'Remove the old user install files (keeping the config)? [y/N]'
      case "$answer" in y|Y|yes|YES) cleanup=1 ;; esac
      ;;
  esac
}

remove_user_install() {
  [ "$cleanup" = 1 ] || return 0
  for path in "$bin/telescreen" "$desktop" "$icon" "$applet" "$share"; do
    rm -rf "$path" || fail "cannot remove $path"
    echo "Removed $path"
  done
}

install_native() {
  download telescreen-arch-x86_64.pkg.tar.zst
  prepare_native_command
  # Resolve every question before pacman or removal changes the installation.
  if [ "$yes" != 1 ]; then open_tty; fi
  confirm_cleanup
  set -- pacman -U --needed
  if [ "$yes" = 1 ]; then set -- "$@" --noconfirm; fi
  set -- "$@" "$work/telescreen-arch-x86_64.pkg.tar.zst"
  if [ "$(id -u)" != 0 ]; then set -- sudo "$@"; fi
  if [ "$yes" = 1 ]; then
    "$@" || fail 'pacman installation failed'
  else
    "$@" <&3 || fail 'pacman installation failed'
  fi
  remove_user_install
}

install_macos() {
  extract_archive telescreen-macos-arm64.tar.gz macos
  copy_resources
  mkdir -p "$prefix/Applications" || fail 'cannot create Applications directory'
  rm -rf "$prefix/Applications/Telescreen.app" || fail 'cannot replace macOS app'
  mv "$source_dir/Telescreen.app" "$prefix/Applications/Telescreen.app" || fail 'cannot install macOS app'
  ln -sf "$prefix/Applications/Telescreen.app/Contents/MacOS/telescreen" "$bin/telescreen" || fail 'cannot link macOS app'
  echo "Installed $prefix/Applications/Telescreen.app and $bin/telescreen."
  echo 'macOS asks for Accessibility and Input Monitoring at the first start.'
}

end_messages() {
  if [ "$os" = Linux ]; then
    if [ "$selected" != native ]; then echo "Installed $bin/telescreen and $desktop."; fi
    if [ ! -w /dev/uinput ]; then
      echo 'You cannot write to /dev/uinput. Add your user to its group, then log in again.'
    fi
  fi
  examples=$share/examples
  if [ "${selected:-}" = native ]; then examples=/usr/share/doc/telescreen/examples; fi
  echo "The example config is in $examples/config.yaml. Write your config to"
  echo "${XDG_CONFIG_HOME:-$HOME/.config}/telescreen/config.yaml."
  if pgrep -x telescreen >/dev/null 2>&1; then
    echo 'Telescreen runs now. Start it again to use the new version.'
  fi
  if [ "$applet_installed" = 1 ] && pgrep -x plasmashell >/dev/null 2>&1; then
    echo 'If the tray does not show Telescreen, run: systemctl --user restart plasma-plasmashell'
  fi
  if [ "${selected:-}" != native ]; then
    case ":$PATH:" in *":$bin:"*) ;; *) echo "Add $bin to your PATH." ;; esac
  fi
}

main() {
  parse_options "$@"
  initialize
  if [ "$os" = Darwin ]; then
    install_macos
  else
    current_method
    select_method
    case "$selected" in
      native) install_native ;;
      native-user|generic) install_archive ;;
      appimage) install_appimage ;;
    esac
  fi
  end_messages
}

main "$@"
