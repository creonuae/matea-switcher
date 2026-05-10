# matea-switcher

AI-powered keyboard layout switcher для билингвов RU↔EN. Цель — пользователь
**вообще не думает о раскладке**: набрал «ghbdtn» — мгновенно стало «привет»,
переключение языка между приложениями происходит автоматически по контексту.

Аналог Punto Switcher / Caramba Switcher, но:
- Linux Wayland-first (KDE Plasma 6 основной таргет, потом GNOME)
- встроенная локальная LLM (Qwen-2.5-0.5B GGUF) для амбигуальных случаев и
  proactive language prediction по контексту окна
- никаких cloud API, никаких задержек на сетку
- кросс-платформенная архитектура с заделом на macOS / Windows

## Статус

🚧 v0.1 — milestones M1-M4 готовы (evdev reader, xkbcommon, WordBuffer,
Hunspell classifier). Следующий — M5: uinput rewriter. matea сейчас умеет
**слушать** клавиатуру и говорить «надо ли переключить раскладку», но ещё
не **переписывает** ввод.

## Для AI-агентов / contributors

Точка входа — [`AGENTS.md`](AGENTS.md) (universal формат) и
[`CLAUDE.md`](CLAUDE.md) для Claude Code. Полный план дальнейшей работы —
[`docs/NEXT_STEPS.md`](docs/NEXT_STEPS.md). Архитектура —
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Не повторяйте research,
не переоткрывайте решения — всё load-bearing зафиксировано в этих файлах.

## Roadmap

- **v0.1** — Linux/Wayland MVP без LLM
  - evdev → uinput pipeline
  - Hunspell + n-gram fast-path classifier
  - hardcoded blacklist (Konsole, VSCode, JetBrains, terminals)
  - reactive rewrite на word boundary
- **v0.2** — встраиваем Qwen-0.5B GGUF через `llama_cpp_2`
  - slow-path для амбигуальных слов (latency ≤50мс)
  - benchmarks
- **v0.3** — proactive layout prediction
  - KWin DBus listener `activeWindowChanged`
  - AT-SPI чтение текста активного окна для контекста
  - per-window memory (запоминание языка по окну/чату)
- **v0.4** — macOS port (CGEventTap, AXUIElement)
- **v0.5** — Windows port (WH_KEYBOARD_LL, UI Automation)
- **v0.6+** — Tab-completion (использует ту же LLM с контекстом экрана)

## Архитектура

См. `docs/ARCHITECTURE.md`.

```
matea
├── core (platform-agnostic)
│   ├── classifier  — keep | flip | uncertain
│   ├── mapper      — qwerty ↔ йцукен таблицы
│   ├── context     — WordBuffer + WordHistory
│   └── llm         — llama_cpp_2 wrapper (v0.2+)
└── platform
    ├── linux       — evdev + uinput + zbus(KWin) + atspi
    ├── macos       — core-graphics + AXUIElement (v0.4)
    └── windows     — windows-rs (v0.5)
```

## Сборка (Linux)

Требуется:
- Rust 1.93+ (Fedora: `dnf install rust cargo`)
- clang, cmake, pkg-config (для будущей сборки llama_cpp_2)
- `llvm-devel` (для llama_cpp_2 в v0.2)
- Группа `input` для пользователя (для чтения `/dev/input/event*`):
  ```
  sudo usermod -aG input $USER
  # перелогиниться
  ```
- Пакеты Hunspell словарей (Fedora):
  ```
  sudo dnf install hunspell hunspell-en-US hunspell-ru
  ```

```bash
git clone https://github.com/creonuae/matea-switcher.git
cd matea-switcher
cargo build --release
./target/release/matea-switcher
```

## Установка как systemd-user-сервис

```bash
./scripts/install.sh
# Это:
#   - cargo build --release
#   - install binary в ~/.local/bin/matea-switcher
#   - install systemd unit в ~/.config/systemd/user/matea-switcher.service
#   - daemon-reload + enable (autostart на следующем входе в Plasma session)

# Запустить сейчас (без logout):
systemctl --user start matea-switcher.service

# Логи в реальном времени:
journalctl --user -u matea-switcher -f

# Удалить:
./scripts/uninstall.sh
```

При первом запуске matea создаст `~/.config/matea-switcher/config.toml` с
дефолтами. Edit его → `systemctl --user restart matea-switcher`.

**Аварийный stop переписывания** (без stop'а демона) — `Ctrl+Shift+M` в
любом окне. Hotkey настраивается в config.

## Если у тебя keyd

`keyd` (популярный Linux remapper клавиатуры) по дефолту обрабатывает
**все** virtual keyboards включая наш. Это создаёт echo-loop и дубли
символов в окне после FLIP. **Один раз** добавь блок в keyd-конфиг —
см. [`docs/keyd-setup.md`](docs/keyd-setup.md). Без этого matea-switcher
работать будет, но юзер будет видеть `давайдавай` вместо `давай`.

## Privacy

Весь анализ текста (классификация раскладки, контекст экрана для proactive
prediction, будущий autocomplete) — **строго on-device**. Никаких HTTP-запросов
во внешнюю сеть. LLM веса bundled / download один раз с HuggingFace.

Password fields (AT-SPI `STATE_PROTECTED`) — не читаем никогда.

## License

MIT OR Apache-2.0
