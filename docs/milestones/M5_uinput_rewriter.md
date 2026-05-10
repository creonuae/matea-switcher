# Milestone 5 — uinput rewriter + KWin layout switch

> Дата: 2026-05-10. Commit: `<заполнится после push>`.
>
> Это первая итерация когда matea **меняет состояние системы**, а не только
> читает. Все решения, выборы и грабли — здесь.

## Цель

При `Verdict::Flip` от классификатора:
1. Стереть слово которое юзер только что напечатал в неправильной раскладке.
2. Переключить системную раскладку.
3. Заново впечатать те же физические клавиши — теперь они дадут корректный
   текст в активном окне.

Цель пользователя: набрал `ghbdtn ` → за ~80мс в окне появилось `привет `,
без участия пользователя в переключении раскладки.

## WHY — почему именно так

### Почему uinput, а не `wtype` / `ydotool` / `xdotool`

| Опция | Минусы |
|---|---|
| `wtype` (Wayland virtual-keyboard-v1) | Fork-exec на каждое слово (медленно), плохо с не-ASCII (кириллица), не везде поддерживается приложениями. |
| `ydotool` | Делает то же что мы здесь, но через системный daemon. Лишний слой, чужой systemd-unit, лишняя зависимость. |
| `xdotool` | X11-only, на Wayland не работает. |
| **uinput напрямую** | Один long-lived virtual evdev device, kernel-level, работает с любым приложением одинаково на Wayland и X11. |

Выбран uinput. Через crate `evdev::uinput::VirtualDeviceBuilder` (он уже в
зависимостях для read, не нужно отдельный crate).

### Почему re-emit keycodes, а не char→keycode reverse

Когда пользователь набирает `ghbdtn` в US-раскладке, в evdev приходят коды
`KEY_G, KEY_H, KEY_B, KEY_D, KEY_T, KEY_N`. Compositor с активной US-раскладкой
интерпретирует их как `g, h, b, d, t, n` и шлёт в окно как `ghbdtn`.

При flip нам **не нужно** переводить строку `привет` обратно в keycodes — это
лишняя работа (плюс если у пользователя нестандартная раскладка типа
Workman/Dvorak — наш статичный `mapper.rs` сломается). Достаточно:
1. Поменять системную раскладку на RU.
2. Эмитить **те же** keycodes `KEY_G, KEY_H, KEY_B, KEY_D, KEY_T, KEY_N`.
3. Compositor уже с RU-раскладкой даст глифы `п, р, и, в, е, т` → в окно
   улетит `привет`.

Поэтому `WordBuffer` теперь хранит параллельный массив `keycodes: Vec<u16>`
(плюс к `chars: String`). При `take()` отдаём оба.

### Почему KWin DBus, а не xkb_state.update_layout

xkb-state у нас в matea — это **локальная** state-машина для translation внутри
matea. Если мы её переключим, наш log начнёт писать новые глифы — но **активные
приложения** будут продолжать использовать предыдущую раскладку (compositor об
этом не знает). re-emit'ы попадут в старую раскладку, никакого эффекта.

Нужно дёргать compositor через D-Bus. Plasma 6 экспортирует
`org.kde.keyboard /Layouts org.kde.KeyboardLayouts` с методами
`getLayout() -> u`, `setLayout(u) -> b`, signal `layoutChanged(u)`. Используем
`setLayout(target_index)`.

### Почему 50мс sleep после setLayout

`setLayout` — async-call в compositor. KWin обрабатывает, обновляет xkb-state
для всех клиентов (через `wl_keyboard.modifiers`/`wl_keyboard.keymap` events).
Если мы отправим uinput keycodes до того как клиенты получат новый keymap,
первая буква (или несколько) попадёт в старую раскладку → fizzle.

50мс — эмпирический буфер. Реально KWin обычно укладывается в 5-15мс, но 50мс
безопасно даже на нагруженной системе. В будущем (Milestone 6+) — заменим на
ожидание signal'а `layoutChanged` через zbus subscribe вместо sleep'а.

### Почему НЕ EVIOCGRAB в v0.1

Изначально планировался grab на virtual keyd-device (event15) перед rewrite,
чтобы юзер не успел напечатать новый символ посреди backspace+replay. Но:
- На системе пользователя физическую клавиатуру **уже** грабит keyd. Если
  matea grab'нет ту же event15 — конфликт (kernel может отказать или повести
  непредсказуемо).
