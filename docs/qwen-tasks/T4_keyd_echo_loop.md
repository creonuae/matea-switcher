# T4 — Закрыть echo-loop через keyd (дубли символов в окне)

> **ПРЕЖДЕ ЧЕМ КОДИТЬ:** прочитай `docs/qwen-tasks/README.md` секцию
> «Уроки T1 iteration #1 (REJECTED)» — там 6 граблей которые нельзя
> повторять.
>
> **Branch:** `qwen-T4-keyd-echo-loop` (создать от свежего main).
>
> **Pre-push checklist:** cargo build clean, cargo test зелёный,
> HEREDOC commit message, никаких выдуманных тестовых результатов.

## Проблема

Live smoke 2026-05-10 показал: даже с EVIOCGRAB (M5d) и self-echo
suppression (M5c), юзер видит **дубли символов** в активном окне после
FLIP. Текст `автозамена не случилась.лучилась ну в целом целом
дубли лииногда происходят слов.слов` — каждое слово или его кусок
появляется дважды.

## Root cause

На системе пользователя стоит `keyd` (`/usr/local/bin/keyd`). keyd:
- Grab'ит physical keyboard (`/dev/input/event3` — AT Translated Set 2)
  через `EVIOCGRAB`.
- Создаёт virtual keyboard `keyd virtual keyboard` (`/dev/input/event15`).
- Все юзерские нажатия идут через `event3 → keyd → event15`.
- **keyd подписан на новые input devices автоматически** (его
  default behaviour — обрабатывать все клавиатуры в системе).

matea-switcher делает:
1. Создаёт `matea-switcher virtual keyboard` через uinput
   (`/dev/input/event14` или подобный новый event-номер).
2. На `Verdict::Flip` через uinput emit'ит keycodes (backspace + replay).
3. Эти events идут в **наш** event-device (event14).

**Echo loop:**
- Compositor (KWin) видит наши events на event14 → передаёт
  активному окну. Юзер видит наш rewrite.
- **keyd ТОЖЕ видит** наши events на event14 → пропускает их через
  свою processing pipeline → emits на event15.
- Compositor видит события на event15 → ещё раз передаёт окну. Юзер
  видит **дубль**.

В логах видно:
- `grab_all grabbed=1 total=2` — наш grab event3 fails (keyd там),
  grab event15 success. Это блокирует чтение matea reader'ом, но не
  блокирует **doblication через keyd → event15 → compositor → окно**.
- `103 ignored self-echo` — counter работает, но это **только** для
  matea's reader. Окно видит дубли через compositor параллельно.

## Что НЕ работает в текущей архитектуре

- Self-echo counter защищает только matea reader от self-loop, не
  закрывает дубли в окне.
