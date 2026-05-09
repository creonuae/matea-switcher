# MaTea — архитектура

## Высокоуровневое

Тул работает как **systemd user daemon** (или standalone background process).
Он не лезет в существующие keyboard switchers KDE/GNOME — вместо этого читает
input напрямую через evdev (Linux) или CGEventTap (macOS) или WH_KEYBOARD_LL
(Windows), и пишет коррекции через uinput / CGEvent / SendInput соответственно.

Раскладку переключает через native API окружения (KGlobalAccel /
TISInputSourceRef / LoadKeyboardLayout) — то есть честно меняет «активный язык»
системы, чтобы дальнейший ввод шёл сразу правильным.

## Поток данных (Linux/KWin Wayland, v0.1+)

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              MaTea daemon                                │
│                                                                          │
│  /dev/input/event*                                                       │
│  ──── evdev ────► [KeyEvent stream]                                      │
│                          │                                               │
│                          ▼                                               │
│                   [WordBuffer]  ←── (sees char + KEY_BACKSPACE)          │
│                          │                                               │
│              on space / enter / punct ─┐                                 │
│                          ▼             │                                 │
│  ┌──────────────────────────────────┐  │                                 │
│  │  Fast-path classifier            │  │                                 │
│  │  - hunspell::check(en_US)        │  │                                 │
│  │  - hunspell::check(ru_RU)        │  │                                 │
│  │  - bigram lookup (precomp)       │  │                                 │
│  └────────┬─────────────────────────┘  │                                 │
│           │ Verdict                    │                                 │
│           ├─ Keep ────────────────────►│ → push to WordHistory           │
│           ├─ Flip ───────────────────► │                                 │
│           │       │                    │                                 │
│           │       ▼                    │                                 │
│           │  ┌───────────────────────┐ │                                 │
│           │  │ Rewriter              │ │                                 │
│           │  │  EVIOCGRAB                                              │ │
│           │  │  uinput: BS×N + text  │ │                                 │
│           │  │  zbus → KWin          │ │                                 │
│           │  │    setLayout(ru/en)   │ │                                 │
│           │  │  release GRAB         │ │                                 │
│           │  └───────────────────────┘ │                                 │
│           │                            │                                 │
│           └─ Uncertain ─►┌───────────┐ │                                 │
│                          │ LLM slow- │ │                                 │
│                          │ path      │ │                                 │
│                          │ Qwen-0.5B │ │                                 │
│                          │ GBNF: keep│ │                                 │
│                          │       flip│ │                                 │
│                          └─────┬─────┘ │                                 │
│                                ▼       │                                 │
│                         (Keep / Flip)──┘                                 │
│                                                                          │
│                                                                          │
│  zbus → org.kde.KWin                                                     │
│    listen `activeWindowChanged` ───► [Context: window_class, chat_id]    │
│                                                                          │
│  AT-SPI (org.a11y.Bus) ───► [Context: visible text, password flag]       │
│                                                                          │
│  → Predictor (proactive) ───► auto-switch layout BEFORE user types       │
└──────────────────────────────────────────────────────────────────────────┘
```

## Модули

### `classifier` (platform-agnostic)
- Принимает `ClassifyInput { word, active_layout, recent_words, window_class }`
- Возвращает `Verdict::Keep | Flip | Uncertain`
- Fast-path: Hunspell словарь + n-gram таблица (precomputed bigram-frequency для
  RU и EN — детектит «эта последовательность букв нереалистична для языка X»)
- Slow-path (v0.2+): llama_cpp_2 inference с GBNF grammar `keep|flip`

### `mapper` (platform-agnostic)
- Чистые функции `en_to_ru(&str) -> String` и `ru_to_en(&str) -> String`
- Сохраняют регистр
- Пробрасывают unknown chars без изменения

### `context` (platform-agnostic)
- `WordBuffer` — собирает текущее печатаемое слово
- `WordHistory` — кольцевой буфер последних N слов на окно

### `platform::linux`
- `evdev` Reader: `Device::open_all()`, фильтр клавиатур, async stream через
  tokio (use `tokio::io::unix::AsyncFd`)
- `uinput` Writer: создаёт `/dev/uinput` устройство один раз на старте
- `EVIOCGRAB`: блокирует physical клавиатуру на момент rewrite
- `zbus` клиент:
  - subscribe на `org.kde.KWin.activeWindowChanged` (для proactive)
  - call `org.kde.keyboard.Layouts.setLayout(int)` для переключения
  - call `org.kde.KWin.activeWindow` для metadata текущего окна (resourceClass)
- AT-SPI client (`atspi` crate):
  - subscribe на `Object:TextChanged:Insert/Delete`
  - read `Accessible.text(...)` для context
  - check `STATE_PROTECTED` чтобы не читать password fields

### `platform::macos` (v0.4)
- `core-graphics::event::CGEvent` для tap + injection
- `core-foundation` + `accessibility-sys` для AXUIElement read

### `platform::windows` (v0.5)
- `windows-rs`: `SetWindowsHookEx(WH_KEYBOARD_LL, ...)` для read
- `SendInput` для write
- UI Automation (`IUIAutomation`) для context

## LLM подсистема (v0.2+)

- Bundled GGUF (download script тащит из HuggingFace, не в git)
- `llama_cpp_2` — Rust-биндинги к llama.cpp
- Inference в отдельном blocking thread, общение через mpsc channel
- Prefix caching: prompt prefix («Вы определяете правильную раскладку…»)
  кешируется один раз, classify-запрос только добавляет хвост
- Latency бюджет: ≤50мс на короткое слово (1-15 chars)
- Fallback при перегрузе CPU: пропустить LLM, отдать `Verdict::Keep`

## Конфиг

`~/.config/matea/config.toml`:

```toml
[general]
enabled = true
proactive_prediction = true

[classifier]
use_llm = true
llm_min_word_length = 1
llm_max_word_length = 15

[layouts]
us = { name = "us", display = "EN" }
ru = { name = "ru", display = "RU" }

[blacklist]
# WM_CLASS resourceClass значения — в этих окнах rewrite не происходит
window_classes = [
    "konsole", "yakuake", "kitty", "alacritty", "wezterm",
    "code", "code-oss", "VSCodium",
    "jetbrains-idea", "jetbrains-pycharm", "jetbrains-rider",
    "jetbrains-webstorm", "jetbrains-clion",
]

[hotkeys]
# Manual override hotkeys (если auto не сработал)
manual_flip_last_word = "Ctrl+Shift+L"
toggle_enabled = "Ctrl+Shift+M"
```

## Logging

Через `tracing`. Default уровень `info`, переопределяется через
`MATEA_LOG=debug` env var. Логи в `~/.local/state/matea/matea.log`
(если запущен через systemd) или stderr (если интерактивно).

## Testing

- Unit тесты на `mapper` (в файле `src/mapper.rs`)
- Unit тесты на `classifier` с mocked Hunspell
- Integration тесты на `platform::linux` под mock evdev (через `tempfile` +
  `nix::pty`) — TODO в v0.1
- E2E тест: запустить два процесса (mock-keyboard и matea), убедиться что
  rewrite happen — TODO в v0.2
