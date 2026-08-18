#!/usr/bin/env bash
# Build and install POP Flow's cosmic-app-list (taskbar window-preview on hover).
#
# The applets normally share one multiplexed binary (/usr/bin/cosmic-applets),
# and /usr/bin/cosmic-app-list is a symlink to it. Building that multiplexed
# binary pulls in every applet (some need libudev-dev etc.). We only changed
# app-list, which has its own standalone `main`, so we build JUST that and drop
# the standalone binary in place of the symlink — surgical, no extra system deps,
# other applets untouched. Reversible with ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"

echo "==> Building (cargo build --release -p cosmic-app-list)..."
cargo build --release -p cosmic-app-list

BIN="target/release/cosmic-app-list"
[ -f "$BIN" ] || { echo "Build failed: $BIN not found"; exit 1; }

echo "==> Installing to /usr/bin/cosmic-app-list (replaces the symlink; needs sudo)..."
sudo install -m 0755 "$BIN" /usr/bin/cosmic-app-list

# Keep the auto-reapply golden copy in sync, or say out loud that this install
# is temporary — silence here used to hide the fact that a package update wipes
# the feature (dpkg owns /usr/bin/cosmic-app-list and restores it as a symlink).
GOLDEN=/usr/local/lib/pop-flow/cosmic-app-list
if [ -f "$GOLDEN" ]; then
    echo "==> Refreshing auto-reapply golden copy"
    sudo install -m 0755 "$BIN" "$GOLDEN"
else
    echo "!! No auto-reapply hook installed: the next cosmic-applets package"
    echo "   update will restore the stock symlink and drop this applet."
    echo "   Run ./setup-auto-reapply.sh to make this install stick."
fi

# --- show-desktop applet ---------------------------------------------------
# This one is NEW software rather than a replacement, so it installs under
# /usr/local (which is on XDG_DATA_DIRS) and needs no auto-reapply hook: a
# package update cannot restore a file dpkg never owned.
echo
echo "==> Building (cargo build --release -p cosmic-applet-show-desktop)..."
cargo build --release -p cosmic-applet-show-desktop

SD_BIN="target/release/cosmic-applet-show-desktop"
[ -f "$SD_BIN" ] || { echo "Build failed: $SD_BIN not found"; exit 1; }

SD_ID=com.popflow.CosmicAppletShowDesktop
echo "==> Installing the show-desktop applet (needs sudo)..."
sudo install -Dm 0755 "$SD_BIN" /usr/local/bin/cosmic-applet-show-desktop
sudo install -Dm 0644 "cosmic-applet-show-desktop/data/$SD_ID.desktop" \
    "/usr/local/share/applications/$SD_ID.desktop"
sudo install -Dm 0644 \
    "cosmic-applet-show-desktop/data/icons/scalable/apps/$SD_ID.svg" \
    "/usr/local/share/icons/hicolor/scalable/apps/$SD_ID.svg"
sudo gtk-update-icon-cache -f -t /usr/local/share/icons/hicolor 2>/dev/null || true

# --- folder buttons --------------------------------------------------------
# These need no binary of ours at all. cosmic-panel-button -- already on the
# system, from this very package -- draws a panel button out of a desktop entry
# and runs that entry's Exec when pressed, which is how the App Library button
# works. So a folder button is four small files and no code: two applet entries
# for the panel to list, and the two entries they point at. Under /usr/local
# for the same reason as show-desktop above: dpkg never owns it, so no package
# update can take it away and no auto-reapply hook is needed.
echo
echo "==> Installing the Pictures and Downloads folder buttons (needs sudo)..."
for f in data/folder-buttons/*.desktop; do
    sudo install -Dm 0644 "$f" "/usr/local/share/applications/$(basename "$f")"
    # ~/.local/share/applications comes first in the desktop-entry search path,
    # so a copy left there wins over the one we just installed -- silently, and
    # forever. Say so rather than deleting a file in the user's home.
    home_copy="$HOME/.local/share/applications/$(basename "$f")"
    if [ -f "$home_copy" ] && ! cmp -s "$f" "$home_copy"; then
        echo "    !! $home_copy is older and shadows this install; remove it."
    fi
done

echo "==> Restarting the panel to reload the applets..."
pkill -x cosmic-panel 2>/dev/null || true

echo
echo "==> Done."
echo "    Hover a running app's icon in the panel to preview its window(s)."
echo "    For the show-desktop button, add it in Settings -> Desktop -> Panel"
echo "    -> Configure panel applets. It is not added automatically, because"
echo "    that would mean rewriting your panel configuration."
echo
echo "    The Pictures and Downloads buttons are added the same way, in that"
echo "    same panel-applet list."
echo
echo "    The same toggle, for a keyboard shortcut or a gesture:"
echo "        cosmic-applet-show-desktop --toggle"
echo
echo "    (If the panel doesn't come back on its own, log out and back in.)"
