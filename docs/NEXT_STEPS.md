# MaTea — план дальнейшей работы

> Документ описывает **что осталось сделать** для v0.1 MVP и далее. Без кода —
> только архитектурные решения, последовательность шагов и зафиксированные
> детали реализации, чтобы при возобновлении не уходить в research заново.
>
> **Last updated:** 2026-05-10

---

## Текущее состояние (snapshot)

### Что уже работает (закоммичено в main)

| Слой | Статус | Файлы |
|---|---|---|
| Cargo project skeleton | ✅ | `Cargo.toml`, `src/main.rs` |
| Trait `Platform` (?Send, current_thread runtime) | ✅ | `src/platform/mod.rs` |
| evdev async reader (через `evdev::EventStream`) | ✅ | `src/platform/linux.rs` |
| xkbcommon translator (keycode → utf8 + layout) | ✅ | `src/platform/xkb.rs` |
| WordBuffer + word-boundary detection | ✅ | `src/context.rs` |
| qwerty ↔ йцукен mapper | ✅ | `src/mapper.rs` |
| Hunspell classifier (en_US + ru_RU UTF-8 cache) | ✅ | `src/classifier.rs` |

15 unit-тестов проходят: 4 mapper + 1 xkb + 4 context + 6 classifier.

### Что **не** работает / отложено

- **uinput rewriter** (Milestone 5) — самая mясная часть. matea сейчас только
  **слушает**, ничего не **переписывает** в активное окно.
- **Proactive layout switching** (Milestone 6) — только `keep|flip` decision,
  без изменения активной системной раскладки.
- **AT-SPI** (Milestone 7) — нет интеграции, classifier работает по тексту из
  evdev-буфера, а не из реального текстового поля приложения.
- **LLM** (v0.2) — модели в проекте нет.

---

## Зафиксированные находки из smoke-тестов 2026-05-09

Эти детали — load-bearing для следующих шагов, не повторять research.

### Про систему пользователя

- Fedora 43, KDE Plasma 6 (Wayland-only).
- Установлен `keyd` (`/usr/local/bin/keyd`, PID ~38857). keyd захватывает
  физическую клавиатуру через `EVIOCGRAB` и создаёт `/dev/input/event15` —
  «keyd virtual keyboard». **Все нажатия пользователя приходят в matea
  именно с virtual keyboard**, не с physical.
- В системе ещё `/dev/input/event3` («AT Translated Set 2 keyboard») —
  это physical PS/2-клавиатура, которая закрыта `keyd`'s grab. matea видит
  её handle, но events оттуда не идут.
- Активная раскладка в системе переключается через **Alt+Space** (системная
  опция xkb через `localectl`/`xorg.conf.d`).

### Про код

- `xkb::State` в crate `xkbcommon` 0.8 содержит raw C pointer и **не Send**.
  Из-за этого:
  - tokio запущен с `flavor = "current_thread"` (одна нить async loop).
  - `trait Platform` помечен `#[async_trait(?Send)]`.
  - `XkbTranslator` живёт **только в main loop**, reader-задачи шлют
    `RawKeyEvent` в `mpsc::channel`.
- `evdev::EventStream` v0.13 **не** implement `Stream` — нужно
  `stream.next_event().await` в loop, не `StreamExt::next()`.
- `kwriteconfig6` для group с точками (`org.kde.kdecoration2`) **молча игнорит** —
  правит файл напрямую через editor.
- Hunspell ru_RU словарь Fedora — **в кодировке KOI8-R** (`SET KOI8-R` в .aff).
  hunspell-rs ожидает UTF-8. Поэтому в `classifier::ensure_utf8_dicts()`
  при первом запуске конвертим через `encoding_rs::KOI8_R` и кешируем в
  `~/.local/share/matea/dicts/`. Cache проверяется по существованию файла —
  reload только при удалении.
- `tracing-subscriber::fmt` при выводе в **non-TTY pipe** батчит events.
  Чтобы не терять при стриминге через Monitor — пишем в файл и `tail -F` его.
  ANSI escape codes в default fmt тоже мешают grep'у на field-name (`keycode=`
  превращается в `[3mkeycode[0m[2m=[0m...`).

