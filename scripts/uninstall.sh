#!/usr/bin/env bash
# matea-switcher — uninstall script. Парный к install.sh.
#
# Не удаляет user data (~/.config/matea-switcher/, ~/.local/share/matea-switcher/).
# Если нужно полностью — удали их руками.

set -euo pipefail

UNIT_NAME="matea-switcher.service"
BIN_PATH="$HOME/.local/bin/matea-switcher"
UNIT_PATH="$HOME/.config/systemd/user/$UNIT_NAME"

echo "==> Stop + disable systemd unit"
systemctl --user stop "$UNIT_NAME" 2>/dev/null || true
systemctl --user disable "$UNIT_NAME" 2>/dev/null || true

echo "==> Remove unit file"
rm -f "$UNIT_PATH"

echo "==> Remove binary"
rm -f "$BIN_PATH"

echo "==> daemon-reload"
systemctl --user daemon-reload

cat <<EOF

✅ Удалено.

User data (config + dict cache) **не тронуты**:
  ~/.config/matea-switcher/
  ~/.local/share/matea-switcher/

Если хочешь полностью убрать:
  rm -rf ~/.config/matea-switcher ~/.local/share/matea-switcher

EOF
