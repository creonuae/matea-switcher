# Milestone 10 — systemd user unit + install/uninstall scripts

> Дата: 2026-05-10. Минимальная инфраструктура для daily-use.

## Что сделано

### `systemd/matea-switcher.service`

User-service для автозапуска matea-switcher при старте Plasma session:
- `PartOf=graphical-session.target` — стопается при logout.
- `After=plasma-plasmashell.service` — KWin DBus гарантированно готов
  (matea зависит от `org.kde.keyboard` /Layouts).
- `Restart=on-failure RestartSec=3` — самовосстанавливается при крэшах.
- `TimeoutStartSec=10` — даёт Plasma инициализироваться, не ругается сразу.
- `Environment=MATEA_LOG=info` — в production достаточно info, debug
  включается через `systemctl --user edit matea-switcher` (override).
- `StandardOutput/Error=journal` — все логи через journalctl,
  `journalctl --user -u matea-switcher -f` для live view.

`%h` в `ExecStart` — systemd специфика, разворачивается в `$HOME` (на этой
системе `/home/gold`). Бинарь ожидается в `~/.local/bin/matea-switcher`.

### `scripts/install.sh`

Скрипт самодостаточной установки для конечного юзера:
1. `cargo build --release`.
2. `install -m 755 target/release/matea-switcher ~/.local/bin/`.
3. `install -m 644 systemd/matea-switcher.service ~/.config/systemd/user/`.
4. `systemctl --user daemon-reload`.
5. `systemctl --user enable matea-switcher.service`.

**Не запускает service** (`systemctl start`) — это юзер делает сам. Так
безопаснее: установка не приводит к мгновенным изменениям в keyboard
behaviour, юзер может почитать config / hotkey перед стартом.

### `scripts/uninstall.sh`

Парный скрипт. Останавливает + disable'ит unit, удаляет binary и unit
file. **НЕ удаляет** user data (config + dict cache) — на случай если
юзер потом переустанавливает и не хочет потерять настройки. Если нужно
полностью — отдельные команды в выводе скрипта.

## WHY systemd-user, а не root-системный

- matea читает `/dev/input/event*` от лица **юзера** (через группу `input`),
  не root. Запуск под root — лишние права.
- Per-user service не аффектит других юзеров на машине.
- Auto-start привязан к Plasma session конкретного юзера, не к boot'у системы.
- `journalctl --user` показывает ровно те логи которые юзер ожидает увидеть,
  не смешивает с system-уровнем.

## WHY README+scripts вместо `cargo install`

`cargo install --path .` положил бы binary в `~/.cargo/bin/`, но не сделал
бы systemd unit. Юзеру всё равно пришлось бы вручную копировать unit и
делать enable. Скрипт `install.sh` делает обе части за один прогон.

## Не сделано (M10b/будущее)

- **RPM/DEB пакеты**. Сейчас только source install. Для распространения
  нужны:
  - Fedora: `.spec` файл, `dnf copr` (от COPR build farm).
  - Ubuntu: `debian/control`, PPA.
- **udev rule для /dev/uinput**. Если у юзера на Fedora 43 уже автоматически
  есть `+rw` для group input (как на dev-машине), не нужно. На других
  системах может понадобиться:
  ```
  KERNEL=="uinput", MODE="0660", GROUP="input"
  ```
  Положить в `/etc/udev/rules.d/99-matea-switcher.rules`.
- **Pre-install проверки** в install.sh: что юзер в группе input, что
  hunspell-en-US/ru установлены, что libxkbcommon есть. Сейчас падать на
  старте matea если чего-то не хватает — workable но не дружелюбно.
- **systemctl --user edit template**. Юзер может захотеть кастомный
  MATEA_LOG=debug — сейчас нужно вручную делать override. Можно добавить
  в install.sh опцию `--debug-log`.