### Про Plasma/KWin

- D-Bus name `org.kde.KWin` живёт в plasmashell-процессе (notification daemon
  тоже там).
- D-Bus name `org.kde.keyboard` есть, через него `/Layouts setLayout(int)` —
  переключает раскладку. Это будем использовать для proactive switching.

---

## Roadmap

### Milestone 5: uinput rewriter ✅

**Статус:** done на 2026-05-10. Детальный отчёт о реализации, обоснованиях
и known issues — в [`docs/milestones/M5_uinput_rewriter.md`](milestones/M5_uinput_rewriter.md).

Краткое summary:
- `src/platform/uinput.rs` — `Rewriter` через `evdev::uinput::VirtualDeviceBuilder`.
- `src/platform/kwin.rs` — `KwinLayout` через zbus proxy на `org.kde.keyboard /Layouts`.
- `src/context.rs` `WordBuffer` хранит `keycodes: Vec<u16>` для re-emit.
- На `Verdict::Flip`: backspace×N → setLayout(target) → sleep 50мс → replay keycodes.
- 14/14 unit-тестов зелёные (без unit-тестов на uinput/zbus — kernel-зависимы).

Что отложено в M5b (см. M5 doc):
- `EVIOCGRAB` для гарантии atomic rewrite (race window 50мс открыт).
- Wait for `layoutChanged` signal вместо sleep'а.
- Динамическое определение индексов раскладок через `getLayoutsList()`.
- udev rule для `/dev/uinput`.

### Milestone 5 (исторический план — оставлен для аудита)

**Цель:** при `Verdict::Flip` физически удалить N символов из активного окна
и впечатать корректную строку, плюс переключить системную раскладку.

#### Архитектура

```
on Verdict::Flip(word, layout_started):
    flipped = mapper(word, layout_started → other)
    target_layout = "ru" if layout_started == "us" else "us"

    1. EVIOCGRAB на ВСЕХ клавиатурах (event15, event3) — блокируем
       пользователя на ~50мс чтобы он не успел напечатать новый символ
       поверх нашего rewrite.

    2. Через uinput устройство:
       - KEY_BACKSPACE × len(word) → удаляет напечатанное юзером
       - выпустить flipped как последовательность keysyms через uinput.
         Сложность: uinput отправляет KEY_X, не glyph. Чтобы напечатать
         "привет" мы должны отправить KEY_G KEY_R KEY_I KEY_D KEY_T KEY_M
         (qwerty positional → ru даёт "привет"), при условии что активная
         раскладка ru.
       - Поэтому ПЕРЕД отправкой нужно убедиться что system layout = target.

    3. Если system layout != target:
       - вызвать `org.kde.keyboard /Layouts setLayout(target_index)`
       - подождать short delay (~30мс) пока compositor применит
       - тогда uinput KEY_X-events будут интерпретированы как нужные glyphs.

    4. EVIOCGRAB release.
```

#### Подводные камни

- **Конфликт с keyd**: keyd сам делает `EVIOCGRAB` на physical event3.
  Если matea тоже захочет grab — конфликт. Решение: matea grab'ит
  **только virtual** event15 (keyd output). Это единственная клавиатура
  через которую к приложениям приходят events. Не grab'им physical.
- **uinput-устройство**: создаём один раз на старте через `nix::sys::ioctl`
  (`UI_SET_EVBIT`, `UI_SET_KEYBIT`, `UI_DEV_CREATE`). Срок жизни — весь
  процесс matea. На shutdown — `UI_DEV_DESTROY`.
- **Layout switch race**: после `setLayout` нужен delay перед KEY_X-emits.
  Эмпирически 30-50мс достаточно. Если меньше — первая буква придёт в
  старой раскладке. В будущем — ждать `org.kde.keyboard.layoutChanged`
  signal вместо sleep'а.
