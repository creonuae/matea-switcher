#!/usr/bin/env bash
# matea-switcher — local install script
#
# Делает:
#   1. cargo build --release
#   2. Копирует binary в ~/.local/bin/matea-switcher
#   3. Копирует systemd unit в ~/.config/systemd/user/
#   4. systemctl --user daemon-reload && enable matea-switcher.service
#
# НЕ запускает service — это юзер делает сам через `systemctl --user start`
# или logout/login (после enable юнит autostart'нет на следующем входе).
#
# Пред-условия:
#   - Rust toolchain (cargo, rustc)
#   - System deps: см. README install section
#   - Юзер в группе input (для evdev/uinput)
#
# Удалить установку: scripts/uninstall.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$HOME/.local/bin"
UNIT_DIR="$HOME/.config/systemd/user"
UNIT_NAME="matea-switcher.service"

echo "==> Build release binary"
cd "$REPO_ROOT"
cargo build --release

echo "==> Install binary → $BIN_DIR/matea-switcher"
mkdir -p "$BIN_DIR"
install -m 755 target/release/matea-switcher "$BIN_DIR/matea-switcher"

echo "==> Install systemd unit → $UNIT_DIR/$UNIT_NAME"
mkdir -p "$UNIT_DIR"
install -m 644 "$REPO_ROOT/systemd/$UNIT_NAME" "$UNIT_DIR/$UNIT_NAME"

echo "==> systemctl --user daemon-reload"
systemctl --user daemon-reload

echo "==> systemctl --user enable $UNIT_NAME"
systemctl --user enable "$UNIT_NAME"

cat <<EOF

✅ Установлено.

Дальше:
  - Запустить сейчас:    systemctl --user start $UNIT_NAME
  - Посмотреть логи:     journalctl --user -u $UNIT_NAME -f
  - Остановить:          systemctl --user stop $UNIT_NAME
  - Disable autostart:   systemctl --user disable $UNIT_NAME
  - Удалить полностью:   $REPO_ROOT/scripts/uninstall.sh

Конфиг по умолчанию создастся в ~/.config/matea-switcher/config.toml
при первом запуске. Edit его → systemctl --user restart $UNIT_NAME.

Аварийный stop переписывания (без stop'а демона) — Ctrl+Shift+M в любом
окне. Это hotkey по умолчанию, можно сменить в config.

EOF
