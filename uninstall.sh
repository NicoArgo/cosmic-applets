#!/usr/bin/env bash
# Undo install.sh: restore /usr/bin/cosmic-app-list to a symlink to the stock
# multiplexed cosmic-applets binary.
set -euo pipefail
cd "$(dirname "$0")"

# Turn off auto-reapply first, or the next package operation would re-patch the
# binary right after we restore the original. Delegating to the script that owns
# those paths rather than repeating them: this used to remove only the golden
# copy, leaving a root-owned APT hook behind for good.
if [ -x ./remove-auto-reapply.sh ]; then
    ./remove-auto-reapply.sh
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

echo "==> Removing the show-desktop corner..."
systemctl --user disable --now cosmic-show-desktop-corner.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/cosmic-show-desktop-corner.service"
systemctl --user daemon-reload
sudo rm -f /usr/local/bin/cosmic-show-desktop-corner

echo "==> Removing the folder buttons (needs sudo)..."
sudo rm -f /usr/local/share/applications/com.popflow.CosmicAppletPicturesFolder.desktop \
           /usr/local/share/applications/com.popflow.PicturesFolder.desktop \
           /usr/local/share/applications/com.popflow.CosmicAppletDownloadsFolder.desktop \
           /usr/local/share/applications/com.popflow.DownloadsFolder.desktop \
           /usr/local/bin/cosmic-applet-folder-button

echo "==> Removing vampire mode (sleep returns to normal first)..."
# --off before the binary goes: it puts the idle settings back as they were.
if [ -x /usr/local/bin/cosmic-applet-vampire ]; then
    /usr/local/bin/cosmic-applet-vampire --off || true
fi
systemctl --user disable --now pop-flow-vampire.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/pop-flow-vampire.service" \
      "$HOME/.config/systemd/user/pop-flow-vampire-watch.service" \
      "$HOME/.local/state/pop-flow/vampire-until"
systemctl --user daemon-reload
sudo rm -f /usr/local/bin/cosmic-applet-vampire \
           /usr/local/share/applications/com.popflow.CosmicAppletVampire.desktop \
           /usr/local/share/icons/hicolor/scalable/apps/com.popflow.CosmicAppletVampire.svg

echo "==> Restarting the panel..."
pkill -x cosmic-panel 2>/dev/null || true
echo "==> Restored. (Log out/in if the panel doesn't return.)"