- EVIOCGRAB на event15 защищает компонент окна от user input во время
  grab, но **не блокирует** наш emit через event14 + keyd echo через
  event15 (грab был ДО наших emit'ов, отжимается ДО ungrab).

## Решение — три варианта

### Вариант A (рекомендован): Suppress keyd echo через `KEY_NUMERIC` или specific naming

keyd по умолчанию обрабатывает все клавиатуры с определёнными свойствами.
Если matea virtual keyboard **не выглядит как keyboard** (не имеет
`KEY_A`, `KEY_BACKSPACE` в supported keys list?) — keyd может его
проигнорировать. Но нам **нужны** эти keys для emit'а.

Альтернатива: **имя устройства** — keyd может иметь `ignore` rules. У
автора keyd ([rvaiya/keyd](https://github.com/rvaiya/keyd)) поддерживается
конфиг с `[ids]` секцией где можно блэклистить vendor:product. Наш
uinput-устройство имеет vendor=0x0000 product=0x0000 (по дефолту).

**В этом варианте:**
1. В Rewriter::new() задать кастомные vendor/product ID (например
   `0x6d61, 0x7465` = ASCII "ma","te").
2. Документировать в README что юзеру с keyd нужно добавить в
   `~/.config/keyd/default.conf`:
   ```
   [ids]
   -6d61:7465
   ```
   (минус = ignore это устройство).

**Минусы:** требует ручного шага юзера + изменение keyd-конфига.

### Вариант B (чище, но больше работы): Выдавать events через KWin virtual-keyboard-v1

Wayland protocol `zwp_virtual_keyboard_v1` позволяет приложению
эмитить keyboard events **напрямую в compositor** без uinput-устройства.
keyd не видит этого пути.

Реализация:
1. Создать Wayland-клиент (через `wayland-client` crate или подобное).
2. Bind to `zwp_virtual_keyboard_manager_v1` global.
3. Create virtual keyboard через `create_virtual_keyboard()`.
4. Использовать `key()` method вместо uinput.

**Плюсы:**
- keyd не имеет к этому отношения.
- Cleaner architecture (без low-level uinput).
- Работает на любой Wayland-сессии где protocol поддерживается (KWin
  поддерживает).

**Минусы:**
- ~150 LOC новых.
- Требует wayland-client + protocol bindings.
- На X11 не работает (но X11 и так не наш таргет).

### Вариант C (костыль, но быстро): Detect keyd + warn юзера

Если keyd запущен — log warning и **не** делать FLIP-rewrite вообще.
Юзер использует ручной flip (Ctrl+Shift+L после T3) который тоже
страдает от echo, но реже.

**Это хак. Не делать.**

## Цель T4

Реализовать **Вариант A** (быстрее) или **Вариант B** (правильнее).
Решить может Qwen — описать в commit message **почему** выбрал.

## Если Вариант A

### Изменения

`src/platform/uinput.rs::Rewriter::new`:
```rust
let device = VirtualDeviceBuilder::new()
    .name("matea-switcher virtual keyboard")
    .input_id(BusType::USB, 0x6d61, 0x7465, 0x0001)  // vendor, product, version
    .with_keys(&keys)
    .build()
    ...
```

Точное API имени метода — проверить в `evdev::uinput`. Может быть
`with_input_id`, `vendor_id` или прямой setter.

### Документация

Создать `docs/keyd-setup.md`:
```
# matea-switcher с keyd

Если у тебя установлен keyd, добавь в `~/.config/keyd/default.conf`:

[ids]
-6d61:7465

Затем `sudo systemctl reload keyd`.

Это говорит keyd игнорировать matea-switcher virtual keyboard,
закрывая echo-loop. Без этой настройки в окне будут дубли символов
после FLIP.
```

И ссылка из README + AGENTS.md.

## Если Вариант B

### Зависимости

`Cargo.toml`:
```toml
[target.'cfg(target_os = "linux")'.dependencies]
wayland-client = "0.31"
wayland-protocols-misc = "0.3"  # для zwp_virtual_keyboard_v1
# или
wayland-protocols-wlr = "0.3"
```

### Архитектура

Новый модуль `src/platform/wayland_kbd.rs`:
- `WaylandKeyboardEmitter` struct.
- На init: connect to Wayland display, bind manager, create virtual_keyboard.
- Method `emit_key(keycode, pressed)` через protocol.
- `emit_keymap(xkb_keymap)` для передачи keymap (требуется протоколом).

Заменить use `Rewriter` на `WaylandKeyboardEmitter` в `linux.rs::do_flip`.

### Подсказки

- Документация: https://docs.rs/wayland-client/0.31/
- Protocol: `wayland-protocols-misc` имеет `zwp_virtual_keyboard_manager_v1`.
- Keymap для `zwp_virtual_keyboard_v1.keymap()`: нужно XKB keymap as
  fd to shared memory. Можно получить из текущего `XkbTranslator`.

## Тесты

### Unit

В обоих вариантах — minimal unit-тесты на pure-функции (если они есть
в новой архитектуре). Если main path требует kernel/wayland — manual
smoke.

### Manual smoke (обязательно verify)

1. Запустить matea-switcher (с keyd работающим в системе).
2. В gedit набрать `lfdfq ` (us-keycode для "давай").
3. Ожидать: в окне `давай ` ровно один раз. **Без дубля `давайдавай`**.
4. Повторить с `ghbdtn привет xnj-nj`. Дублей быть не должно.
5. Проверить чистоту: после rewrite активная раскладка ru, юзер
   продолжает на ru → нет двойных букв в новом слове.

## Что НЕ менять

- T1 v2 (xkb sync) — работает, не трогать.
- AT-SPI listener — работает, не трогать.
- EVIOCGRAB и self-echo counter в Rewriter — оставить (они закрывают
  partial cases).
- Hunspell classifier и context bias — не трогать.

## Файлы которые потенциально меняем

**Вариант A:**
- `src/platform/uinput.rs` — добавить input_id в VirtualDeviceBuilder.
- `docs/keyd-setup.md` — новый файл.
- `README.md` + `AGENTS.md` — упоминание keyd setup.

**Вариант B:**
- `Cargo.toml` — добавить wayland-client + wayland-protocols.
- `src/platform/wayland_kbd.rs` — новый модуль.
- `src/platform/mod.rs` — register.
- `src/platform/linux.rs::do_flip` — заменить Rewriter на новый emitter.
- `src/platform/uinput.rs` — оставить как fallback или удалить.

## Что в commit message

```
fix(uinput|wayland): T4 закрыть echo-loop через keyd (дубли символов)

Live smoke 2026-05-10 показал дубли каждого символа после FLIP на
системе с keyd. Root cause: keyd подписывается на наш matea virtual
keyboard и пере-эмитит наши events через свою virtual клавиатуру,
compositor видит их дважды.

Выбран вариант [A: kustom vendor/product ID + keyd setup doc | B:
zwp_virtual_keyboard_v1 protocol]. WHY [обоснование выбора].

[Изменения по файлам.]

cargo test: N passed; 0 failed
manual smoke в gedit: дубли пропали (verified)
```