- v0.1 принимает race: за 80мс rewrite пользователь успевает нажать максимум
  1 клавишу. Эта клавиша попадёт в начало нового слова — приемлемо.
- v0.2/M5b: попробуем `EVIOCGRAB` именно на event15 (keyd virtual) — если
  keyd сделал grab на event3 (physical), наш grab на event15 не должен
  конфликтовать. Тест отдельно.

### Защита от self-loop

Наш Rewriter создаёт новое evdev-устройство `matea virtual keyboard`. Если
matea при следующем запуске обнаружит его в `evdev::enumerate()` и подключится
читать — каждый наш emit вернётся к нам же → backspace зациклится.

Защита: в `discover_keyboards()` пропускаем устройства с именем содержащим
`"matea"`. Простой текстовый фильтр, надёжно покрывает.

## WHAT — что конкретно сделано

### Новые модули

1. **`src/platform/uinput.rs`** — `Rewriter`:
   - `new()` создаёт VirtualDevice с поддержкой keys 1..=255 (вся стандартная
     клавиатура).
   - `tap(keycode)` — emit press+release.
   - `backspace(n)` — n раз tap KEY_BACKSPACE.
   - `replay_keycodes(&[u16])` — sequence tap'ов.

2. **`src/platform/kwin.rs`** — `KwinLayout`:
   - zbus async proxy на `org.kde.keyboard /Layouts org.kde.KeyboardLayouts`.
   - `current() -> u32` (для proactive M6+, пока unused).
   - `set(index) -> bool`.

### Изменения в существующих модулях

3. **`src/context.rs`** `WordBuffer`:
   - Добавлен `keycodes: Vec<u16>` параллельный массив.
   - `push(ch, layout, keycode)` — теперь третьим параметром.
   - `pop()` чистит и keycode.
   - `take() -> TakenWord` отдаёт `{ word, layout, keycodes }`.
   - Тесты обновлены под новый API.

4. **`src/platform/linux.rs`**:
   - `run()` теперь создаёт `Rewriter::new()` и `KwinLayout::new()`.
   - `handle_event()` стал async (нужен `await` на DBus call).
   - При `Verdict::Flip` вызывается `do_flip()`:
     ```
     1. rewriter.backspace(keycodes.len())
     2. kwin.set(target_index).await
     3. tokio::time::sleep(50ms).await
     4. rewriter.replay_keycodes(&keycodes)
     ```
   - `discover_keyboards()` пропускает устройства с `"matea"` в имени.

### Новые модули в `platform/mod.rs`

```rust
#[cfg(target_os = "linux")] mod kwin;
#[cfg(target_os = "linux")] mod uinput;
```

## HOW — как это тестируется

### Unit-тесты (cargo test)

14/14 зелёные. Мы **не** пишем unit-тесты на сам Rewriter и KwinLayout —
обе зависят от kernel/D-Bus. Их покрытие — manual smoke ниже.

### Manual smoke

```bash
# Pre-requisites:
sudo dnf install libxkbcommon-devel hunspell-devel clang-devel \
                 hunspell-en-US hunspell-ru
groups | grep input  # должна быть group input
ls -la /dev/uinput   # должно быть rw для group input
                     # (если нет — udev rule в TODO ниже)

cd ~/MaTea
cargo build --release

# Запуск (фон, лог в файл):
MATEA_LOG=info ./target/release/matea > /tmp/matea.log 2>&1 &

# Тестовый ввод в отдельном окне (Notepadqq, Konsole, browser):
#   Печатай "ghbdtn " (US-раскладка)
#   Ожидаем: backspace×6 → setLayout(ru) → re-emit → "привет " в окне
#
# В логе:
#   INFO ... WORD word=ghbdtn flipped=привет verdict=FLIP
#   INFO ... FLIP: переписываю keycodes=[34,35,48,32,20,49] target_layout_index=1
```

## Live smoke-тест 2026-05-10 — что узнали

Запустили matea в Konsole-сессии (где Claude conversation шёл — не лучшая идея,
урок), напечатали серию слов. Результаты:

**Классификатор — на 5+:**
- `hello` → KEEP ✓
- `привет`, `тут`, `работает`, `лог` (русские в ru) → KEEP ✓
- `ghbdtn`, `nfv`, `yt`, `nfr`, `rjhjxt`, `ybxtuj`, `cnhfyyj`, `gjrf`, `cfv`,
  `xnj-nj`, `jy` (русские слова на us-раскладке) → **FLIP** ✓
