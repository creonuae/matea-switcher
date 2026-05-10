#!/usr/bin/env bash
# matea-switcher: добавить наш virtual keyboard в keyd ignore-list.
# Запуск:  sudo bash scripts/setup-keyd.sh
# Идемпотентно — повторный запуск ничего не сломает.

set -euo pipefail

CONFIG=/etc/keyd/default.conf

if [ ! -f "$CONFIG" ]; then
    echo "ERROR: $CONFIG не существует — keyd не установлен либо config в другом месте."
    exit 1
fi

if grep -q '^-6d61:7465' "$CONFIG"; then
    echo "OK: -6d61:7465 уже в $CONFIG, ничего не делаю."
    exit 0
fi

BACKUP="$CONFIG.bak.matea-$(date +%Y%m%d_%H%M%S)"
cp "$CONFIG" "$BACKUP"
echo "backup сохранён: $BACKUP"

if grep -q '^\[ids\]' "$CONFIG"; then
    # секция уже есть — добавить строку после её заголовка
    sed -i '/^\[ids\]/a -6d61:7465' "$CONFIG"
    echo "добавил -6d61:7465 в существующую [ids] секцию"
else
    # секции нет — допишем целиком в конец
    printf '\n[ids]\n-6d61:7465\n' >> "$CONFIG"
    echo "создал новую [ids] секцию с -6d61:7465"
fi

echo "--- $CONFIG после изменения ---"
cat "$CONFIG"
echo "---"

# keyd unit на разных дистрибутивах не всегда поддерживает 'reload' (нет
# ExecReload= директивы). Сначала пробуем reload (soft, без обрыва текущих
# grab'ов), при неудаче — restart.
if systemctl reload keyd 2>/dev/null; then
    echo "OK: keyd reload прошёл."
elif systemctl restart keyd; then
    echo "OK: keyd restart прошёл (reload не поддержан unit'ом)."
else
    echo "ERROR: keyd reload и restart провалились. journalctl -u keyd -n 20"
    exit 1
fi

sleep 1
if systemctl is-active --quiet keyd; then
    echo "OK: keyd active после применения изменений."
else
    echo "WARN: keyd не active после reload/restart. journalctl -u keyd -n 20"
    exit 1
fi
