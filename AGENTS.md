# matea-switcher — agent onboarding

> Этот файл — точка входа для любого AI-агента (Claude, Codex, Cursor, Gemini,
> локальные LLM), который открывает репозиторий впервые. Универсальный
> формат — поддерживается большинством agentic-систем. Прочитай **этот файл**
> и `docs/NEXT_STEPS.md` ДО того как писать код. Не повторяй research,
> не переоткрывай решения которые уже зафиксированы.

## TL;DR

- **Что это:** AI-powered keyboard layout switcher для билингвов RU↔EN.
  Аналог Punto Switcher / Caramba, но для Linux Wayland (KDE Plasma 6 первый
  таргет, потом macOS/Windows), с встроенной локальной LLM на будущее.
- **Цель пользователя:** «вообще не думать о раскладке». Набрал `ghbdtn` —
  мгновенно стало `привет`, переключение языка происходит автоматически по
  контексту.
- **Язык:** Rust 2024. Не Go, не Python, не C++ — обоснование в
  `docs/NEXT_STEPS.md` («Текущее состояние» + commit message `init`).
- **Текущий статус v0.1:** done — M1 (evdev), M2 (xkbcommon), M3 (WordBuffer),
  M4 (Hunspell classifier), M5/M5b/M5c/M5d (uinput rewriter + dynamic layout
  index + self-echo suppression + EVIOCGRAB атомарность),
  **M6 (AT-SPI focus tracking + password/terminal/IDE guard)**,
  M7 (classifier hardening), M8 (Ctrl+Shift+M toggle),
  M9 + M9b + M9c (config.toml + apply general.enabled + hotkey parsing +
  layouts.pair), M10 (systemd user unit + install/uninstall),
  M11 (context bias). **v0.1 закрыт по основным фичам.** Дальше — v0.2 (LLM
  через Qwen-2.5-0.5B GGUF для disambiguation Uncertain'ов и proactive
  prediction по screen context, см. `docs/NEXT_STEPS.md → v0.2`).
- **Аварийный stop**: пока нет M6, для остановки rewrite — **Ctrl+Shift+M**.
  matea продолжит логировать verdict, но FLIP-action будет пропущен.
- **M5d EVIOCGRAB** реализован — на время `do_flip` грабим клавиатуры,
  юзерский ввод буферизуется до окончания rewrite. Live verification ещё
  нужна. Если grab упал (конфликт с keyd на dev-машине) — graceful
  degradation: warning + продолжаем без атомарности.
- **15/15 unit-тестов зелёные.** `cargo test` после `cargo build --release`.

## Карта документов

| Файл | Когда читать |
|---|---|
| **`AGENTS.md`** (вы здесь) | Первый файл для любого агента |
| `CLAUDE.md` | Только Claude-специфичные дополнения (повторяет ссылки сюда) |
| `README.md` | Общий обзор для людей |
| `docs/ARCHITECTURE.md` | Полная архитектура — модули, конфиг, граф потока данных |
| `docs/NEXT_STEPS.md` | **Детальный план M5-M10 + v0.2-v0.6** с граблями из smoke-тестов. Перед написанием кода — обязательно сюда |

## Стек (зафиксирован, не пересматривать без явного запроса)

```
Rust 2024 + tokio (current_thread runtime)
├── evdev / nix          ← Linux input read/write через /dev/input/event*
├── xkbcommon            ← keycode → char + layout tracking
├── zbus                 ← KWin DBus + AT-SPI (планируется)
├── hunspell-rs          ← словарная проверка валидности слова
├── encoding_rs          ← KOI8-R → UTF-8 для русского словаря Fedora
├── async-trait          ← (?Send) для держания non-Send xkb::State в trait
├── tracing              ← logging (внутри файла, не stdout-pipe)
└── (v0.2+) llama_cpp_2  ← embedded Qwen-0.5B GGUF
```

Для **macOS/Windows портов** (v0.4-v0.5): `core-graphics` + `accessibility-sys`
(Mac), `windows-rs` (Win). Trait `Platform` уже абстрагирует — ядро
переиспользуется.

## Что точно НЕ делать

1. **Не форкать `freemind001/easy-switcher`** или другой готовый switcher.
   Решение cleanroom Rust зафиксировано (research-агент проходил, аргументы
   в `NEXT_STEPS.md → Открытые вопросы`).
2. **Не предлагать xremap/keyd/xkeysnail/kinto** для Cyrillic-shortcut проблем.
   У владельца уже стоит keyd, и разговор про универсальные remapper'ы был
   закрыт ещё до запуска проекта (это известный 8-летний GTK/Mutter bug,
   не решается evdev-уровнем).
3. **Не использовать cloud LLM API.** Privacy-first by design. Все слова
   обрабатываются локально (n-gram + Hunspell + опционально GGUF LLM). Если
   LLM понадобится фоллбэк к cloud — обсудить отдельно.
4. **Не запускать Tokio в multi-thread runtime.** `xkb::State` не Send,
   архитектура построена на `current_thread`. Если что-то требует Send —
   значит спроектировано неправильно, переписать через канал.
