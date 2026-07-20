#!/usr/bin/env bash
# Restore the original system cosmic-applets binary (undo install.sh).
set -euo pipefail
cd "$(dirname "$0")"

[ -f cosmic-applets.orig ] || { echo "No backup (cosmic-applets.orig) found."; exit 1; }

if [ -f /usr/local/lib/pop-flow/cosmic-applets ]; then
    echo "==> Removing auto-reapply golden copy (needs sudo)..."
    sudo rm -f /usr/local/lib/pop-flow/cosmic-applets
fi

echo "==> Restoring original /usr/bin/cosmic-applets (needs sudo)..."
sudo install -m 0755 cosmic-applets.orig /usr/bin/cosmic-applets

echo "==> Restarting the panel..."
pkill -x cosmic-panel 2>/dev/null || true
echo "==> Restored. (Log out/in if the panel doesn't return.)"
