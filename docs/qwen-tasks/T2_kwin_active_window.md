# T2 — KWin DBus active-window detection (закрывает AT-SPI слепоту к Konsole)

## Контекст

В M6 (`src/platform/atspi.rs`) реализован listener AT-SPI focus events
который строит `FocusContext { window_class, is_password }`. Главный
loop проверяет `focus.allows_flip()` перед каждым FLIP и пропускает
rewrite если активное окно — terminal/IDE/password manager.

**Ограничение:** Konsole, Yakuake, Alacritty, kitty и другие GL-rendered
terminals **не emit AT-SPI focus events** (их UI не a11y-exposed).
matea получает focus event только при переключении на a11y-aware
приложение (Firefox/Telegram/gedit), а на возврат в Konsole — нет события.
В результате `FocusContext` сохраняет последнее значение, и blacklist
по window_class в Konsole **не срабатывает** — matea переписывает в
Konsole, что ломает shell completion.

В live smoke M6 видно: юзер печатал в Konsole, FLIP'ы проходили без
"FLIP suppressed" в логе.

## Цель T2

Добавить **fallback detection активного окна через KWin DBus**, который
работает для ВСЕХ окон Plasma 6 (включая Konsole). Это становится
**вторым источником** информации для FocusContext — параллельно с
AT-SPI listener'ом.

## Решение архитектурное

KWin Plasma 6 НЕ предоставляет прямого DBus method'а
`getActiveWindow().resourceClass`. Но через **KWin Scripting API** это
возможно: `org.kde.KWin.Scripting.evaluateScript("workspace.activeWindow.resourceClass")`.

### Вариант A: Polling через KWin Scripting (рекомендуется)

Создать background task которая каждые 200мс дёргает KWin Scripting:
```js
print(workspace.activeWindow.resourceClass);
```
Парсит output, обновляет shared state (`window_class_kwin`).

**Плюсы:** работает для любых окон Plasma 6 (terminal/Wayland-native/Xwayland).

**Минусы:** 200мс latency между сменой окна и обновлением focus state.
Это приемлемо — юзер не печатает first word в течение 200мс после
переключения окна.

### Вариант B: Subscribe на signal `org.kde.KWin.activeWindowChanged`

Если такой signal есть в Plasma 6 — лучше polling'а. Нужно проверить:

```bash
busctl --user introspect org.kde.KWin /KWin | grep -i 'signal.*active'
```

Если signal есть — использовать его, polling не нужен.

**Qwen:** проверь оба варианта на dev-машине (если есть доступ через
busctl) ИЛИ имплементируй Вариант A (polling) и оставь TODO про
signal-based вариант.

## API Contract

### Изменение 1: расширить `KwinLayout` (или новый struct)

В `src/platform/kwin.rs` добавить:

```rust
impl KwinLayout {  // или новый KwinWorkspace struct
    /// Получить resource class активного окна через KWin Scripting eval.
    /// Возвращает None если scripting не сработал (нестандартная Plasma
    /// конфигурация) или активного окна нет.
    pub async fn active_window_class(&self) -> Result<Option<String>> {
        // Через self.proxy.connection() построить proxy на /Scripting
        // (org.kde.KWin.Scripting), вызвать evaluateScript().
        // Внимание: evaluateScript() asynchronous и НЕ возвращает
        // результат напрямую — нужен trick через registerCallback или
        // через временный print() + capture stdout.
        //
        // Альтернатива через `loadScript` + `org.kde.kwin.Script`:
        //   1. write скрипт в /tmp/matea-active-window.js
        //   2. loadScript path → script_id
        //   3. start(script_id)
        //   4. результат через signal printError / log
        //
        // Самое простое — через `dbus-send` style chain работает плохо
        // в zbus. Проверить какой API есть.
    }
}
```

Если KWin Scripting слишком неудобен — использовать **прямой proxy
на `/Workspace`**:
```
org.kde.KWin.Workspace.activeWindow → ObjectPath
org.kde.KWin.Window.resourceClass на этом ObjectPath → String
```
Может быть проще через zbus.

**Qwen:** выбрать самый рабочий путь, написать в comment-секции какой
выбран и почему.

### Изменение 2: интегрировать в FocusContext

Текущий `FocusContext { window_class, is_password }` обновляется только
из AT-SPI listener'а. Изменения:

```rust
// src/platform/atspi.rs — расширяем FocusContext
#[derive(Debug, Clone, Default)]
pub struct FocusContext {
    pub window_class: String,        // ← из AT-SPI
    pub kwin_window_class: String,   // ← НОВОЕ: из KWin DBus polling
    pub is_password: bool,
}

impl FocusContext {
    pub fn allows_flip(&self) -> bool {
        if self.is_password {
            return false;
        }
        // Проверяем ОБА источника. Если хотя бы один blacklisted — skip.
        if is_blacklisted_class(&self.window_class)
            || is_blacklisted_class(&self.kwin_window_class) {
            return false;
        }
        true
    }
}
```

