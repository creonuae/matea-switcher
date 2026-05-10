# Milestone 5d — EVIOCGRAB на время rewrite

> Дата: 2026-05-10. Закрывает критический blocker M5c (race с user input).

## Проблема (M5c напоминалка)

`do_flip` это последовательность 16+ операций (для слова из 8 букв):
backspace × 8 → setLayout → sleep 50мс → replay × 8. Total ~80мс. За это
время юзер на 500знм успевает напечатать 1-2 символа. Эти символы попадают
в окно **между** нашими шагами → frankenstein-слова типа `mctuj` (юзер `m` +
наш partial replay `всего`). Self-echo фикс M5c эту race не закрыл.

## Решение

`EVIOCGRAB` — kernel-level ioctl на keyboard fd. После grab:
- Все события на это устройство буферизуются в evdev kernel buffer.
- НИ ОДИН клиент (включая reader, compositor, gedit, etc.) не получает
  events до ungrab.
- После ungrab — буфер сливается клиентам в порядке поступления.

Для нашего случая это означает: пока matea делает rewrite, юзер
**физически не может ничего напечатать в активное окно**. Его keypress'ы
ждут в буфере и применяются после rewrite.

Latency: с `EVIOCGRAB` пользователь чувствует ~80мс «paus» при flip. Если
он печатал быстро — несколько символов «застрянут» в буфере и появятся
после flip. Это **намного** лучше чем мешанина без grab.

## Реализация

### Архитектура fd

В Rewriter::new(grab_paths: Vec<PathBuf>) теперь:
1. Создаёт virtual keyboard (как раньше для emit).
2. Открывает **отдельные** fd на каждую клавиатуру в `grab_paths`. Это
   ОТДЕЛЬНЫЕ fd от тех что reader держит — kernel позволяет multiple open
   handles на один character device.
3. Хранит `grab_devices: Vec<Device>` для последующих grab/ungrab.

### Grab/ungrab API

- `Rewriter::grab_all() -> usize` — пытается grab каждый device, возвращает
  количество успешных. Если grab упал (PermissionDenied, EBUSY от другого
  grab-holder'а) — лог debug + продолжаем со следующими.
- `Rewriter::ungrab_all()` — best-effort ungrab всех (errors игнорим, на
  shutdown не блокируем).

### do_flip flow (M5d)

```rust
let grabbed = rewriter.grab_all();        // ← новое
if grabbed == 0 {
    warn!("grab_all не сработал — race окно открыто");
}
rewriter.backspace(t.keycodes.len())?;
kwin.set(target_index).await?;
sleep(50ms).await;
rewriter.replay_keycodes(&t.keycodes)?;
rewriter.ungrab_all();                    // ← новое
```

### Graceful degradation

Если grab падает (например keyd конфликтует), do_flip всё равно делает
rewrite — просто без атомарности. Юзер увидит поведение M5b/M5c. Лог
warning подскажет что нужно дебажить.

## WHY отдельные fd для grab

Reader-task держит `Device` который превращён в `EventStream` через
`into_event_stream()` — это owned move. Дать reader'у grab'ить было бы
сложно (нужен `&mut Device`, а stream съел owner). Отдельный fd в Rewriter
делает архитектуру чище: reader и rewriter разделены, grab не мешает чтению
кода reader'а. Однако kernel-level grab блокирует events для **всех** fd
(включая reader'а), что и есть нужное поведение.

## Конфликт с keyd (открытый вопрос)

На системе пользователя стоит keyd, который сам grab'ит physical клавы и
создаёт keyd virtual keyboard (`/dev/input/event15`). Наш grab пытается
взять второй grab на event15. Гипотеза:
- Если kernel позволяет nested grabs (одно устройство — несколько grab'ов
  одновременно с разных fd) — всё работает.
- Если нет — наш grab вернёт `EBUSY` (которое мы log'ируем как debug),
  и race остаётся. Это поведение проверяется live smoke.

В худшем случае — grab падает, поведение деградирует до M5c (self-echo
фикс работает, но не атомарность). Достаточно безопасно для следующего
теста, не катастрофа.

## Что осталось

- [ ] **Live smoke** проверить что grab реально блокирует юзерский ввод во
      время rewrite на этой системе с keyd.
- [ ] Если конфликт с keyd — попробовать **wait-for-pause** approach (не
      делать flip если за 200мс был user input).
- [ ] Эмпирически понять минимальное window grab — может 30-40мс достаточно
      без блюра восприятия.
- [ ] Telemetry: считать `grabbed` count в метриках для diagnostic.

## Тесты

Unit-тестов на grab/ungrab нет — kernel-зависимая операция. Покрытие
только manual smoke. Total cargo test: 27/27 ok (без новых).
