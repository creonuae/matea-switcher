# Milestone 9 — config file (минимальный)

> Дата: 2026-05-10. Базовая часть. Полный config с blacklists/whitelists
> ждёт M6 (window class detection даёт смысл blacklist'у).

## Цель

Дать юзеру настраивать поведение matea без перекомпиляции и без CLI-флагов:
правка `~/.config/matea/config.toml`, перезапуск daemon.

## Структура

```toml
[general]
enabled = true              # глобальный switch (false = matea читает но не флипает)

[layouts]
pair = ["us", "ru"]         # пара для flip-mapping

[hotkeys]
toggle = "Ctrl+Shift+M"     # toggle ON/OFF (M9b: реальный парсинг)
```

При первом запуске matea пишет дефолтный config в стандартное место
(XDG `$XDG_CONFIG_HOME/matea/config.toml`, обычно `~/.config/matea/`).
В файле — комментарий «удали, будет пересоздан» — упрощает onboarding.

## WHY такие defaults

- `enabled = true` — иначе после установки matea выглядит «не работающей».
- `pair = ["us", "ru"]` — целевая аудитория проекта.
- `toggle = "Ctrl+Shift+M"` — то же что hardcode'ом сейчас (M8).

## Реализация (`src/config.rs`)

- `serde::{Deserialize, Serialize}` derive на всех structs.
- `directories::BaseDirs` для определения config path кроссплатформенно.
- На старте `load()`:
  - Если файла нет → пишем дефолт + возвращаем дефолт.
  - Если есть и парсится → возвращаем загруженный.
  - Если есть но битый TOML → log warning + дефолт. **НЕ падаем** —
    daemon должен жить даже при кривом config.
- Тесты: `defaults_round_trip` и `parse_partial_uses_defaults`.

## Что делает / не делает прямо сейчас

**Делает:**
- Создаёт config-файл при первом запуске.
- Парсит существующий.
- Логирует загруженную конфигурацию на старте (юзер видит что подхватилось).

**НЕ делает (M9b):**
- Реальное применение `general.enabled` — пока default `true` в run(), не
  читается из config (нужно прокинуть Config через Platform::run; сейчас
  config просто загружается и логируется).
- Парсинг `hotkeys.toggle` строки в `(modifier_mask, keycode)`. Сейчас
  hardcoded Ctrl+Shift+M.
- `layouts.pair` тоже не используется в do_flip — там пары всё ещё
  hardcoded `us↔ru`.

Это **намеренно** маленький first-step — ставим инфраструктуру и тесты,
дальше каждое поле подключается отдельной мини-итерацией.

## Hot reload (M9c)

Когда config станет load-bearing, добавить `notify` crate для file-watch,
перезагружать на изменение. Но пока — рестарт daemon.

## Тесты

2 новых assert в `src/config.rs::tests`. Total cargo test: **27/27 ok**.
