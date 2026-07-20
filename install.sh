#!/usr/bin/env bash
# Build and install POP Flow's cosmic-applets (taskbar window-preview on hover)
# over the system applets. All applets share ONE multiplexed binary
# (/usr/bin/cosmic-applets); the per-applet names (cosmic-app-list, ...) are
# symlinks to it, so we only swap that one binary. Reversible with ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"

echo "==> Building (cargo build --release -p cosmic-applets)..."
cargo build --release -p cosmic-applets

BIN="target/release/cosmic-applets"
[ -f "$BIN" ] || { echo "Build failed: $BIN not found"; exit 1; }

if [ ! -f cosmic-applets.orig ] && [ -f /usr/bin/cosmic-applets ]; then
    echo "==> Backing up current /usr/bin/cosmic-applets -> ./cosmic-applets.orig"
    cp /usr/bin/cosmic-applets cosmic-applets.orig
fi

echo "==> Installing to /usr/bin/cosmic-applets (needs sudo)..."
sudo install -m 0755 "$BIN" /usr/bin/cosmic-applets

# Keep the auto-reapply golden copy in sync when one is present.
if [ -f /usr/local/lib/pop-flow/cosmic-applets ]; then
    echo "==> Refreshing auto-reapply golden copy"
    sudo install -m 0755 "$BIN" /usr/local/lib/pop-flow/cosmic-applets
fi

echo "==> Restarting the panel to reload the applet..."
pkill -x cosmic-panel 2>/dev/null || true

echo "==> Done. Hover a running app's icon in the panel to preview its window(s)."
echo "    (If the panel doesn't come back on its own, log out and back in.)"
