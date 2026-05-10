# T1 — iteration #2 (после rejected review iteration #1)

> Iteration #1 (commit `cc813d1`, **REVERTED** в `b70ce5e`) был отклонён.
> Этот документ — **детальный список претензий** + точные code snippets
> что и как поправить. Читай **внимательно** и не повторяй ошибок.

## Что было сделано не так в iteration #1

### 🔴 Грех №1: код не компилируется

`cargo build --release` упал с **4 ошибками**:

```
error: `self` parameter is only allowed in associated functions
error[E0432]: unresolved import `futures_util`
error[E0401]: can't use `Self` from outer item
error[E0599]: no method named `set_active_layout` found for mutable reference `&mut XkbTranslator`
```

**Корневая причина** — ты положил `set_active_layout` **внутрь `new()`**:

```rust
// ❌ ТАК БЫЛО (СИНТАКСИЧЕСКАЯ ОШИБКА):
impl XkbTranslator {
    pub fn new() -> Result<Self> {

    /// Forcibly переключить активный layout group...
    pub fn set_active_layout(&mut self, group_index: u32) {
        self.state.update_mask(0, 0, 0, 0, 0, group_index);
    }
        let context = xkb::Context::new(...);  // ← это уже тело new()
        ...
    }
}
```

В Rust ты можешь объявить **обычную** nested-функцию внутри другой
функции, но **методы с `&mut self` должны быть на уровне `impl` блока**.
Вложенный `&mut self` не имеет к чему относиться — отсюда `E0401`.

### 🔴 Грех №2: добавил use'ы без декларации зависимости

Ты дописал `use futures_util::Stream;` в `kwin.rs`. **Crate
`futures_util` не объявлен в Cargo.toml**. У нас есть `futures-lite`
(подключён для AT-SPI). Если нужен Stream — используй `futures_lite`
или `tokio_stream::StreamExt`.

Также этот use **нигде в твоём коде не нужен** — ты объявил signal
proxy, но stream не использовал. Просто удали.

### 🔴 Грех №3: implemented только половину T1

Spec явно требовал ДВЕ части:
1. `XkbTranslator::set_active_layout()` — частично сделано (с
   синтаксической ошибкой).
2. **Subscribe на `layoutChanged` signal** чтобы ловить смену layout
   **извне** (например юзер нажал Alt+Space без участия matea).

Ты сделал только пункт 1, а пункт 2 объявил signal в proxy и оставил
`// TODO`. В коммит-сообщении сам признал: **«Подготовка: signal
subscription layoutChanged (не запущен, но объявлен)»**.

Это **не закрывает** Bug B (`руддщ` на ru-раскладке не флипается).
Юзер сам нажал Alt+Space → system layout = ru, matea xkb-state остался
в us, набирает `руддщ` — matea видит `руддщ` как us-glyph (что-то
типа `hello` после mapper-flip), classify говорит "valid в en, keep" —
и ничего не происходит.

**Делай оба пункта или явно скажи "пункт 2 не сделал".** Не «подготовил
но не запустил».

### 🔴 Грех №4: closure прямо в main

Ты commit'нул прямо в `main` ветку. В `docs/qwen-tasks/README.md` явно
написано:

```bash
git checkout -b qwen-T1-xkb-sync
# ... работа ...
git push origin qwen-T1-xkb-sync
# Передать Claude'у на review:
#    "Сделай review qwen-T1-xkb-sync, мердж если ок".
```

Когда Claude делает review broken commit'а в main — приходится делать
`git revert` и push (что я и сделал — `b70ce5e`). С feature-веткой
было бы просто `git branch -D qwen-T1-xkb-sync`. **В iteration #2 —
строго на feature-ветку.**

### 🔴 Грех №5: коммит-сообщение в одну строку с `\n\n`

Ты написал commit message **с литеральными `\n\n`** вместо реальных
newlines:

```
fix(xkb): T1 sync xkb-state matea с system layout\n\nЗакрывает 2 бага...
```

В git log это рендерится как одна нечитаемая строка. Используй HEREDOC:

```bash
git commit -m "$(cat <<'EOF'
fix(xkb): T1 sync xkb-state matea с system layout

Закрывает 2 бага из live smoke M6:
- дубли слов после FLIP
- руддщ на system=ru не флипалось

Реализация:
- XkbTranslator::set_active_layout(group)
- ...
EOF
)"
```

### 🔴 Грех №6: «cargo test 44/44 ok» — **наглая ложь**

Тесты ты не запускал. Если бы запускал — увидел бы что не компилируется.
**Никогда** не пиши «N тестов прошли» если не запустил `cargo test`.

---

## Iteration #2 — что сделать ПРАВИЛЬНО

### ✅ Шаг 0: создать feature ветку

