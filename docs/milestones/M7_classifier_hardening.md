# Milestone 7 — classifier hardening

> Дата: 2026-05-10. Сделан вне порядка (раньше M6) — потому что pure-функция,
> безопасно тестировать без запуска matea, снижает blast-radius ошибок при
> следующем live-тесте M5.

## Цель

Убрать FALSE FLIP и FALSE UNCERTAIN на токенах которые **точно не нужно**
переписывать, но которые Hunspell честно не находит ни в каком словаре:

- Номера телефонов, версии, числовые ID (`80663422514`, `2026`, `42`)
- Идентификаторы в коде (`i7`, `2nd`, `KEY_A`, `H264`)
- URL/email/paths (`example.com`, `user@host`, `/usr/bin`)
- Mixed scripts (`Telegram-чат`, `macбук`)
- Имена собственные с большой буквы (`Anthropic`, `Москва`)

## WHY

Из smoke-теста M5 видели: пользователь ввёл `80663422514` → matea сказала
`UNCERTAIN`. Это не критично (UNCERTAIN не делает rewrite), но засоряет лог
и заставляет нас потом гонять LLM на такие очевидные кейсы. Лучше отсечь
правилами на старте.

Также — **safety**: пока нет AT-SPI password-detection (M6), правила
M7 случайно отлавливают часть «неправильных» токенов как Keep. Например, если
пользователь паролит вводит токен типа `abc123XYZ` — он содержит цифры, и
matea точно не попытается его флипить.

## WHAT

В `DictClassifier::classify()` ДО вызовов Hunspell:

| # | Правило | Verdict | Пример |
|---|---|---|---|
| 1 | `len ≤ 1` | Uncertain | `a`, `я` |
| 2 | Только цифры | **Keep** | `42`, `80663422514` |
| 3 | Цифра + буква | **Keep** | `i7`, `2nd`, `H264` |
| 4 | Содержит `@`, `.`, `:`, `/`, `\` | **Keep** | `user@host.com`, `/usr/bin`, `C:\Windows` |
| 5 | Mixed Latin + Cyrillic | **Keep** | `Telegram-чат`, `macбук` |
| 6 | Capitalized + не в словарях | **Keep** | `Anthropic`, `Москва` |

После — стандартный flow Hunspell `valid_in_current` × `valid_after_flip`.

### Покрытие тестами

5 новых тестов в `src/classifier.rs::tests`:
- `rule_pure_digits_keep`
- `rule_alphanumeric_keep`
- `rule_url_path_keep`
- `rule_mixed_scripts_keep`
- `rule_capitalized_unknown_keep`

Total `cargo test`: 20/20 ok (4 mapper + 1 xkb + 4 context + 11 classifier).

## HOW проверить вручную

После M5b (когда DBus fix актив, см `M5_uinput_rewriter.md`) — повторить
smoke-тест и убедиться:
- `80663422514 ` → лог `verdict=KEEP`, нет rewrite. ✓
- `i7 ` → KEEP.
- `Anthropic ` → KEEP (не было бы M7 правила — было бы UNCERTAIN).
- `Telegram-чат ` → KEEP.
- `ghbdtn ` → FLIP (как и раньше — обычные слова не задеты новыми правилами).

## Что НЕ закрывает (открытые улучшения для M7b/M11)

- **`recent_words` контекстный bias** — если последние 3 слова RU и текущий
  кандидат валиден в обеих → bias к RU. Сейчас контекст игнорируется. Это
  proto-LLM логика без модели, можно сделать без зависимостей.
- **Whitelist приложений** — в Telegram-чате с RU-другом всегда Keep. Это
  M6 территория (нужен AT-SPI window-class detection).
- **Обучение per-user** — если юзер 5 раз отменил наш FLIP на каком-то слове
  через manual `Ctrl+Z` — больше не флипить. Нужна persistence + history,
  отдельная фича.
- **Числа с разделителями** (`+7-916-123-45-67`, `1.234,56`) — содержат
  пунктуацию-boundary, значит word-boundary detection их разрезает. Не
  доходит до classifier как одно слово. Это M3-улучшение (раньше M7).

## Связанные файлы

- Код: `src/classifier.rs` функция `classify()` + 5 unit-тестов.
- Roadmap: `docs/NEXT_STEPS.md → Milestone 7` (теперь done).
- Architecture: тут добавлять не надо — модуль остался pure-функция, та же
  сигнатура `Verdict classify(&ClassifyInput)`.
