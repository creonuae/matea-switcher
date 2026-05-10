# Milestone 9c — applied hotkey + layouts.pair из config

> Дата: 2026-05-10. Закрывает TODO M9b (config полей реально применить
> в runtime, не только загрузить).

## Что сделано

### Hotkey parser (`src/config.rs`)

Новый struct `Hotkey { ctrl, shift, alt, meta, keycode: u16 }` + `Hotkey::parse(s: &str)`.

Формат строки: `Modifier+Modifier+Key`, регистр не важен. Поддержанные:
- **Modifiers:** `Ctrl/Control`, `Shift`, `Alt`, `Meta/Super/Win/Windows`.
- **Letters:** `A-Z` (через keycode-таблицу evdev — порядок не алфавитный!
  KEY_A=30, KEY_Z=44, etc.)
- **Digits:** `0-9` (top row, KEY_1=2, …, KEY_0=11).
- **F-keys:** `F1`-`F12` (KEY_F1=59, KEY_F12=88).
- **Specials:** `Space`, `Tab`, `Enter`/`Return`, `Esc`/`Escape`, `Pause`,
  `Backspace`, `CapsLock`/`Caps`.

Нет в v0.1: стрелки, NUMPAD, многомедиа-кнопки. Добавим по запросу.

WHY таблица а не xkbcommon-резолв: модификаторы и shortcut-keys должны быть
**раскладко-независимыми** (Ctrl+M в любой раскладке = Ctrl+M). evdev keycodes
именно такие. xkb keysym (например `Cyrillic_em`) меняется при переключении —
это для другого слоя.

### Modifier state в main loop

`update_modifiers()` (старая) → `ModState::update()` теперь tracking всех
4 modifier'ов: Ctrl/Shift/Alt/Meta (raw evdev keycodes 29/97, 42/54, 56/100,
125/126 — left+right варианты на каждый).

`check_toggle_hotkey_v2()` сравнивает event + ModState с распарсенным Hotkey
**точно**: все 4 модификатора должны совпадать (лишний Alt при Ctrl+Shift+M
не срабатывает).

### Layouts.pair применяется в do_flip

Раньше `do_flip` имел hardcoded `match t.layout { "us" => "ru", "ru" => "us" }`.
Теперь принимает `pair: &[String]` из `cfg.layouts.pair` и ищет первый
не-current'ый layout как target. То есть юзер может в config поставить
`layouts.pair = ["us", "de"]` и matea будет работать в паре us↔de (если
обе раскладки есть в KWin LayoutList).

В v0.1 поддерживается **пара** (2 layout'а). Если в pair 3+ — берётся
первый отличный от current. Полная поддержка multi-layout (3+) — отдельный
milestone (нужен trigram-classifier для определения **какой** язык target).

## Тесты (M9c добавил)

В `src/config.rs::tests`:
- `hotkey_parse_default` — Ctrl+Shift+M → корректный struct
- `hotkey_parse_case_insensitive`
- `hotkey_parse_meta_aliases` (Meta/Super/Win/Windows → meta=true)
- `hotkey_parse_pause` (одиночная клавиша без модификаторов)
- `hotkey_parse_unknown_key` (error path)
- `hotkey_parse_no_main_key` (только модификаторы — error)

В `src/platform/linux.rs::tests`:
- `mod_state_tracks_all_modifiers` (все 4 + left/right варианты)
- `hotkey_v2_exact_modifier_match` (лишний модификатор → не срабатывает)
- `hotkey_v2_pause_no_modifiers` (одиночная клавиша)

Total: **33/33 ok** (было 27).

## Использование (пример)

`~/.config/matea-switcher/config.toml`:
```toml
[general]
enabled = true

[layouts]
pair = ["us", "ru"]

[hotkeys]
toggle = "Pause"          # вместо дефолтного Ctrl+Shift+M
```

После рестарта matea — Pause-кнопка теперь toggle'ит rewrite ON/OFF.
Невалидная строка хоткея → matea упадёт на старте с понятным сообщением
(`invalid config.hotkeys.toggle = '...'`), не молчком ломаясь.

## Что не сделано

- **Hot reload config** — пока требуется рестарт matea при изменении config.
  Через `notify` crate можно сделать file-watcher → перепарс + apply без
  рестарта.
- **Multi-layout support (3+ раскладок)** в pair. Нужен LLM или trigram
  classifier для решения **в какой язык** flip'ить. v0.2 территория.
- **Manual flip-last-word hotkey** (`Ctrl+Shift+L`) — для случаев когда
  auto не сработал и юзер хочет ручной FLIP. Архитектура готова —
  добавить второй Hotkey field в config + вторую проверку в loop.
