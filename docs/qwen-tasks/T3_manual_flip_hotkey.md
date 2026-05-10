# T3 — Manual flip-last-word hotkey (Ctrl+Shift+L)

## Контекст

В smoke M6 юзер заметил что `руддщ` (на ru-раскладке) **не флипнулся** в
`hello`. Корень — xkb-state desync (закрывается T1). Но даже после T1
будут случаи когда auto-flip не сработал (UNCERTAIN на короткиx словах,
неудачный context, edge case'ы). Юзер должен иметь **manual escape
hatch** — нажать одну hotkey и вручную флипнуть последнее напечатанное
слово.

Plus: после T1+T2, если Konsole заблокирован by AT-SPI guard, юзер всё
равно может **захотеть** flip в Konsole осознанно (например в комментарии
кода). Manual hotkey даст это в обход всех guard'ов.

## Цель T3

Реализовать второй hotkey `Ctrl+Shift+L` (configurable) который при
нажатии:
1. Берёт **последнее напечатанное слово** (или текущее в WordBuffer
   если оно непустое).
2. Применяет mapper-flip независимо от classifier verdict.
3. Делает do_flip — тот же путь что auto: backspace + setLayout + replay.
4. **Игнорирует FocusContext blacklist** — это manual override, юзер
   осознанно нажал hotkey, скорее всего в IDE/Konsole где auto не
   разрешён.
5. Игнорирует `enabled=false` toggle (Ctrl+Shift+M off-state) — manual
   action всегда работает.

## API Contract

### Изменение 1: новое config-поле

В `src/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotkeys {
    #[serde(default = "default_toggle_hotkey")]
    pub toggle: String,

    /// Manual flip-last-word: руками флипнуть последнее напечатанное
    /// слово. Bypasses classifier и blacklist.
    #[serde(default = "default_manual_flip_hotkey")]
    pub manual_flip: String,
}

fn default_manual_flip_hotkey() -> String { "Ctrl+Shift+L".into() }
```

Тест:
```rust
#[test]
fn config_default_includes_manual_flip() {
    let c = Config::default();
    assert_eq!(c.hotkeys.manual_flip, "Ctrl+Shift+L");
}

#[test]
fn config_partial_uses_manual_flip_default() {
    let s = r#"
        [hotkeys]
        toggle = "Pause"
    "#;
    let c: Config = toml::from_str(s).unwrap();
    assert_eq!(c.hotkeys.toggle, "Pause");
    assert_eq!(c.hotkeys.manual_flip, "Ctrl+Shift+L");
}
```

### Изменение 2: tracking last-typed-word

`WordHistory` в context.rs хранит последние N слов (только String). Для
manual flip нужно ещё **keycodes** — для re-emit без char→keycode reverse.

Расширить — отдельный struct `LastTypedWord` в main loop state:

```rust
// src/platform/linux.rs::run() local state
let mut last_typed: Option<crate::context::TakenWord> = None;

// На каждый WORD-event (после classify) обновляем:
last_typed = Some(t.clone());  // даже если verdict был Keep — для
                               // случая когда юзер хочет manual flip
                               // на "правильное" слово.
```

`TakenWord` уже имеет `{ word, layout, keycodes }` (после T1 не
меняется). Cloneable.

### Изменение 3: detection manual_flip hotkey в main loop

```rust
let toggle_hotkey = config::Hotkey::parse(&cfg.hotkeys.toggle)?;
let manual_flip_hotkey = config::Hotkey::parse(&cfg.hotkeys.manual_flip)?;

// В main loop:
mod_state.update(&ev);
if check_toggle_hotkey_v2(&ev, &mod_state, &toggle_hotkey) {
    enabled = !enabled;
    info!(enabled, "matea toggle через {}", cfg.hotkeys.toggle);
    buffer.take();
    continue;
}
if check_toggle_hotkey_v2(&ev, &mod_state, &manual_flip_hotkey) {
    if let Some(t) = &last_typed {
        info!(word = %t.word, "MANUAL FLIP requested");
        // Bypass enabled, bypass focus.allows_flip — manual это manual.
        do_flip(rewriter, kwin, &cfg.layouts.pair, &mut xkb, t).await?;
        // После manual flip — обновим last_typed на flipped word
        // (чтобы повторное Ctrl+Shift+L флипнуло снова, циклически).
        let flipped = match t.layout.as_str() {
            "us" => crate::mapper::en_to_ru(&t.word),
            "ru" => crate::mapper::ru_to_en(&t.word),
            _ => t.word.clone(),
        };
        let other_layout = if t.layout == "us" { "ru" } else { "us" };
        last_typed = Some(crate::context::TakenWord {
            word: flipped,
            layout: other_layout.into(),
            keycodes: t.keycodes.clone(),
        });
    } else {
        debug!("MANUAL FLIP requested but no last_typed available");
    }
    continue;
}
```

**ВАЖНО:** функция `check_toggle_hotkey_v2` уже есть и работает с любым
`config::Hotkey`. Её reuse'ить.

### Изменение 4: пометить last_typed после WORD-event

В существующем `handle_event`, когда мы делаем `buffer.take()` и
classify:

```rust
let t = buffer.take();
let verdict = classifier.classify(...);

// ... existing logic ...

// НОВОЕ: запомнить TakenWord для возможного manual flip позже.
*last_typed_ref = Some(t.clone());
```

Это требует pass `last_typed: &mut Option<TakenWord>` в handle_event. Или
вернуть `TakenWord` из handle_event и обновить в caller.

**Qwen:** выбрать чище. Вариант с `&mut` параметром OK.

## Тесты

### Unit (обязательны)

`src/config.rs::tests`:

```rust
#[test]
fn hotkey_parse_ctrl_shift_l() {
    let h = Hotkey::parse("Ctrl+Shift+L").unwrap();
    assert!(h.ctrl);
    assert!(h.shift);
    assert!(!h.alt);
    assert!(!h.meta);
    assert_eq!(h.keycode, 38); // KEY_L = 38 в evdev
}

#[test]
fn config_default_manual_flip() {
    let c = Config::default();
    assert_eq!(c.hotkeys.manual_flip, "Ctrl+Shift+L");
}
```

`src/platform/linux.rs::tests`:

```rust
#[test]
fn manual_flip_hotkey_distinct_from_toggle() {
    let toggle = config::Hotkey::parse("Ctrl+Shift+M").unwrap();
    let manual = config::Hotkey::parse("Ctrl+Shift+L").unwrap();
    let state = ModState { ctrl: true, shift: true, ..Default::default() };

    // KEY_M (50) → toggle, не manual
    assert!(check_toggle_hotkey_v2(&ev(50, true), &state, &toggle));
    assert!(!check_toggle_hotkey_v2(&ev(50, true), &state, &manual));

    // KEY_L (38) → manual, не toggle
    assert!(!check_toggle_hotkey_v2(&ev(38, true), &state, &toggle));
    assert!(check_toggle_hotkey_v2(&ev(38, true), &state, &manual));
}
```

### Manual smoke

1. Запустить matea-switcher.
2. Набрать `hello ` (us-раскладка) — verdict=Keep, ничего не
   переписывается.
3. Нажать **Ctrl+Shift+L** — `hello ` должно стать `руддщ ` (manual
   flip).
4. Нажать ещё раз **Ctrl+Shift+L** — `руддщ ` обратно в `hello `
   (циклично — manual flip всегда переключает).
5. В Konsole (где auto-flip заблокирован T2): набрать `ghbdtn `, нажать
   `Ctrl+Shift+L` — должно стать `привет ` (manual override blacklist'а).
6. С `enabled=false` (Ctrl+Shift+M): manual flip всё равно работает.

## Что НЕ менять

- НЕ трогать `do_flip` логику, EVIOCGRAB, replay.
- НЕ трогать AT-SPI или KWin модули.
- НЕ трогать классификатор.
- НЕ трогать что Ctrl+Shift+M делает (он остаётся toggle ON/OFF).

## Файлы которые меняем

- `src/config.rs` — добавить `Hotkeys::manual_flip` field + 2 unit-теста.
- `src/platform/linux.rs`:
  - parse `manual_flip_hotkey` на старте.
  - tracking `last_typed: Option<TakenWord>`.
  - check + execute manual flip в main loop.
  - +1 unit-тест.

## Что в коммит-сообщении

```
feat(linux): T3 manual flip-last-word hotkey (Ctrl+Shift+L)

Manual escape hatch когда auto-flip не сработал (UNCERTAIN, edge case,
blacklisted окно). Юзер нажимает Ctrl+Shift+L → matea флипает последнее
напечатанное слово, bypassing classifier + focus blacklist + enabled
toggle.

Реализация:
- config.hotkeys.manual_flip (default "Ctrl+Shift+L"), парсится так же
  как toggle через config::Hotkey::parse().
- В main loop track last_typed: Option<TakenWord>. Обновляется на
  каждом word-boundary после classify.
- Detection через check_toggle_hotkey_v2 (reuse существующей функции).
- При срабатывании — do_flip напрямую, игнорируя enabled/focus.allows_flip.
- После flip — last_typed обновляется на flipped word, чтобы повторный
  Ctrl+Shift+L циклически переключал.

3 unit-теста (config default, parse Ctrl+Shift+L, distinct from toggle).
Manual smoke verification: см. T3 spec.
```

## Подсказки

- `KEY_L = 38` в evdev (см. `src/config.rs::parse_keyname`).
- `check_toggle_hotkey_v2(&ev, &mod_state, &hotkey)` — уже работает,
  переиспользовать.
- Для tracking last_typed — `Option<TakenWord>` достаточно, не нужна
  персистенция между запусками (manual flip только в текущей сессии).
- `TakenWord` уже Clone (см. `src/context.rs`).
