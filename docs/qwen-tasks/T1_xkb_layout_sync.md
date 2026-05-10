# T1 — Sync xkb-state matea с system layout (закрывает баги дублей)

## Контекст

В matea-switcher после `do_flip` системная раскладка переключается через
KWin DBus (`org.kde.keyboard.setLayout`), но **локальный xkb-state
matea остаётся в старой раскладке**. Это создаёт два бага:

### Баг A: дубли слов после FLIP

1. Юзер на system=us набирает `lfdfq ` (5 keycodes + space).
2. matea: classify → FLIP → backspace×5 + setLayout(ru) + replay.
   В окне появляется `давай`. system layout стал ru.
3. Юзер продолжает на system=ru набирать `тут ` (keycodes 49,18,49 +
   space).
4. xkb-state matea **думает что us** (его никто не sync'ил после
   setLayout). Видит `nen ` где должно быть `тут`.
5. classify('nen', layout='us') → `nen` невалидно в en, флип → `тут`
   валидно в ru → Verdict::Flip.
6. matea второй раз делает do_flip: backspace×3 + setLayout(ru) + replay.
   В окне получается `тут` **дважды**: первое появилось от прямого ввода
   юзера на system=ru, второе — наш ненужный rewrite.

### Баг B: `руддщ` на system=ru НЕ флипнулось в `hello`

Юзер сменил system layout на ru (через Alt+Space), набрал `руддщ`. matea
xkb-state остался в us — видит keycodes как us-glyphs (`hello` или
`hjkkj` в зависимости от точных keycodes). classify(layout='us'):
`hello` валидно в en — Verdict::Keep. Юзер не получил ожидаемый flip.

## Цель T1

Sync'нуть xkb-state matea с system layout сразу после `kwin.set(target)`
И слушать `org.kde.keyboard.layoutChanged` D-Bus signal чтобы ловить
смену системной раскладки **извне** (юзер сам нажал Alt+Space).

## API Contract

### Изменение 1: метод `XkbTranslator::set_active_layout(group: u32)`

В `src/platform/xkb.rs` добавить публичный метод:

```rust
impl XkbTranslator {
    /// Forcibly переключить активный layout group (us=0/ru=1) в state.
    /// Используется после kwin.set() чтобы matea xkb-state не desync'ился
    /// от системной compositor'овой раскладки.
    pub fn set_active_layout(&mut self, group_index: u32) {
        // Реализация через xkbcommon::xkb::State::update_mask:
        //   self.state.update_mask(0, 0, 0, 0, 0, group_index);
        // (depressed_mods=0, latched_mods=0, locked_mods=0, depressed_layout=0,
        //  latched_layout=0, locked_layout=group_index)
    }
}
```

### Изменение 2: `do_flip` синхронизирует xkb после setLayout

`src/platform/linux.rs::do_flip`:
```rust
let _ok = kwin.set(target_index).await.context("flip: KWin setLayout")?;
tokio::time::sleep(Duration::from_millis(50)).await;
xkb.set_active_layout(target_index);  // ← НОВОЕ: sync после KWin update
rewriter.replay_keycodes(&t.keycodes).context("flip: replay")?;
```

**ВАЖНО:** для этого `do_flip` теперь принимает `xkb: &mut XkbTranslator`.
Нужно изменить сигнатуру и pass-through из `handle_event`.

### Изменение 3: subscribe на `layoutChanged` signal от KWin

В `src/platform/kwin.rs` добавить:

```rust
#[proxy(...)]
trait KeyboardLayouts {
    // ... существующие методы ...

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;
}

impl KwinLayout {
    /// Subscribe на signal layoutChanged. Возвращает stream который
    /// emits каждый раз когда compositor сменил активный layout
    /// (включая Alt+Space от юзера, переключение через panel widget,
    /// и наш собственный setLayout).
    pub async fn watch_layout_changes(&self) -> Result<impl Stream<Item = u32>> {
        // Используется через self.proxy.receive_layout_changed().await
        // и .map(|sig| sig.args()?.index()) или подобное.
    }
}
```

И в `src/platform/linux.rs::run` запустить эту подписку как **задачу
которая отдаёт events в watch::channel**, главный loop читает текущий
group и обновляет xkb перед каждым `handle_event`. Либо через `select!`
интегрировать прямо в main loop.

