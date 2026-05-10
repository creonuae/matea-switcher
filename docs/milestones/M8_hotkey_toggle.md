# Milestone 8 — global hotkey toggle (Ctrl+Shift+M)

> Дата: 2026-05-10. Сделан после M7 как **safety quick-win** перед следующим
> live-smoke. Без него — единственный способ остановить matea это `pkill`,
> что больно (теряется буфер, нужен терминал).

## Цель

Дать пользователю **аварийный switch** одной комбинацией: нажал — matea
перестала переписывать (но продолжает читать события и логировать verdict).
Нажал ещё раз — снова переписывает.

Чем не M9 (config файл): toggle нужен **сейчас** для безопасного следующего
теста; полный config (через TOML) — отдельная работа.

## WHY Ctrl+Shift+M, а не системный KGlobalAccel

В Plasma 6 регистрация global hotkey через `org.kde.kglobalaccel` D-Bus
требует `KGlobalAccelComponent` и xml-файл `.actions`. Это inflated overhead
для одной кнопки toggle и привязывает нас к Plasma. Поверх этого:

- Мы **уже читаем** все evdev keypresses глобально — добавить detection
  Ctrl+Shift+M это 5 строк кода.
- Хоткей универсальный (Linux/Wayland/X11/любой compositor).
- Если когда-то портируемся на Mac/Windows — там тоже работает (мы там читаем
  keys через CGEventTap/SendInput hook'и одинаково).

## WHAT — реализация

В `src/platform/linux.rs::run()` добавлены три state-переменные main loop'а:
```rust
let mut enabled: bool = true;
let mut ctrl_pressed = false;
let mut shift_pressed = false;
```

Перед каждым `handle_event` обрабатываем event дважды:
1. `update_modifiers(&ev, &mut ctrl, &mut shift)` — поддерживает actual состояние
   Ctrl/Shift по физическим keycodes (29/97 для Ctrl, 42/54 для Shift). Не
   используем xkb keysym — модификаторы должны работать в **любой** раскладке.
2. `check_toggle_hotkey(&ev, ctrl, shift)` — true только если pressed event
   на KEY_M (50) И оба модификатора зажаты.

Если hotkey сработал:
- `enabled = !enabled`
- `buffer.take()` — сбрасываем недо-собранное слово (иначе следующий boundary
  обработает его в новом enabled-state, что неинтуитивно).
- `info!(enabled, "matea toggle...")` в лог.

В `handle_event` появился параметр `enabled: bool`. Влияет **только** на
`Verdict::Flip` action: classify по-прежнему вызывается и логируется. Если
`enabled=false` — пишем `debug!("FLIP suppressed")` и НЕ дёргаем `do_flip`.

## HOW — тестирование

### Unit-тесты (3 новых)

В `src/platform/linux.rs::tests`:
- `modifiers_track_ctrl` — Press/release left Ctrl (29) обновляет state.
- `modifiers_track_shift` — Press left shift (42), release right shift (54).
  (Кросс-проверка что обе клавиши обрабатываются как один логический modifier.)
- `hotkey_requires_both_modifiers` — все 5 комбинаций (M alone, M+Ctrl only,
  M+Shift only, M+Ctrl+Shift, release-event с зажатыми модификаторами).

Total `cargo test`: 22/22 ok.

### Manual smoke

```bash
MATEA_LOG=info ./target/release/matea > /tmp/matea.log 2>&1 &
# В любом окне:
# 1. Печатаешь "ghbdtn " — ожидаешь FLIP в "привет ".
# 2. Нажимаешь Ctrl+Shift+M (без отпускания M пока модификаторы зажаты).
#    В логе должно появиться:
#      INFO ... enabled=false matea toggle через Ctrl+Shift+M
# 3. Печатаешь "ghbdtn " — теперь classifier пишет:
#      INFO ... WORD verdict=FLIP   (как и раньше)
#    + debug:
#      DEBUG ... word=ghbdtn FLIP suppressed (matea disabled)
#    Никакого rewrite не происходит, текст остаётся "ghbdtn".
# 4. Ctrl+Shift+M снова — возврат в active.
```

## Известные ограничения

- **`Ctrl+Shift+M` потенциально занят** в некоторых приложениях (Telegram —
  forward message, IDE — какой-то shortcut). Когда matea видит этот
  combo, она НЕ блокирует его прохождение к приложению — мы только читаем
  evdev, не grab'им. Так что приложение тоже среагирует. Если конфликт
  раздражает — поменять на `Ctrl+Alt+M` или `Pause` (которая редко используется).
  Будет в config (M9).
- **На repeat (autorepeat)** не реагируем (`pressed` приходит как value=2 для
  autorepeat, но мы фильтруем только value==1 в `read_keyboard`). Так что
  зажатый Ctrl+Shift+M не toggle'ит N раз — только на event press.
- Toggle не персистится между перезапусками matea — каждый старт начинается с
  `enabled=true`. Сохранение в config — M9.

## Что сделать в M8b / M9

- [ ] Конфигурируемый hotkey (config.toml: `toggle_hotkey = "Ctrl+Shift+M"`).
- [ ] Toast-нотификация через dunst (`notify-send "matea: OFF"`) для
      визуальной обратной связи когда сработал toggle.
- [ ] Persistence: сохранить enabled-state в файл, восстанавливать на старте.
- [ ] Manual flip-last-word hotkey (Ctrl+Shift+L) — если auto не сработал и
      юзер хочет ручной FLIP последнего слова.