- **Backspace ломает undo-стек** в большинстве приложений. Mitigations:
  - В Milestone 6 (AT-SPI) — где есть editable-text interface, использовать
    его вместо backspace+rewrite.
  - Принимать факт что `Ctrl+Z` после rewrite теряет историю до точки rewrite.
  - **Не делать rewrite в blacklisted приложениях** (Konsole, JetBrains,
    VSCode — там undo критичен и AT-SPI не помогает).
- **`/dev/uinput` permissions**: на Fedora `/dev/uinput` принадлежит
  `root:input`, mode `0660` (после установки udev rules). Если нет — нужен
  udev rule в `/etc/udev/rules.d/99-matea.rules`:
  ```
  KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
  ```
  В README инсталляции добавить шаг "проверь что `/dev/uinput` доступен".

#### Шаги реализации

1. Создать `src/platform/uinput.rs` — обёртка над `nix` ioctls + `OwnedFd`.
   Методы: `new()`, `send_key(KeyCode, pressed: bool)`, `send_string(&str)` —
   последний ломает строку на keycode'ы по xkb reverse-mapping.
2. Создать `src/platform/grab.rs` — wrapper для `EVIOCGRAB`/`EVIOCREVOKE`.
3. Расширить `src/platform/linux.rs::handle_event`: на `Verdict::Flip`
   собрать **TODO-action** в очередь, отдать `Rewriter` в отдельный шаг
   (НЕ из event-loop напрямую — backspace-events рискуют сами в evdev
   попасть и зациклить нас).
4. zbus client для `org.kde.keyboard` — простой call `setLayout(int)`.
5. Manual smoke: открыть Notepadqq, набрать "ghbdtn ", проверить что
   ловится "привет" + раскладка стала ru.

### Milestone 6: AT-SPI integration

**Цель:** уважать undo-стек приложения. Не везде где можно — пользоваться
editable-text replacement вместо backspace+rewrite.

- Использовать crate `atspi` (или `atspi-rs`) — pure-Rust client для
  D-Bus AT-SPI accessibility.
- Subscribe на `org.a11y.Bus.GetAddress`, `Object:TextChanged:Insert/Delete`
  events (для **chtenia** контекста, опционально).
- На каждое **получение фокуса окна** (`org.a11y.atspi.Event.Object.StateChanged`
  с `state=focused`):
  - Получить `Accessible` объект.
  - Проверить роль: `text`, `entry`, `paragraph`, `terminal`, `password text`.
  - Если `password text` (role 89) или state `STATE_PROTECTED` — добавить
    окно в **dynamic blacklist** и больше не лезть.
  - Если есть editable-text — сохранить refs для replacement.
- При `Verdict::Flip`:
  - Если у нас есть editable-text для активного поля — попробовать
    `EditableText.deleteText(start, end) + EditableText.insertText(start, replacement)`.
  - Если нет — fallback на uinput путь из Milestone 5.
- Window class blacklist (статический): Konsole, kitty, alacritty, Code, IDEA,
  PyCharm, WebStorm. Получаем через KWin `org.kde.KWin.activeWindow().resourceClass`.

#### Что AT-SPI **не закрывает**

- VSCode/Electron — accessibility tree включается флагом `--accessibility=on`,
  но events нестабильны. Полагаться нельзя.
- JetBrains IDE — Java/Swing практически не эмитит AT-SPI.
- Терминалы — text rendered через GL, AT-SPI events отсутствуют.

В этих случаях работает blacklist + статический skip rewrite.

### Milestone 7: classifier hardening ✅

**Статус:** done на 2026-05-10. Сделан вне порядка (раньше M6) — pure-функция,
не требует запуска matea, снижает blast-radius при следующих smoke-тестах.

