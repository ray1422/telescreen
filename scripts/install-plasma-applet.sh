#!/bin/sh
# Install or upgrade the Telescreen Plasma applet for the current user (UI-11).
#
#   scripts/install-plasma-applet.sh
#
# The script installs the package in ~/.local/share/plasma/plasmoids. It does not
# add the applet to a panel: the system tray shows it while Telescreen runs.
# To remove it: kpackagetool6 -t Plasma/Applet --remove dev.telescreen.menu
set -eu

package=$(dirname "$0")/../applet/dev.telescreen.menu
if kpackagetool6 -t Plasma/Applet --show dev.telescreen.menu >/dev/null 2>&1; then
  kpackagetool6 -t Plasma/Applet --upgrade "$package"
else
  kpackagetool6 -t Plasma/Applet --install "$package"
fi