Решение архитектуры на усмотрение Qwen — главное чтобы:
- xkb_state matea **всегда** соответствовал реальному system layout.
- Не было дублей FLIP'ов.
- Юзерский Alt+Space (внешняя смена) тоже подхватывался.

## Тесты

### Unit (обязательны)

В `src/platform/xkb.rs::tests`:

```rust
#[test]
fn set_active_layout_changes_glyph() {
    let mut t = XkbTranslator::new().unwrap();
    // Изначально group 0 (us): KEY_A → "a"
    assert_eq!(t.key_to_utf8(30), "a");

    // Переключаем на group 1 (ru):
    t.set_active_layout(1);
    assert_eq!(t.key_to_utf8(30), "ф"); // KEY_A в ru = "ф"
    assert_eq!(t.active_group(), 1);

    // Обратно в us:
    t.set_active_layout(0);
    assert_eq!(t.key_to_utf8(30), "a");
}

#[test]
fn set_active_layout_invalid_group_no_panic() {
    let mut t = XkbTranslator::new().unwrap();
    t.set_active_layout(999); // только us+ru настроены, group 999 нет
    // Не должно паниковать — xkbcommon clamps или ignores. После вызова
    // active_group() должен вернуть валидное значение (0 или 1).
    let g = t.active_group();
    assert!(g <= 1);
}
```

### Manual smoke (не unit-тест, упомянуть в PR description)

1. Запустить matea-switcher.
2. Открыть gedit (или любое window НЕ в blacklist).
3. Набрать `lfdfq ` (us-раскладка) → должно стать `давай ` ровно один раз.
4. Сразу после этого набрать `nen ` (на текущей теперь ru) → ожидаем
   `nen` не флипается (т.к. на ru это `тут`, валидное русское) и в окне
   появится `тут` ровно один раз. **Без дублей.**
5. Нажать Alt+Space (юзер сам сменил на us). Без перезапуска matea.
6. Набрать `hello ` → KEEP, `hello` остаётся в окне.
7. Набрать `ghbdtn ` → FLIP в `привет`.

Цель: нет дублей слов, layout tracker matea всегда соответствует
системе.

## Что НЕ менять

- НЕ трогать `src/classifier.rs`, `src/context.rs`, `src/mapper.rs`,
  `src/config.rs` — они не связаны с xkb-state sync.
- НЕ трогать `src/platform/atspi.rs` — отдельная задача (T2).
- НЕ менять формат конфига.
- НЕ трогать `Rewriter::backspace/replay_keycodes` — они работают
  правильно.

## Файлы которые меняем

- `src/platform/xkb.rs` — добавить `set_active_layout()` + 2 unit-теста.
- `src/platform/kwin.rs` — добавить signal subscription.
- `src/platform/linux.rs` — `do_flip` sync xkb + подписка на layout signal.

## Что в коммит-сообщении

```
fix(xkb): T1 sync xkb-state matea с system layout

Закрывает 2 бага из live smoke M6 (см. docs/qwen-tasks/T1_xkb_layout_sync.md):
- дубли слов после FLIP (matea видела current keycodes в старой раскладке
  и второй раз флипала уже-флипнутое слово)
- руддщ на system=ru не флипалось в hello (matea xkb думал us)

Реализация:
- XkbTranslator::set_active_layout(group) через xkb_state.update_mask
- do_flip синхронизирует xkb_state matea сразу после kwin.set()
- Subscribe на org.kde.keyboard.layoutChanged signal — ловим Alt+Space
  смены раскладки извне (без участия matea)

Manual smoke verification: см. T1 spec.
cargo test 44/44 ok (+2 новых).
```

## Подсказки

- API xkbcommon `State::update_mask` принимает 6 args: `(depressed_mods,
  latched_mods, locked_mods, depressed_layout, latched_layout,
  locked_layout)`. Для смены группы — последний параметр.
- Документация: https://docs.rs/xkbcommon/0.8/xkbcommon/xkb/struct.State.html
- Signal subscription через zbus 5 — пример в существующем коде:
  `proxy.receive_<name>().await` возвращает stream.
- Тип сигнала `layoutChanged(uint index)` — в zbus 5 derive макрос
  `#[zbus(signal, name = "layoutChanged")]` сгенерит `receive_layout_changed`.
