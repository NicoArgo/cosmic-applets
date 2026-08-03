#!/usr/bin/env bash
# Undo install.sh: restore /usr/bin/cosmic-app-list to a symlink to the stock
# multiplexed cosmic-applets binary.
set -euo pipefail
cd "$(dirname "$0")"

if [ -f /usr/local/lib/pop-flow/cosmic-app-list ]; then
    echo "==> Removing auto-reapply golden copy (needs sudo)..."
    sudo rm -f /usr/local/lib/pop-flow/cosmic-app-list
fi

echo "==> Restoring /usr/bin/cosmic-app-list -> cosmic-applets symlink (needs sudo)..."
sudo ln -sf cosmic-applets /usr/bin/cosmic-app-list

SD_ID=com.popflow.CosmicAppletShowDesktop
echo "==> Removing the show-desktop applet (needs sudo)..."
sudo rm -f /usr/local/bin/cosmic-applet-show-desktop \
           "/usr/local/share/applications/$SD_ID.desktop" \
           "/usr/local/share/icons/hicolor/scalable/apps/$SD_ID.svg"
sudo gtk-update-icon-cache -f -t /usr/local/share/icons/hicolor 2>/dev/null || true
echo "!! If it was on your panel, remove it in Settings -> Desktop -> Panel;"
echo "   the config still lists it and the slot would sit empty."

echo "==> Restarting the panel..."
pkill -x cosmic-panel 2>/dev/null || true
echo "==> Restored. (Log out/in if the panel doesn't return.)"
