#!/bin/sh
# Install Telescreen from a GitHub release for the current user (docs/spec.md DIST-3).
#
#   curl -fsSL https://github.com/ray1422/telescreen/releases/latest/download/install.sh | sh
#
# The script needs no sudo. It does not change the config or a system setting.
#
# Environment:
#   TELESCREEN_VERSION   a release tag, for example v0.3.0 (default: the latest release)
#   TELESCREEN_BASE_URL  the location of the release files (for a test)
#   TELESCREEN_PREFIX    the install root (default: $HOME)
set -eu

repo=https://github.com/ray1422/telescreen
if [ -n "${TELESCREEN_BASE_URL:-}" ]; then
  base=$TELESCREEN_BASE_URL
elif [ -n "${TELESCREEN_VERSION:-}" ]; then
  base=$repo/releases/download/$TELESCREEN_VERSION
else
  base=$repo/releases/latest/download
fi
prefix=${TELESCREEN_PREFIX:-$HOME}
bin=$prefix/.local/bin
share=$prefix/.local/share/telescreen

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) name=telescreen-macos-arm64 ;;
  Linux-x86_64) name=telescreen-linux-x86_64 ;;
  *)
    echo "telescreen: no release for $(uname -s) $(uname -m). Build from source." >&2
    exit 1
    ;;
esac

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL "$base/$name.tar.gz" -o "$work/$name.tar.gz"
curl -fsSL "$base/sha256sums.txt" -o "$work/sha256sums.txt"

grep " $name.tar.gz\$" "$work/sha256sums.txt" >"$work/expected"
if [ ! -s "$work/expected" ]; then
  echo "telescreen: sha256sums.txt has no entry for $name.tar.gz" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$work" && sha256sum -c expected >/dev/null)
else
  (cd "$work" && shasum -a 256 -c expected >/dev/null)
fi
tar -xzf "$work/$name.tar.gz" -C "$work"

mkdir -p "$bin" "$share"
rm -rf "$share/examples" "$share/contrib"
cp -R "$work/telescreen/examples" "$work/telescreen/contrib" "$share/"

if [ "$(uname -s)" = Darwin ]; then
  apps=$prefix/Applications
  mkdir -p "$apps"
  rm -rf "$apps/Telescreen.app"
  mv "$work/telescreen/Telescreen.app" "$apps/Telescreen.app"
  ln -sf "$apps/Telescreen.app/Contents/MacOS/telescreen" "$bin/telescreen"
  echo "Installed $apps/Telescreen.app and $bin/telescreen."
  echo "macOS asks for Accessibility and Input Monitoring at the first start."
else
  # A rename also works while the old program runs.
  cp "$work/telescreen/telescreen" "$bin/.telescreen.new"
  chmod 755 "$bin/.telescreen.new"
  mv "$bin/.telescreen.new" "$bin/telescreen"
  # The session of a launcher can have no ~/.local/bin in its PATH (DIST-4).
  apps=$prefix/.local/share/applications
  mkdir -p "$apps"
  sed "s|^Exec=.*|Exec=\"$bin/telescreen\"|" "$work/telescreen/dev.telescreen.Telescreen.desktop" \
    >"$apps/dev.telescreen.Telescreen.desktop"
  echo "Installed $bin/telescreen and $apps/dev.telescreen.Telescreen.desktop."
  if [ ! -w /dev/uinput ]; then
    echo "You cannot write to /dev/uinput. Add your user to its group, then log in again."
  fi
fi
echo "The example config is in $share/examples/config.yaml. Write your config to"
echo "${XDG_CONFIG_HOME:-$HOME/.config}/telescreen/config.yaml."
if pgrep -x telescreen >/dev/null 2>&1; then
  echo "Telescreen runs now. Start it again to use the new version."
fi
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Add $bin to your PATH." ;;
esac