### Изменение 3: spawn KWin polling task

В `src/platform/linux.rs::run` параллельно с AT-SPI listener:

```rust
let focus_rx = spawn_atspi_listener();
let kwin_focus_tx = focus_rx.subscribe();  // или отдельный watch
spawn_kwin_active_window_poller(kwin.clone(), kwin_focus_tx);
```

Polling task:
- Каждые 200мс дёргает `kwin.active_window_class()`.
- Если изменилось — обновляет field `kwin_window_class` в FocusContext.
- На ошибку — debug log, не паникует.

**Сложность:** `FocusContext` в `watch::channel` атомарно обновляется
двумя источниками (AT-SPI + KWin poller). Решения:
- **A.** Два отдельных watch'а: `atspi_rx` + `kwin_class_rx`. `handle_event`
  читает оба и склеивает в `FocusContext`. Проще для concurrency.
- **B.** Один watch + `Arc<Mutex<FocusContext>>` в task'е poller'а
  делает merge.

**Qwen:** выбрать A (проще, без локов).

## Тесты

### Unit на pure-функциях

Существующие тесты `is_blacklisted_class` уже есть. Добавить:

```rust
#[test]
fn focus_context_denies_via_kwin_class_when_atspi_empty() {
    // Сценарий M6 ограничения: AT-SPI молчит про Konsole, но KWin
    // poller видит konsole — должен заблокировать FLIP.
    let c = FocusContext {
        window_class: "".into(),         // AT-SPI не репортнул
        kwin_window_class: "konsole".into(),
        is_password: false,
    };
    assert!(!c.allows_flip());
}

#[test]
fn focus_context_denies_via_atspi_when_kwin_empty() {
    let c = FocusContext {
        window_class: "konsole".into(),
        kwin_window_class: "".into(),
        is_password: false,
    };
    assert!(!c.allows_flip());
}

#[test]
fn focus_context_allows_when_both_normal() {
    let c = FocusContext {
        window_class: "firefox".into(),
        kwin_window_class: "firefox".into(),
        is_password: false,
    };
    assert!(c.allows_flip());
}
```

### Manual smoke

1. Запустить matea-switcher.
2. Открыть Konsole, набрать `ghbdtn ` — FLIP должен **не сработать**
   (в логе "FLIP suppressed (focus context blocks)" с указанием
   konsole в kwin_window_class).
3. Переключиться в gedit, `ghbdtn ` → `привет`. FLIP работает.
4. Обратно в Konsole — снова blocked.

## Что НЕ менять

- НЕ трогать AT-SPI listener (он уже работает для GTK/Qt-apps).
- НЕ менять `is_blacklisted_class()` список.
- НЕ менять `Rewriter` или `do_flip`-логику.
- НЕ требовать дополнительных deps кроме того что уже есть (zbus, tokio,
  futures-lite, anyhow, tracing). Если что-то критично нужно — спросить.

## Файлы которые меняем

- `src/platform/kwin.rs` — добавить `active_window_class()`.
- `src/platform/atspi.rs` — расширить `FocusContext` доп.полем
  `kwin_window_class`, обновить `allows_flip()`.
- `src/platform/linux.rs` — spawn polling task, склеивание двух источников
  в handle_event.

## Что в коммит-сообщении

```
feat(kwin): T2 active-window detection через DBus — закрывает AT-SPI
слепоту к Konsole/terminals

В M6 AT-SPI listener не получает focus events от Konsole/Yakuake/Alacritty
(не a11y-exposed). matea переписывала в Konsole — ломала shell completion.

T2 добавляет KWin DBus polling/signal как второй источник window_class
для FocusContext. Polling каждые 200мс через
[Scripting.evaluateScript ИЛИ Workspace.activeWindow — выбрано: ...].

FocusContext теперь имеет два поля window_class (atspi) +
kwin_window_class (kwin). allows_flip() проверяет ОБА — если любой
blacklisted, skip rewrite.

3 unit-теста + manual smoke (см. T2 spec).
```

## Подсказки

- Проверка signal на dev-машине:
  ```
  busctl --user introspect org.kde.KWin /KWin | grep -iE 'signal.*active'
  ```
  Если есть `activeWindowChanged` — использовать вместо polling'а.
- KWin Scripting API доступ:
  ```
  qdbus-qt6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start
  ```
- Stream из zbus signal:
  ```rust
  let mut s = proxy.receive_active_window_changed().await?;
  while let Some(sig) = s.next().await { /* ... */ }
  ```
