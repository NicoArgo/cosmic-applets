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

# Keep the auto-reapply golden copy in sync when one is present.
if [ -f /usr/local/lib/pop-flow/cosmic-app-list ]; then
    echo "==> Refreshing auto-reapply golden copy"
    sudo install -m 0755 "$BIN" /usr/local/lib/pop-flow/cosmic-app-list
fi

echo "==> Restarting the panel to reload the applet..."
pkill -x cosmic-panel 2>/dev/null || true

echo "==> Done. Hover a running app's icon in the panel to preview its window(s)."
echo "    (If the panel doesn't come back on its own, log out and back in.)"