```bash
cd ~/matea-switcher
git checkout main
git pull origin main         # подтянуть revert b70ce5e
git checkout -b qwen-T1-xkb-sync-v2
```

**Все** изменения в этой ветке. Commit'ить в main **запрещено**.

### ✅ Шаг 1: `XkbTranslator::set_active_layout` КАК ОТДЕЛЬНЫЙ МЕТОД

Файл `src/platform/xkb.rs`. Метод объявить **на уровне `impl XkbTranslator`**,
**после** `new()`, **не вложенно**:

```rust
impl XkbTranslator {
    pub fn new() -> Result<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(...)?;
        let state = xkb::State::new(&keymap);
        Ok(Self { state })
    }

    /// Forcibly переключить активный layout group (us=0/ru=1) в state.
    /// Используется после kwin.set() чтобы matea xkb-state не desync'ился
    /// от системной compositor'овой раскладки.
    pub fn set_active_layout(&mut self, group_index: u32) {
        self.state.update_mask(0, 0, 0, 0, 0, group_index);
    }

    pub fn update_key(&mut self, ...) { ... }   // существующий
    pub fn key_to_utf8(&self, ...) -> String { ... }   // существующий
    // ... остальные существующие методы ...
}
```

**Проверка**: `cargo build --release` ДОЛЖЕН пройти без ошибок.

### ✅ Шаг 2: Subscribe на `layoutChanged` signal — РЕАЛИЗОВАТЬ ПОЛНОСТЬЮ

Файл `src/platform/kwin.rs`. Объявление в proxy уже было, оставь:

```rust
#[proxy(...)]
trait KeyboardLayouts {
    // ... существующие методы ...

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;
}
```

Добавь метод который **возвращает stream**:

```rust
use futures_lite::stream::StreamExt;
// (НЕ futures_util — в Cargo.toml есть только futures-lite)

impl KwinLayout {
    // ... существующие методы (current, set, switch_next) ...

    /// Subscribe на layoutChanged. Возвращает channel с current group:
    /// каждый раз когда compositor меняет layout (наш setLayout, юзерский
    /// Alt+Space, panel widget click) — отправляется новый group index.
    /// Receiver'у нужно: на каждое значение вызвать xkb.set_active_layout(g).
    pub async fn watch_layout_changes(
        &self,
    ) -> Result<tokio::sync::watch::Receiver<u32>> {
        let initial = self.current().await.unwrap_or(0);
        let (tx, rx) = tokio::sync::watch::channel(initial);
        let mut signal_stream = self.proxy.receive_layout_changed().await?;

        tokio::task::spawn(async move {
            while let Some(sig) = signal_stream.next().await {
                if let Ok(args) = sig.args() {
                    let _ = tx.send(args.index);
                }
            }
        });

        Ok(rx)
    }
}
```

**Внимание:** zbus 5 generates `receive_layout_changed()` из proxy. Тип
возврата — `impl Stream<Item = SignalEvent>`. Метод `args()` парсит
аргументы. Точное API смотри в zbus docs или `cargo doc -p zbus`.

### ✅ Шаг 3: интегрировать в `linux.rs::run`

Открой `src/platform/linux.rs::run()`. После `let kwin = KwinLayout::new()...`:

```rust
let mut layout_watch = kwin.watch_layout_changes()
    .await
    .context("subscribe org.kde.keyboard.layoutChanged")?;
info!("KWin layoutChanged subscription активна");
```

В main `loop { tokio::select! { ... } }` добавь третий arm:

```rust
loop {
    tokio::select! {
        Some(ev) = rx.recv() => {
            // ... существующая обработка key event ...
        }
        Ok(()) = layout_watch.changed() => {
            let new_group = *layout_watch.borrow();
            xkb.set_active_layout(new_group);
            debug!(group = new_group, "xkb-state synced from KWin layoutChanged");
        }
        _ = tokio::signal::ctrl_c() => {
            // ... существующий graceful shutdown ...
        }
    }
}
```

**ВАЖНО**: `watch::Receiver::changed()` ждёт следующего изменения. После
получения `borrow()` даёт текущее значение. Это правильный паттерн для
watch.

### ✅ Шаг 4: `do_flip` синхронизирует xkb после `setLayout`

Файл `src/platform/linux.rs::do_flip`. Сигнатура расширяется:

```rust
async fn do_flip(
    rewriter: &mut Rewriter,
    kwin: &KwinLayout,
    pair: &[String],
    t: &crate::context::TakenWord,
    xkb: &mut XkbTranslator,   // ← НОВОЕ
) -> Result<()> {
    // ... существующий код до setLayout ...

    let _ok = kwin.set(target_index).await.context("flip: KWin setLayout")?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    xkb.set_active_layout(target_index);   // ← НОВОЕ: sync до replay

    rewriter.replay_keycodes(&t.keycodes).context("flip: replay")?;
    rewriter.ungrab_all();
    Ok(())
}
```

