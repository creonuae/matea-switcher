# Milestone 5c — self-echo suppression + урок про user-input race

> Дата: 2026-05-10. Частичный milestone: один loop закрыт, второй (race с
> user input во время rewrite) **открыт** — требует EVIOCGRAB или другой
> блокировки. Зафиксирован как ключевой блокер для daily-use.

## Проблема 1 — self-echo loop (РЕШЕНО)

После M5 в live smoke увидели: одно слово даёт два FLIP подряд (например
`buhe→игру` дублируется). Корень — **наши uinput-emit'ы возвращаются
обратно через keyd virtual keyboard**:

```
matea uinput emit "rrrrr"
   ↓ kernel /dev/uinput
keyd видит KEY_R через свой grab
   ↓ keyd virtual keyboard /dev/input/event15
matea evdev reader видит KEY_R
   ↓ обработан как user input
WordBuffer наполняется → новое слово → новый FLIP → петля
```

`discover_keyboards` фильтрует устройства с "matea" в имени, но keyd virtual
keyboard в имени matea **не содержит** — фильтр не помогает.

### Решение

`Rewriter` теперь хранит `pending_echo: Option<(Instant, count)>`:
- На каждом emit (`backspace(n)`, `replay_keycodes(&[u16])`) вызываем
  `arm_self_echo(n)` — суммируем counter в `pending_echo`.
- В `handle_event` ПЕРЕД обработкой press-event reader спрашивает
  `maybe_consume_self_echo()`:
  - Если counter > 0 в окне 500мс → событие игнорируется, counter--.
  - Если counter == 0 или окно истекло → обычный flow.

500мс — safety cap; если keyd упал и эхо не приходит, мы не залипаем в
"ignore everything".

Подтверждено в логе:
```
INFO ... FLIP: переписываю word=jcnfyjdb keycodes=[36, 46, 49, 33, 21, 36, 32, 48]
DEBUG ... ignored self-echo keycode=KEY_BACKSPACE  (×8)
DEBUG ... ignored self-echo keycode=KEY_J
DEBUG ... ignored self-echo keycode=KEY_C
... (×8)
```

Self-loop закрыт ✓.

## Проблема 2 — race с user input во время rewrite (ОТКРЫТО)

Live smoke 2026-05-10 после M5c фикса: классификатор прекрасно работает
(`rjhtym→корень`, `dct→все`, `'nj→это`, `jcnfyjdb→останови` все правильно
помечены FLIP). Но при быстрой печати в окне получается мешанина — символы
юзера втыкаются между нашими backspace'ами и replay'ями.

Пример из лога: `mctuj` (видимо `m` от юзера + `cтуj` от нашего partial replay
`всего`).

### Корень

`do_flip` это последовательность ~16 операций (для слова из 8 букв):
1. backspace × 8
2. KWin DBus setLayout
3. sleep 50мс
4. replay × 8

Total ~80мс. За это время быстрый юзер (500 знм = 8 знв/сек = ~120мс
интервал) успевает напечатать 0-1 символ. На моменте `mctuj` юзер видимо
печатал _следующее_ слово сразу после space, и символы попали в окно во
время нашего rewrite.

### Что нужно (и НЕ сделано в M5c)

**EVIOCGRAB** на keyd virtual keyboard (`/dev/input/event15`) на время
rewrite. Когда мы grab'нули — kernel перестаёт отдавать events на эту
устройство **никому** (ни нам, ни приложениям) до release. Юзер физически
печатает в пустоту — события буферизуются в evdev и доходят после release.

Минус: **keyd сам уже grab'ит physical клавиатуру** через её own evdev
device. Наш EVIOCGRAB на keyd virtual — отдельная цепочка, в теории не
конфликтует, но не проверено эмпирически.

### Альтернативы (если EVIOCGRAB не сработает)

1. **Wait-for-pause** перед FLIP. На boundary не сразу делать rewrite, а
   подождать 200мс что юзер не печатает. Если за 200мс новый event — отменить
   rewrite (юзер уже что-то напечатал, мешать поздно). Минус: задержка
   восприятия — после space слово появится только через 200мс.

2. **Замораживание буфера** на время rewrite. Простой trick: пока
   `pending_echo > 0`, не пушить новые символы в WordBuffer. Это **не**
   останавливает попадание символов в gedit, но хотя бы не плодит фантомные
   FLIP'ы.

3. **Disable FLIP полностью**, оставить только classify+log + manual hotkey
   (Ctrl+Shift+L) для ручного флипа последнего слова. Юзер сам решает когда
   применять. Безопасно, но теряется главная фича «не думай о раскладке».

## Решение для следующего шага

Реализовать **EVIOCGRAB на event15** во время `do_flip`. Если конфликт с
keyd — попробовать grab/release в очень узком окне (~80мс, само время
rewrite). Если всё равно ломается — вернуться к (2) wait-for-pause.

Это **M5d** — критический blocker для daily-use. До M5d не запускать matea
для реальной работы; smoke-тесты только в gedit с медленной печатью.

## Что в коде после M5c

Файлы:
- `src/platform/uinput.rs` — `pending_echo` field + `arm_self_echo` /
  `maybe_consume_self_echo` API.
- `src/platform/linux.rs::handle_event` — guard в начале на pressed events.

## Извлечённые уроки

- **Любой rewriter на Linux БЕЗ EVIOCGRAB** будет иметь race с быстрой
  печатью. Это не наш баг, это inherent ограничение keyboard-injection
  подхода. Punto на macOS использует CGEventTap.consume; matea на Linux
  должна использовать EVIOCGRAB.
- **keyd как middleware** вообще усложняет: наш virtual keyboard виден
  keyd'у, наши emit'ы возвращаются через его virtual. Альтернатива на
  будущее — детектить keyd и делать matea как **plugin** в keyd config'е
  (он поддерживает Lua-скрипты). Это **не сейчас**, но хорошая мысль.
- **Нельзя запускать matea с активной Claude session в Konsole** — даже с
  Ctrl+Shift+M toggle, race может попасть на твой следующий ответ. Pre-test
  всегда: matea OFF → переключиться в gedit → matea ON.

## Что НЕ блокирует другую работу

Основной классификатор + dictionary lookups + context bias + tracking layout
работают **прекрасно**. Логирование verdict'ов корректное. Можно идти в M6
(AT-SPI window blacklist) и M5d (EVIOCGRAB) параллельно.
