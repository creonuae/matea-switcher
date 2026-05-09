//! Word-level classifier: keep | flip | uncertain.
//!
//! Архитектура:
//! - **Fast-path** (n-gram + Hunspell словарь):
//!     - проверяем что текущее слово валидно в активной раскладке
//!     - если **не** валидно, но валидно после mapper-flip — → Flip
//!     - если валидно в обеих после flip — → Uncertain (отдадим в LLM)
//!     - если валидно в текущей — → Keep
//! - **Slow-path** (Qwen-0.5B GGUF, опционально с v0.2):
//!     - prompt: short context из последних N слов + кандидат
//!     - GBNF grammar ограничивает output на `keep|flip`
//!     - latency бюджет ~50мс
//!
//! Большинство слов закрываются fast-path. LLM зовём только когда оба варианта
//! орфографически валидны (амбигуальные 1-3 char токены, имена собственные, etc.).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Слово на правильной раскладке. Не трогать.
    Keep,
    /// Слово на неправильной раскладке. Переписать в другой mapping
    /// и переключить активную раскладку.
    Flip,
    /// Не уверен по эвристике — нужен LLM (или оставить пользователю как Keep
    /// если LLM выключен).
    Uncertain,
}

/// Контекст для классификации.
#[derive(Debug, Clone)]
pub struct ClassifyInput<'a> {
    /// Текущее «слово» (что юзер только что напечатал до space/enter/punct).
    pub word: &'a str,
    /// Активная раскладка по мнению ОС: "us" / "ru".
    pub active_layout: &'a str,
    /// Предыдущие N слов в этом окне (контекст для LLM).
    pub recent_words: &'a [String],
    /// WM_CLASS активного окна (для blacklist'ов и LLM-контекста).
    pub window_class: Option<&'a str>,
}

pub fn classify(_input: &ClassifyInput<'_>) -> Verdict {
    // TODO v0.1:
    //   - hunspell::Dict::check(word) для en_US и ru_RU
    //   - mapper::flip(word) и проверить .check() ещё раз
    //   - bigram lookup в precomputed таблицах (для быстрого detect "точно бред в этой раскладке")
    Verdict::Keep
}