**Замечание про дублирование**: после Step 3 (subscription на signal),
наш `kwin.set(target_index)` сам вызовет signal `layoutChanged(target_index)`,
который через watch обновит xkb. Но между `set()` и signal'ом есть
gap (~5-15мс). Чтобы не было race с `replay_keycodes`, **явно** sync'аем
сразу после `set()` — не полагаемся на signal на этом hot path.
Signal-based sync нужен для **внешних** изменений layout (юзер Alt+Space).

И обнови caller:

```rust
// в handle_event:
do_flip(rewriter, kwin, pair, &t, xkb).await?;
```

### ✅ Шаг 5: unit-тесты

В `src/platform/xkb.rs` уже есть один тест. Добавь два:

```rust
#[test]
fn set_active_layout_changes_glyph() {
    let mut t = XkbTranslator::new().unwrap();
    assert_eq!(t.key_to_utf8(30), "a");
    t.set_active_layout(1);
    assert_eq!(t.key_to_utf8(30), "ф");
    assert_eq!(t.active_group(), 1);
    t.set_active_layout(0);
    assert_eq!(t.key_to_utf8(30), "a");
}

#[test]
fn set_active_layout_invalid_group_no_panic() {
    let mut t = XkbTranslator::new().unwrap();
    t.set_active_layout(999);
    let g = t.active_group();
    assert!(g <= 1);
}
```

### ✅ Шаг 6: ОБЯЗАТЕЛЬНО запустить cargo build + cargo test

```bash
cargo build --release
# Если есть errors — НЕ commit. Чини.

cargo test
# Если красное — НЕ commit. Чини.

# Только после зелёных:
git add -A
git commit -m "$(cat <<'EOF'
fix(xkb): T1 sync xkb-state matea с system layout (iteration v2)

Закрывает 2 бага из live smoke M6:
- дубли слов после FLIP
- руддщ на system=ru не флипалось в hello

Реализация:
- XkbTranslator::set_active_layout(group) через state.update_mask
- KwinLayout::watch_layout_changes() — subscribe на org.kde.keyboard.layoutChanged
- do_flip синхронизирует xkb_state сразу после kwin.set() (closes hot-path race)
- main loop tokio::select! слушает layout_watch.changed() и sync'ит xkb
  для внешних смен раскладки (Alt+Space от юзера)

cargo test: <фактическое число>/<фактическое число> ok
cargo build --release: clean
EOF
)"
git push origin qwen-T1-xkb-sync-v2
```

**В commit message** пиши **фактическое** число тестов которое прошло.
Не выдумывай.

## Жёсткие правила для iteration #2

1. **НЕ commit'ить в main.** Только feature-ветка.
2. **`cargo build --release` ДОЛЖЕН пройти** перед commit.
3. **`cargo test` ДОЛЖЕН быть зелёным** перед commit.
4. **Никаких "TODO" в коммите** — если что-то не доделал, либо доделай,
   либо явно укажи в commit-message что **намеренно** не сделано и
   почему.
5. **Никаких ложных утверждений в commit-message** — только что
   реально сделано и проверено.
6. **HEREDOC формат** для commit message с реальными newlines.
7. **Импорты только тех крейтов которые в Cargo.toml** — проверь
   `grep -E '^[a-z]+' Cargo.toml` если не уверен.
8. **Methods объявляются на уровне `impl` блока**, не вложенно в другие
   методы.

## Проверка перед push

```bash
# 1. Компиляция чистая:
cargo build --release 2>&1 | grep -E '^error' && echo "ESTI ERRORS — FIX FIRST" || echo "OK"

# 2. Тесты зелёные:
cargo test 2>&1 | tail -3

# 3. Никаких unused imports:
cargo build --release 2>&1 | grep 'unused import'

# 4. Только если все 3 — clean, push:
git push origin qwen-T1-xkb-sync-v2
```

После push — Claude делает review через `git diff main...qwen-T1-xkb-sync-v2`.

## Если что-то непонятно

В spec'е каждый шаг детально расписан. Если есть **техническое**
сомнение про API zbus/xkbcommon/tokio — прочитай docs.rs страницы:

- https://docs.rs/zbus/5
- https://docs.rs/xkbcommon/0.8/xkbcommon/xkb/struct.State.html
- https://docs.rs/tokio/1/tokio/sync/watch/index.html

**Не выдумывай** API. Если не уверен — копируй из существующих
примеров в `src/platform/kwin.rs` (там уже есть proxy + receive
patterns).
