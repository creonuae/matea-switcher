# Milestone 11 — context bias через recent_words

> Дата: 2026-05-10. Pure-classifier улучшение поверх M7.

## Проблема

Когда слово валидно в **обеих** раскладках (например `ye` валидно в en
как «yore» альтернатива; flipped `не` валидно в ru) — classifier раньше
возвращал `Uncertain`, и FLIP не делался. Юзер вынужден руками править.

В реальности контекст почти всегда даёт ответ: если предыдущие 4 слова
были на ru, то и ambiguous кандидат — скорее ru.

## WHAT

В `DictClassifier::classify()` теперь:

```rust
match (valid_in_current, valid_after_flip) {
    (true, false) => Verdict::Keep,
    (false, true) => Verdict::Flip,
    (true, true) => self.context_bias(input, current_lang, other_lang),  // ← M11
    (false, false) => Verdict::Uncertain,
}
```

`context_bias` смотрит на последние 5 слов из `input.recent_words`:
- Считает в скольких слово валидно в `current_lang` (счётчик A)
- В скольких в `other_lang` (счётчик B)
- Игнорирует digits / single-char (они не сигнал)
- Если `A ≥ B + 2` → bias к current → Verdict::Keep
- Если `B ≥ A + 2` → bias к other → Verdict::Flip
- Иначе (равенство, либо одно слово в каждом) → Uncertain (как раньше)

Margin = 2 — иначе на маленьких выборках (3-4 слова) bias срабатывал бы
случайно (1 слово ru против 0 — недостаточный сигнал).

## Интеграция в main loop

`src/platform/linux.rs::run()` теперь держит `WordHistory::new(10)` —
кольцевой буфер последних 10 слов. На каждом WORD-event:
- На FLIP (если matea enabled) → в history идёт **flipped** строка
  (юзер увидит её, контекст должен её отражать).
- На FLIP suppressed (matea disabled) → оригинал.
- На Keep/Uncertain → оригинал.

`recent_words` передаётся в `ClassifyInput`, доходит до `context_bias`.

## Тесты

3 новых assert в `src/classifier.rs::tests`:
- `context_bias_ru_dominant_keeps_ru_ambiguous` — sanity check, ru-контекст
  не приводит к ru-словам Flip.
- `context_bias_no_recent_words_uncertain_for_ambiguous` — без recent_words
  ambiguous case остаётся Uncertain (старое поведение).
- `context_bias_ru_recent_flips_us_typed_word` — главное: `ye` напечатано
  на us, контекст 4 ru слова → bias к ru → Verdict::Flip.

Total cargo test: **25/25 ok**.

## Что НЕ закрыто (M11b или v0.2)

- **Per-window history**. Сейчас глобальный буфер на весь процесс. В Telegram
  бы хотелось отдельную историю на чат. Это требует window-class detection
  (M6 AT-SPI) + per-app/chat буфер.
- **LLM bias** для случаев где margin недостаточный. v0.2 (Qwen-0.5B GGUF
  с короткой prompt включающей recent_words).
- **Confidence в Verdict**. Сейчас Verdict — enum без confidence. Если
  classifier сомневается (бias 1 vs 1), полезно вернуть Verdict::Flip { confidence: 0.55 }
  и в дальнейшем требовать threshold для action. Подумать в M11b.
