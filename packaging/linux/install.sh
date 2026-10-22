#!/bin/sh
# Installs TVHme for the current user: program, menu entry and icons (no root needed).
# Run it from the folder you unpacked the .tar.gz into. Undo: ./install.sh --uninstall
set -e
here="$(cd "$(dirname "$0")" && pwd)"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
sizes="16 24 32 48 64 128 256 512"
if [ "$1" = "--uninstall" ]; then
  rm -f "$bin/tvhme" "$data/applications/tvhme.desktop" "$data/icons/hicolor/scalable/apps/tvhme.svg"
  for s in $sizes; do rm -f "$data/icons/hicolor/${s}x${s}/apps/tvhme.png"; done
  echo "TVHme removed."; exit 0
fi
mkdir -p "$bin" "$data/applications" "$data/icons/hicolor/scalable/apps"
install -m 755 "$here/tvhme" "$bin/tvhme"
install -m 644 "$here/tvhme.svg" "$data/icons/hicolor/scalable/apps/tvhme.svg"
for s in $sizes; do
  mkdir -p "$data/icons/hicolor/${s}x${s}/apps"
  install -m 644 "$here/icons/${s}x${s}.png" "$data/icons/hicolor/${s}x${s}/apps/tvhme.png"
done
# the menu entry points at the full path, so it works even when ~/.local/bin is not in PATH
sed "s|^Exec=.*|Exec=$bin/tvhme|" "$here/tvhme.desktop" > "$data/applications/tvhme.desktop"
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$data/applications" || true
echo "TVHme installed in $bin. Look for it in your application menu."