- `helo`, `bvf`, `смотир`, `ну`, `т`, `ghjbc`, `jlbn`, `jnftn` (опечатки/обрывки)
  → UNCERTAIN (правильно — не делаем rewrite на сомнительном)

**Rewriter работал частично, упал на DBus:**
- backspace × N **отработал** (юзер видел как слова с экрана исчезали)
- `org.kde.KeyboardLayouts.setLayout` упал с `UnknownMethod 'SetLayout'`
- replay тоже не выполнился (бы`)
- Итог: на каждом FLIP юзеру стерли его слово и **не вернули** заменённое.
  Текст в Konsole превратился в кашу.

**Root cause:** zbus proxy macro по дефолту делает PascalCase из Rust fn name.
KDE экспортирует методы в camelCase. Зафиксировал явный `#[zbus(name = "...")]`
на каждом методе в `src/platform/kwin.rs` (commit `fd266aa`). После фикса —
повторить smoke в **отдельном** окне (gedit/kate/Telegram), не в Konsole.

**Урок про safety:** до M6 (window class blacklist) **не запускать matea с
открытым Claude Code в Konsole**. Risk catch-22 — если что-то ломается, юзер
не может сказать «остановись» через тот же чат, потому что его сообщения
переписываются.

## Что осталось / known issues

### TODO для следующих итераций (M5b)

- [ ] **Wait for `layoutChanged` signal** вместо 50мс sleep'а — точнее по
      времени, ниже latency на быстрых машинах.
- [ ] **EVIOCGRAB** на keyd virtual во время rewrite — race window закрыть.
- [ ] **udev rule для /dev/uinput**: если у нового пользователя нет rw-доступа,
      matea упадёт на `Rewriter::new()`. Положить
      `/etc/udev/rules.d/99-matea.rules` (опубликовать в README install).
- [ ] **Узнать индекс ru/us динамически** — сейчас hard-coded `us=0, ru=1`.
      Должно совпасть с порядком в `kxkbrc LayoutList`. Через `getLayoutsList()`
      из `org.kde.KeyboardLayouts` в init и кешировать map `name → index`.
- [ ] **Защита от FLIP-loop**: если наш rewrite сам триггернет classifier
      (хотя discover пропускает self-device, edge case остаётся), нужно
      tracking что текущее слово — это наш re-emit, не юзер.
- [ ] **Не стирать boundary char**: сейчас boundary (space/punct) который юзер
      ввёл — остаётся, переписанное слово ставится **перед** ним. Это правильно
      по семантике («привет », не «привет», после `ghbdtn `). Edge case: если
      боundary это `/` или `.` посреди URL/email — может мешать. Тестировать.

### Известные ограничения v0.1

- Не работает в **JetBrains IDE** / **VSCode** / **Konsole** — там uinput
  rewrite поломает completion/undo. Pure-evdev/uinput не различает «editable
  text field» от «терминал». Решение — Milestone 6 (AT-SPI + window class
  blacklist).
- **Password fields**: matea читает evdev и видит **все** нажатия, включая
  пароли. Сейчас они попадают в WordBuffer, классифицируются и могут быть
  переписаны (плохо, очень плохо). До M6 (AT-SPI STATE_PROTECTED detection) —
  **не запускать matea во время ввода паролей**. Документировать в README.
- Нет hotkey toggle on/off (M8). Чтобы выключить — `pkill matea`.

## Связанные файлы

- Код: `src/platform/uinput.rs`, `src/platform/kwin.rs`, `src/platform/linux.rs`,
  `src/context.rs`.
- Общий план: `docs/NEXT_STEPS.md` (Milestone 5 раздел расширяется этим
  документом).
- Архитектура: `docs/ARCHITECTURE.md` (поток данных через Rewriter добавлен).

## Что дальше (Milestone 6 кратко)

AT-SPI integration:
1. Subscribe на focus events.
2. Получать `Accessible` для активного поля, проверять `STATE_PROTECTED`
   (password) → dynamic blacklist.
3. Где есть `EditableText` interface — заменять backspace+uinput на
   `deleteText + insertText` (сохраняет undo стек приложения).
4. Window class blacklist (Konsole/code/jetbrains-*) через KWin DBus
   `org.kde.KWin /KWin activeWindow`.

Подробности — `docs/NEXT_STEPS.md → Milestone 6`.