Реализованные правила (отсекают токен ДО Hunspell-проверок и возвращают Keep):
- pure digits (`80663422514`)
- alphanumeric (`i7`, `2nd`, `H264`)
- URL/email/path (`@`, `.`, `:`, `/`, `\` внутри слова)
- mixed Latin+Cyrillic (`Telegram-чат`, `macбук`)
- capitalized + не в словарях (имена собственные `Anthropic`)

Детально — [`docs/milestones/M7_classifier_hardening.md`](milestones/M7_classifier_hardening.md).
20/20 unit-тестов зелёные.

### Milestone 7 (исторический план — оставлен для аудита)

**Цель:** убрать false-positive `Uncertain`/`Flip` на специальных токенах.

- **Pure-digit short-circuit**: если все символы — цифры → `Keep` (телефоны,
  суммы, версии). На текущий момент возвращается `Uncertain` (см. наш тест
  с "80663422514").
- **URL/email detect**: если содержит `@` или `.` уже **внутри** слова (не как
  boundary) → `Keep`. Пользователь набирает почту/домен — не трогаем.
- **Capitalized first char**: имена собственные. Bias к `Keep` если в обоих
  словарях не нашлось но первая буква большая.
- **Mixed Latin+Cyrillic в одном слове** → `Keep` всегда (это кросс-language
  токен, не опечатка).
- **Apostrophe слова**: `don't`, `it's` — apostrophe должен оставаться внутри
  слова (уже в `is_word_boundary_char`).
- **Dash слова**: `re-do`, `какой-то` — тоже внутри. Уже OK.
- **Числа с буквами**: `2nd`, `3D`, `i7` — `Keep` если есть alpha + digit.
- **Расширить уверенность через `recent_words`**: если 3 последних слова — RU
  и текущее candidate валидно в обеих раскладках → bias к RU. Это уже
  proto-LLM логика, без модели.

### Milestone 8: hotkey toggle ✅ (частично)

**Статус:** базовая часть done на 2026-05-10. Глобальный **Ctrl+Shift+M**
toggle ON/OFF реализован прямо в evdev-reader (без KGlobalAccel registration).
Детально — [`docs/milestones/M8_hotkey_toggle.md`](milestones/M8_hotkey_toggle.md).

Что закрыто:
- Tracking modifier-state (Ctrl/Shift left+right) по физическим keycodes.
- Detection Ctrl+Shift+M только на pressed event (autorepeat не toggle'ит).
- На toggle = false → `do_flip` пропускается, classifier продолжает крутиться.
- 3 unit-теста на helper-функции (всего теперь 22/22 ok).

Что осталось (M8b/M9):
- Конфигурируемый hotkey через config.toml.
- Toast-нотификация через dunst при toggle.
- Persistence enabled-state между запусками.
- Manual flip-last-word (Ctrl+Shift+L) — для случаев когда auto не сработал.

Tray icon (отдельная фича) — переехал в M9 целиком.

### Milestone 8 (исторический план — оставлен для аудита)

- KGlobalAccel binding (через D-Bus `org.kde.kglobalaccel`):
  - `Ctrl+Shift+L` — manual flip last word (если auto не сработал).
  - `Ctrl+Shift+M` — toggle matea on/off глобально.
  - `Meta+Z` — popup history dunst-style (опционально).
- Tray icon через `ksni` crate — KDE-native StatusNotifierItem.
  - 3 состояния: enabled / paused / error.
  - Меню: "Pause for 5min", "Add current window to blacklist", "Settings", "Quit".

### Milestone 9: config ✅ (минимальная часть)

**Статус:** базовая часть done на 2026-05-10. config.toml читается на старте,
auto-creates с дефолтами при первом запуске, не падает на битом TOML.
Детально — [`docs/milestones/M9_config.md`](milestones/M9_config.md).

Что закрыто:
- `src/config.rs` с serde/toml. `directories::BaseDirs` для path.
- Структура: `[general] enabled`, `[layouts] pair`, `[hotkeys] toggle`.
- 2 unit-теста (defaults round-trip, partial parse).
- Total cargo test: 27/27 ok.

Что НЕ done (M9b, отдельная итерация):
- Реальное применение полей в `Platform::run()` — сейчас config только
  загружается и логируется, поля не используются.
- Парсинг строки hotkey в (modifier_mask, keycode).
- Hot-reload через `notify` crate.
- Поля для blacklist/whitelist (нужен M6 для смысла).

### Milestone 9 (исторический план)

- `~/.config/matea/config.toml` (template уже описан в `docs/ARCHITECTURE.md`).
- `serde` + `toml` для парсинга. Уже в зависимостях.
- Hot reload через `notify` crate на изменение config-файла.
- Migration system: если schema меняется — backup старого config + apply defaults.

### Milestone 10: systemd user unit

- `~/.config/systemd/user/matea.service` с `Type=simple`, `Restart=on-failure`.
- `WantedBy=graphical-session.target`, `After=plasma-plasmashell.service`
  (чтобы не стартовать пока KWin не готов — иначе KWin DBus не будет отвечать).
- В Cargo packaging добавить шаг install: `cargo run --release -- install`
  кладёт unit и enable'ит.

---

## v0.2: LLM (Qwen-2.5-0.5B-Instruct GGUF)

**Цель:** покрыть `Verdict::Uncertain` (короткие 1-3 char слова, mixed-script,
context-dependent выбор языка).

### Архитектура

```
matea daemon
    └── llama-server (child process, llama.cpp pre-built binary)
        - listens on 127.0.0.1:8081
        - GGUF: ~/.local/share/matea/models/qwen2.5-0.5b-q4_k_m.gguf
        - load на старте matea, alive весь session
        - prompt prefix кешируется (system prompt не меняется)
```

Альтернатива: использовать `llama_cpp_2` crate (in-process binding) вместо
HTTP. Решим в Milestone 11 — HTTP проще debugged, in-process быстрее
(нет сериализации). Скорее всего HTTP, потому что:
- llama-server multi-platform (Linux/Mac/Win, статичный binary с GGUF
  download script);
- crate `llama_cpp_2` тянет CMake build на каждой системе.

### Модель

- **Qwen-2.5-0.5B-Instruct Q4_K_M** — ~400MB, multilingual (RU+EN), на CPU
  prefill короткого промпта ~30-50мс.
- Download script `scripts/download-model.sh` — `huggingface-cli` или прямой
  `curl https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/...`.
- В git модель **не** храним (LFS дорого, бессмысленно для 400MB).

### Prompt template

```
SYSTEM: Ты определяешь правильную раскладку клавиатуры. Билингв пишет на
RU+EN. Получаешь короткий контекст последних слов и кандидата. Ответь
строго одним токеном: keep если слово на правильной раскладке для своего
языка, или flip если набрано на неправильной (нужно переключить).

USER:
context: <last 5 words>
active_layout: us|ru
candidate: <word>
flipped_alternative: <word_after_mapper>

ASSISTANT:
```

GBNF grammar:
```
root ::= "keep" | "flip"
```

### Latency budget

- **Бюджет на total decision**: 100мс от word-boundary до начала rewrite.
- Из этого:
  - fast-path classifier: <1мс
  - LLM (только если `Uncertain`): ≤50мс prefill + ~10мс generation
  - rewrite (uinput): ~30мс (включая EVIOCGRAB+layout switch+keys)
- Если LLM превышает бюджет 2 раза подряд — temporarily fallback на `Keep`
  (логируется warning).

---

## v0.3: proactive layout prediction

**Цель:** matea сам выставляет раскладку **до** того как юзер начал печатать
(на основе контекста активного окна), уменьшая количество flip-перезаписей.

### Источники сигнала

1. **App-rule blacklist/whitelist** (статика):
   - `konsole`, `kitty`, `code`, `jetbrains-*` → всегда EN, no-rewrite.
   - `discord`, `telegram-desktop` → `proactive=true`, layout = last-used per
     chat.
2. **AT-SPI**: при `focus-changed` event:
   - Прочитать видимый текст активного окна (последние ~500 chars).
   - Pass в LLM с prompt: «Какой язык юзер скорее всего начнёт печатать
     дальше?» → `keep_current` / `switch_to_other`.
3. **Per-window memory**: `HashMap<window_class, RecentLayout>` (cap=20).
4. **Per-chat memory** (Telegram, Discord): если AT-SPI даёт chat title →
   `HashMap<(window_class, chat_title), RecentLayout>`.

### Алгоритм

```
on KWin activeWindowChanged(new_window):
    rules = lookup_static_rules(new_window.resource_class)
    if rules.fixed_layout:
        switch_to(rules.fixed_layout)
        return

    history = per_window_memory.get(new_window.id)
    if history.confidence > 0.8:
        switch_to(history.layout)
        return

    # uncertain — спросить LLM
    text = atspi.read_visible_text(new_window) or ""
    if text.is_empty():
        return  # no signal, leave current
    answer = llm.predict_language(text)
    switch_to(answer.layout)
    per_window_memory.update(new_window.id, answer)
```

---

## v0.4-v0.5: macOS / Windows ports

### macOS

- `core-graphics` crate: `CGEventTap` для read keys, `CGEvent.post()` для write.
- `accessibility-sys` (или `objc2-app-kit`) для AXUIElement read.
- Layout switch: `TISInputSourceRef` API (через `objc2-input-method-kit`).
- Tray: `objc2-app-kit::NSStatusBar`.
- Permissions setup в Onboarding: System Preferences → Security → Accessibility,
  Input Monitoring. Это требует UX-step.

### Windows

- `windows-rs` (Microsoft official): `SetWindowsHookEx(WH_KEYBOARD_LL, ...)` для
  read, `SendInput` для write.
- UI Automation (`IUIAutomation`) для context.
- Layout switch: `LoadKeyboardLayoutW` + `ActivateKeyboardLayout`.
- Tray: `Shell_NotifyIcon` (через `windows::Win32::UI::Shell`).
- Распространение: MSI installer через `cargo-wix`.

---

## v0.6+: Tab-completion

**Идея:** использовать ту же LLM (Qwen-0.5B или upgrade на Qwen-2.5-3B если
позволяет железо) для Tab-style автокомплита текста с учётом screen context.

- При `Tab`-нажатии в editable-поле:
  - Прочитать через AT-SPI весь видимый текст окна (≤2000 chars).
  - Прочитать буфер последних N чатов (если Telegram/Slack).
  - Sgenерировать completion.
  - Показать через ghost-text overlay (или системный preedit через text-input-v3).
  - Принять вторым Tab / Enter, отменить Esc.

Это уже отдельный feature-set (v1.0+), не блокирует v0.x.

---

## Milestone 11 ✅ — context bias через recent_words

**Статус:** done на 2026-05-10. Когда Hunspell-проверка возвращает «обе
раскладки валидны» (раньше → Uncertain), теперь смотрим на 5 последних
слов (через `WordHistory` в main loop) и biased к доминирующему языку.
Margin ≥ 2 для решения, иначе Uncertain.

Детально — [`docs/milestones/M11_context_bias.md`](milestones/M11_context_bias.md).
25/25 unit-тестов зелёные.

## Открытые вопросы (на потом)

- **Newton (Wayland-native a11y)** — в разработке. Когда станет stable, AT-SPI
  через D-Bus будет deprecated в пользу прямого Wayland protocol. Следить за
  https://blogs.gnome.org/a11y/.
- **xdg-desktop-portal InputCapture v2** — финализирован в KDE Plasma 6.5.4
  (12.2025). Может быть альтернативой evdev для read, без `EVIOCGRAB` конфликтов
  с keyd. Проверить latency.
- **Hunspell vs spellbook**: spellbook crate (pure Rust) обещает Hunspell-format
  совместимость без C-зависимости. Если выяснится что bindgen с
  hunspell-rs хрупок при cross-compile (особенно для macOS/Windows), мигрируем.
- **Custom раскладки**: пользователи с Workman/Dvorak/Colemak — наш статичный
  `mapper.rs` для них не работает. Решение: читать `setxkbmap -query` и строить
  таблицу динамически. Низкий приоритет.

---

## Где остановились (2026-05-09 → 2026-05-10)

- Локальный код собирается чисто, 15 unit-тестов зелёные.
- Manual smoke-тесты выполнены: evdev reader, xkbcommon translation, WordBuffer,
  classifier (en/ru flip detection).
- **Classifier integration в main loop сделан, но не закоммичен** — лежит
  в working tree вместе с этим документом. Следующий шаг при возобновлении —
  `git add -A && git commit && git push`.
- Следующий milestone — **5 (uinput rewriter)**.
