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

echo "==> Restarting the panel..."
pkill -x cosmic-panel 2>/dev/null || true
echo "==> Restored. (Log out/in if the panel doesn't return.)"