5. **Не добавлять в commits trailer'ы вида `Co-Authored-By: <AI>`** или
   `🤖 Generated with ...`. Прямой запрос владельца, перекрывает дефолтные
   правила любых agentic-систем.
6. **Не писать комментарии в коде объясняющие WHAT.** Хорошие имена и так
   читаются. Комментарии только для WHY (неочевидная причина, скрытое
   ограничение, обход бага). Это общее правило проекта.

## Известные грабли (НЕ переоткрывать)

Полный список в `docs/NEXT_STEPS.md`. Сжато:

- **`keyd` стоит на dev-машине владельца** — physical keyboard захвачена
  через `EVIOCGRAB`, все события идут через `keyd virtual keyboard`
  (`/dev/input/event15`). matea читает уже с virtual. Не пытаться grab'ить
  physical.
- **`/usr/share/hunspell/ru_RU.aff` в KOI8-R**, не UTF-8. `hunspell-rs`
  принимает только UTF-8. Решение: `classifier::ensure_utf8_dicts()` при
  первом запуске конвертит в `~/.local/share/matea/dicts/`.
- **`xkb::State` не Send** — поэтому tokio current_thread + `async_trait(?Send)`.
- **`evdev::EventStream` не implement Stream** — использовать
  `stream.next_event().await` в loop, не `StreamExt::next()`.
- **`tracing-subscriber::fmt` в pipe батчит events** + ANSI escape codes
  ломают `grep` по field-name. Для streaming-наблюдения через monitor —
  писать лог в файл и `tail -F`. Можно отключить ANSI через `with_ansi(false)`.
- **`kwriteconfig6` для KDE config groups с точками** в имени (`org.kde.foo`)
  молча игнорит запись. Править файлы напрямую через editor.
- **Фоновый kill процесса под bwrap-сэндбоксом** (Flatpak, sandboxed apps)
  возвращает `exit code 144` (signal 16) в parent shell. Workaround:
  `flatpak kill <app-id>` или kill точечно по PID, не через `pkill -f`.

## Как запустить (smoke test)

```bash
# Зависимости системы (Fedora 43):
sudo dnf install libxkbcommon-devel hunspell-devel clang-devel \
                 hunspell-en-US hunspell-ru
# Группа input для evdev:
sudo usermod -aG input $USER  # перелогиниться

cd ~/matea-switcher
cargo build --release
cargo test                    # должно быть 15/15 ok

# Live smoke (читает клавиши, классифицирует слова, пишет verdict в лог):
MATEA_LOG=info ./target/release/matea > /tmp/matea.log 2>&1 &
disown
# В другом терминале — наблюдать:
tail -F /tmp/matea.log | grep WORD
# Печатать в любом окне; на каждое слово в логе появится:
#   INFO ... WORD word=hello flipped=руддщ layout_started=us verdict=KEEP
#   INFO ... WORD word=ghbdtn flipped=привет layout_started=us verdict=FLIP
```

## Что сейчас НЕ работает

- **AT-SPI** не подключен — нет различения editable-text от терминалов и
  password fields. **Не запускать matea во время ввода паролей** (matea сейчас
  видит и может переписать парольный ввод). M6 закрывает.
- **JetBrains/VSCode/Konsole** — uinput rewrite сломает completion и undo.
  Window class blacklist — часть M6.
- LLM (Verdict::Uncertain → Keep по умолчанию) — v0.2.
- Tray icon, hotkeys, config файл — M8-M9.

## Что РАБОТАЕТ сейчас (v0.1 после M5)

- Read evdev streams от всех клавиатур, async, latency <1мс.
- xkbcommon translation (keycode → glyph + layout tracking, us+ru с auto-switch
  через grp:alt_space_toggle).
- WordBuffer + word boundary detection.
- Hunspell classifier (en_US + ru_RU UTF-8 cache в `~/.local/share/matea/dicts/`).
- **uinput rewriter** — на `Verdict::Flip` физически переписывает слово в
  активном окне через virtual keyboard.
- **KWin layout switch** через D-Bus (`org.kde.keyboard /Layouts setLayout`).

## Соглашения по коммитам

- Формат заголовка: `feat(scope): что` / `fix(scope): что` / `docs: что`.
  Пример: `feat(linux): xkbcommon translation для keycode → glyph`.
- Тело — **зачем** и **какие тонкости**, не пересказ диффа.
- **Без trailer'ов с AI-coauthor.** См. правило 5 в «Что НЕ делать».
- Коммиты атомарные — один логический шаг. Тесты должны проходить на каждом
  коммите.
- Не пушить если `cargo test` красный.

## Соглашения по PR

- На текущий момент проект early-stage и single-developer (`creonuae`),
  PR-процесса нет. Прямой push в `main`. Когда станет multi-developer —
  пересмотреть.

## Как читать `docs/NEXT_STEPS.md`

Каждый milestone описан как:
- **Цель** (что хотим получить)
- **Архитектура** (как именно)
- **Подводные камни** (что уже знаем что сломается)
- **Шаги реализации** (atomic, в порядке выполнения)

Иди сверху вниз. Не прыгать через milestone — они зависят последовательно
(M5 нужен для M6, M6 → M7, и т.д.).

## Контакты

GitHub: `creonuae` — владелец проекта и единственный maintainer на текущий
момент.
