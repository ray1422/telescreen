#!/bin/sh
# Install or upgrade the current user's Plasma applet (UI-11).
# The system tray shows it while Telescreen runs; this does not add it to a panel.
# Remove it with: kpackagetool6 -t Plasma/Applet --remove dev.telescreen.menu
set -eu

fail() {
  echo "telescreen: $*" >&2
  exit 1
}

install_applet() {
  package=$(dirname "$0")/../applet/dev.telescreen.menu
  if kpackagetool6 -t Plasma/Applet --show dev.telescreen.menu >/dev/null 2>&1; then
    kpackagetool6 -t Plasma/Applet --upgrade "$package" || fail 'cannot upgrade Plasma applet'
  else
    kpackagetool6 -t Plasma/Applet --install "$package" || fail 'cannot install Plasma applet'
  fi
}

install_applet
