# Qwen-on-Cerebras task pack — round 1 (2026-05-10)

> Workflow: Claude (архитектор + ревьюер) пишет detailed task specs здесь.
> Юзер копирует в Cerebras prompt с Qwen 235B — модель генерит код. Claude
> делает review против spec'а и main branch'а. Итерация.

## Контекст

После live smoke M6 (commit `52c0a39`) выявили 2 баг'а и 1 ограничение:

1. **Дубли слов после FLIP** (`туттут`, `давайдавай` — пользователь видит).
   Корень: xkb-state matea desyncs от system layout после FLIP. matea
   продолжает думать что us, юзер на system ru, classifier видит
   us-keycodes как us-glyphs, делает второй FLIP того же слова.

2. **`руддщ` на ru-раскладке не флипнулось в `hello`**. Тот же desync.

3. **AT-SPI guard не блокирует Konsole** — Konsole не emit focus events
   (известное ограничение, см. `M6_atspi.md`). Нужен KWin DBus fallback.

## Задачи

| ID | Что | Файл | Размер | Зависимости |
|---|---|---|---|---|
| **T1** | Sync xkb-state matea с system layout (закрывает #1, #2) | `T1_xkb_layout_sync.md` | ~80 LOC | Без |
| **T2** | KWin DBus active window fallback (закрывает #3) | `T2_kwin_active_window.md` | ~120 LOC | Без |
| **T3** | Manual flip-last-word hotkey (Ctrl+Shift+L) | `T3_manual_flip_hotkey.md` | ~60 LOC | Без |

Все три задачи **независимы** — Qwen может генерить параллельно.

## Универсальные правила для Qwen (везде)

1. **Без `Co-Authored-By: AI`** в коммите. Без `🤖 Generated with...`.
   Прямой запрос владельца repo.
2. **Комментарии в коде только для WHY** (неочевидная причина, скрытое
   ограничение, обход бага). НЕ объяснять WHAT — хорошие имена и так
   читаются.
3. **`#[cfg(target_os = "linux")]`** на всё linux-specific (новые модули
   уже под этим guard'ом в `src/platform/mod.rs`).
4. **Стек fixed:**
   - Rust 2024 edition, MSRV 1.93.
   - tokio current_thread runtime.
   - `#[async_trait(?Send)]` на trait Platform — non-Send state OK.
   - zbus 5 для D-Bus.
   - `tracing` для логов (info!/debug!/warn!).
   - `anyhow::Result` для error propagation, `thiserror` если нужны
     типизированные errors.
5. **`tokio::task::spawn`** (не spawn_local) — current_thread runtime,
   но Send-bound на future нужен компилятору. Если spawn'имый future
   non-Send — переделать архитектуру, не ослаблять.
6. **Тесты на pure-функциях обязательны.** Kernel/D-Bus интеграции —
   manual smoke (но отметить test cases в spec).
7. **Не менять unrelated файлы.** Если что-то требует изменения в
   уже-существующем — пометить в PR description явно.

## Как Claude делает review

1. `git diff main...feature-branch` — посмотреть изменения.
2. Проверить:
   - Соответствует ли task spec'у (контракт, поведение, edge cases)?
   - Тесты покрывают сценарии из spec'а?
   - Нет ли AI-trailers в коммитах?
   - Нет ли WHAT-комментариев?
   - Compile + cargo test проходят?
3. Если есть проблемы — формулирует patch instructions для Qwen
   следующей итерации.
4. Если всё ок — squash-merge в main.

## Workflow для юзера

```bash
# 1. Создать ветку для T1:
cd ~/matea-switcher
git checkout -b qwen-T1-xkb-sync

# 2. Скопировать docs/qwen-tasks/T1_xkb_layout_sync.md в Cerebras prompt.
#    Дождаться генерации.

# 3. Применить патч:
#    Qwen возвращает либо diff, либо файлы целиком.
#    Если diff — git apply qwen-output.patch.
#    Если файлы — копировать в src/...

# 4. Build + test:
cargo build --release
cargo test

# 5. Commit:
git commit -am "feat(linux): T1 xkb-state sync via Qwen"
git push origin qwen-T1-xkb-sync

# 6. Передать Claude'у на review:
#    "Сделай review qwen-T1-xkb-sync, мердж если ок".
```
