# matea-switcher и keyd — настройка

Если у тебя установлен [`keyd`](https://github.com/rvaiya/keyd) (популярный
Linux-remapper клавиатуры), сразу после установки matea-switcher сделай
**одно изменение** в keyd-конфиге, иначе будут дубли символов в окне после
FLIP.

## Что и зачем

keyd по дефолту подписывается на ВСЕ input devices в системе через udev.
Когда matea-switcher делает FLIP-rewrite через `uinput`, наши events идут
двумя путями к compositor'у:

1. Напрямую с нашего virtual keyboard → KWin → активное окно.
2. **Параллельно** keyd видит наш virtual → пропускает через свою
   processing pipeline → эмитит дубль через `keyd virtual keyboard`
   → KWin → окно ещё раз.

Юзер видит каждое FLIP'нутое слово дважды.

## Решение в одну строку

В `~/.config/keyd/default.conf` добавь секцию `[ids]` с одним идентификатором:

```ini
[ids]
-6d61:7465
```

(Минус впереди = «игнорируй это устройство».)

Затем:

```bash
sudo systemctl reload keyd
```

Без перезапуска matea-switcher — изменение применится сразу.

## Что значат `6d61:7465`

matea-switcher создаёт virtual keyboard с нестандартным `vendor:product`
ID — `0x6d61:0x7465`. Это ASCII-коды букв «ma» и «te» (см.
`src/platform/uinput.rs::MATEA_VENDOR_ID`). Любая программа которая
фильтрует input devices по этому ID может его узнать.

## Проверка что сработало

Запусти matea-switcher с `MATEA_LOG=info` — на старте должно быть:

```
INFO matea-switcher v0.1.0 starting
INFO ... обнаружено клавиатур: N
INFO ... uinput virtual keyboard создан: matea-switcher virtual keyboard
WARN keyd detected — без настройки будет echo-loop ...   ← это warning
```

После того как keyd config обновлён и reload сделан — warning
**останется** в логе (мы не проверяем actual keyd config), но **дубли
пропадут**. Smoke-test:

1. Открой gedit (или kate / Notepadqq).
2. Набери `lfdfq ` (us-keycodes на ru-задумке) — ожидаешь `давай ` ровно
   **один раз**.
3. Если видишь `давайдавай` — keyd-config не применён.

## Если у тебя keyd НЕТ

Если у тебя **другой** remapper (xremap, kanata, evremap, kbdd) — он
**может** иметь ту же проблему. Проверь его документацию на тему
device-ignore-list. Идентификатор тот же: `6d61:7465`.

## Если у тебя keyd ЕСТЬ, но ты не хочешь его трогать

Есть **второй путь** — заставить matea-switcher эмитить через
Wayland-protocol `zwp_virtual_keyboard_v1` напрямую в compositor, минуя
uinput. Тогда keyd не увидит наши events вообще, потому что они идут
**не через evdev/input subsystem**.

Это запланировано как **v0.2 / Variant B** в `docs/qwen-tasks/T4_keyd_echo_loop.md`.
Сейчас (v0.1) — путь через keyd config.
