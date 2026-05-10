# Milestone 6 — AT-SPI focus tracking + password/blacklist guard

> Дата: 2026-05-10. Закрывает критичный safety blocker — раньше matea
> переписывала **в любом окне**, включая ввод паролей и IDE.

## Цель

При каждом FLIP-action знать:
1. **Не пишет ли юзер пароль** → пропустить rewrite.
2. **Активное окно — не terminal/IDE** → пропустить rewrite (там rewrite ломает completion и undo).

Источник информации — AT-SPI accessibility bus (`org.a11y.Bus`).

## Архитектура

```
[atspi listener task]                        [main loop]
     │                                            │
     │ subscribe StateChangedEvent                │
     │   (focused state)                          │
     │                                            │
     │ ───► Event приходит                        │
     │     ├─ throttle 50мс (alt-tab burst)       │
     │     ├─ AccessibleProxy.get_role()          │
     │     ├─ walk parents → Application.name()   │
     │     └─ build FocusContext                  │
     │                                            │
     │ tx.send(FocusContext) ─────────────► rx.borrow()
     │                                            │
                                                  │ if Verdict::Flip:
                                                  │   if !focus.allows_flip():
                                                  │     skip rewrite (log debug)
                                                  │   else:
                                                  │     do_flip(...)
```

Связь между task и main loop — `tokio::sync::watch::channel`. Receiver
читается **без блокировки** (`borrow().clone()`), поэтому handle_event на
hot-path не платит за shared state.

## WHY graceful degradation

Если AT-SPI bus не поднята (`org.a11y.Bus` отсутствует, например на
minimal GNOME сессии без accessibility) — listener exit'ит сразу с
warning, watch остаётся с дефолтным `FocusContext` где
`allows_flip() == true`. matea продолжает работать как M5d (без window
context), не падает.

Аналогично: per-event errors (например приложение ответило ошибкой на
`get_role`) → debug log, listener продолжает крутиться.

## WHAT — реализовано

### `src/platform/atspi.rs`

- `FocusContext { window_class, is_password }` — текущее состояние фокуса.
- `allows_flip()` — true если **не** password и **не** в blacklisted
  window class.
- `spawn_listener()` → `watch::Receiver<FocusContext>`. Спавнит фоновую
  task через `tokio::task::spawn`.
- `run_listener()` — async loop:
  - `AccessibilityConnection::new().await` (с graceful degradation).
  - `register_event::<StateChangedEvent>().await` (без него stream молчит).
  - `event_stream()` → loop через `futures_lite::StreamExt::next`.
  - На каждый `Event::Object(ObjectEvents::StateChanged(sc))` где
    `sc.state == State::Focused && sc.enabled` — `handle_focus`.
- `handle_focus()`:
  - Build `AccessibleProxy` из `sc.item.name()` + `sc.item.path()`.
  - `get_role()` → если `Role::PasswordText` → is_password = true.
  - Walk parents до `Role::Application`, оттуда `name()` → window_class
    (lowercased для compare).
- `is_blacklisted_class()` — статический список substring-match'ов:
  - **Terminals:** konsole, yakuake, alacritty, kitty, wezterm,
    gnome-terminal, xterm, tilix, terminator.
  - **IDE:** code, codium, vscodium, jetbrains, intellij, pycharm,
    webstorm, rider, clion, sublime_text.
  - **Sec-sensitive:** keepassxc, bitwarden, 1password.

### `src/platform/linux.rs::run`

- `let focus_rx = spawn_atspi_listener();` сразу после rewriter+kwin.
- На каждом event в main loop:
  ```rust
  let focus_ctx = focus_rx.borrow().clone();
  handle_event(..., &focus_ctx, ...).await
  ```
- В `handle_event` на `Verdict::Flip`:
  ```rust
  if !enabled            → skip + history.push(оригинал)
  else if !focus.allows_flip() → skip + log "FLIP suppressed (focus blocks)"
  else                   → do_flip + history.push(flipped)
  ```

## WHY ограничения текущей реализации

### Password detection — только по `Role::PasswordText`

Стандартный AT-SPI имеет state `STATE_PROTECTED` (24) для скрытых полей.
Но в crate `atspi-common 0.14` (поверх которого работает atspi 0.30)
**этого варианта в `State` enum нет** (проверено эмпирически — полный
список 44 variants без Protected). Поэтому полагаемся на role:
- Qt `QLineEdit::EchoMode::Password` → role = PasswordText ✓
- GTK `Entry::set_visibility(false)` → role = PasswordText ✓
- HTML `<input type="password">` в Firefox → role = PasswordText ✓
- HTML password в Electron/Chromium → нестабильно (зависит от accessibility
  tree включения). Дополнительная защита — blacklist по window class
  (если приложение в нашем blacklist'е, мы не флипаем независимо от
  role).

### Window class через `Application.name()`

Это не строго `WM_CLASS`, а accessibility-уровневое имя приложения. На
Plasma 6 для большинства apps совпадает с resource class (например
"konsole", "firefox", "telegram-desktop"). Для некоторых может отличаться
(Electron-апп может репортить generic "chromium"). Если нужно строго
WM_CLASS — добавить fallback через KWin DBus `getWindowInfo` с UUID
активного окна. Не сделано в M6 — Application.name достаточно для основных
случаев.

### Throttling 50мс

Plasma alt-tab генерирует burst events за ~30мс. 50мс debounce на
listener-level пропускает дубликаты. В худшем случае пропустим
актуальный focus event если юзер очень быстро переключает окна — на
следующее событие подхватится. Acceptable trade-off.

### Не реализовано: `EditableText.delete_text + insert_text`

В первоначальном плане M6 было: использовать AT-SPI editable-text
интерфейс для замены текста (сохраняет undo стек приложения, не ломает
focus, и т.д.). Это **не сделано в M6** — оставлено в **M6b**:
- Нужна проверка что widget supports `Interface::EditableText`.
- Нужно конвертить word/positions в byte offsets корректно для UTF-8.
- Тестировать на разных приложениях (gedit, Firefox, Telegram) что
  replace отрабатывает + undo работает.

Текущий M6 просто **запрещает** rewrite в опасных контекстах. Для
безопасных контекстов используется uinput-rewrite из M5d (с EVIOCGRAB
атомарностью). Это уже **сильно лучше** чем M5d без AT-SPI.

## Тесты

9 новых unit-тестов в `src/platform/atspi.rs::tests`:
- `blacklist_terminals`
- `blacklist_ide`
- `blacklist_password_managers`
- `blacklist_normal_apps_pass` (firefox/telegram-desktop/kate/gedit)
- `empty_class_passes`
- `focus_context_allows_normal`
- `focus_context_denies_password`
- `focus_context_denies_terminal`
- `focus_context_default_allows`

Total cargo test: **42/42 ok**.

## Что осталось (M6b)

- [ ] EditableText replace (вместо backspace+uinput) где интерфейс
      поддерживается. Сохраняет undo, не ломает focus.
- [ ] WM_CLASS через KWin DBus `getWindowInfo` как fallback (Electron-апп
      обманно репортит).
- [ ] Configurable blacklist через `cfg.blacklist.window_classes`. Сейчас
      hardcoded const. Поле в config есть в плане M9.
- [ ] STATE_PROTECTED когда (если) появится в `atspi` crate.
- [ ] Per-window memory: запоминать предпочтительную раскладку для
      конкретного window/chat (для proactive switching в M11b).
