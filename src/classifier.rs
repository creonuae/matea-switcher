//! Word-level classifier: keep | flip | uncertain.
//!
//! Архитектура:
//! - **Fast-path** (Hunspell словари RU + EN):
//!     - проверяем валидно ли слово в активной раскладке
//!     - если **не** валидно, mapping в другой layout, проверяем там
//!     - если валидно после flip — → Flip
//!     - если валидно в обеих — → Uncertain (отдадим в LLM v0.2+)
//!     - если валидно только в текущей — → Keep
//!     - невалидно везде — → Uncertain (опечатка или имя; v0.2 отдаст LLM)
//! - **Slow-path** (Qwen-0.5B GGUF, v0.2+):
//!     - prompt с recent_words + кандидатом
//!     - GBNF grammar `keep|flip`
//!     - latency ≤50мс
//!
//! Большинство слов закрываются fast-path. LLM зовём только для амбигуальных.

use anyhow::{Context, Result};
use hunspell_rs::Hunspell;
use serde::{Deserialize, Serialize};

use crate::mapper;

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

/// Wraps two Hunspell-словаря (en_US и ru_RU), даёт fast-path classify.
pub struct DictClassifier {
    en: Hunspell,
    ru: Hunspell,
}

impl DictClassifier {
    /// Дефолтная инициализация: смотрим в `~/.local/share/matea/dicts/`. Если там
    /// нет UTF-8 версий — авто-конвертим из системных `/usr/share/hunspell/*` через
    /// `iconv` (на Fedora ru_RU словарь поставляется в KOI8-R, а hunspell-rs
    /// принимает UTF-8 — поэтому раз-конвертация при первом запуске).
    pub fn new_default() -> Result<Self> {
        let cache_dir = ensure_utf8_dicts()?;
        Self::new(
            &format!("{cache_dir}/en_US.aff"),
            &format!("{cache_dir}/en_US.dic"),
            &format!("{cache_dir}/ru_RU.aff"),
            &format!("{cache_dir}/ru_RU.dic"),
        )
    }

    pub fn new(en_aff: &str, en_dic: &str, ru_aff: &str, ru_dic: &str) -> Result<Self> {
        // Hunspell::new в hunspell-rs не возвращает Result; ошибки чтения файлов
        // он молча проглатывает, поэтому проверим существование вручную.
        for path in [en_aff, en_dic, ru_aff, ru_dic] {
            if !std::path::Path::new(path).exists() {
                anyhow::bail!("hunspell dict not found: {}", path);
            }
        }
        let en = Hunspell::new(en_aff, en_dic);
        let ru = Hunspell::new(ru_aff, ru_dic);
        Ok(Self { en, ru })
    }

    /// Проверка валидности в указанном языке.
    pub fn is_valid(&self, word: &str, lang: Lang) -> bool {
        if word.is_empty() {
            return false;
        }
        match lang {
            Lang::En => matches!(self.en.check(word), hunspell_rs::CheckResult::FoundInDictionary),
            Lang::Ru => matches!(self.ru.check(word), hunspell_rs::CheckResult::FoundInDictionary),
        }
    }

    /// Главный classify: смотрит на active_layout, проверяет в нём, проверяет flip,
    /// возвращает Verdict.
    ///
    /// Слова длиной 1 char всегда Uncertain (амбигуальны: "a"/"я"/"и" — могут быть
    /// валидны в обеих в зависимости от форм).
    pub fn classify(&self, input: &ClassifyInput<'_>) -> Verdict {
        if input.word.chars().count() <= 1 {
            return Verdict::Uncertain;
        }

        let (current_lang, other_lang, flipped) = match input.active_layout {
            "us" => (Lang::En, Lang::Ru, mapper::en_to_ru(input.word)),
            "ru" => (Lang::Ru, Lang::En, mapper::ru_to_en(input.word)),
            _ => return Verdict::Uncertain,
        };

        let valid_in_current = self.is_valid(input.word, current_lang);
        let valid_after_flip = self.is_valid(&flipped, other_lang);

        match (valid_in_current, valid_after_flip) {
            (true, false) => Verdict::Keep,
            (false, true) => Verdict::Flip,
            (true, true) => Verdict::Uncertain, // обе валидны (короткие/имена) — пусть LLM
            (false, false) => Verdict::Uncertain, // опечатка — оставим пользователю до LLM
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

/// Подготовить UTF-8 словари в `~/.local/share/matea/dicts/`. На первом запуске
/// конвертим ru_RU из системного KOI8-R; en копируем как есть. Возвращает путь
/// к директории кеша.
fn ensure_utf8_dicts() -> Result<String> {
    let dirs = directories::BaseDirs::new().context("BaseDirs::new")?;
    let cache = dirs.data_local_dir().join("matea").join("dicts");
    std::fs::create_dir_all(&cache).context("create cache dir")?;

    let en_aff = cache.join("en_US.aff");
    let en_dic = cache.join("en_US.dic");
    let ru_aff = cache.join("ru_RU.aff");
    let ru_dic = cache.join("ru_RU.dic");

    if !en_aff.exists() {
        std::fs::copy("/usr/share/hunspell/en_US.aff", &en_aff)
            .context("copy en_US.aff")?;
    }
    if !en_dic.exists() {
        std::fs::copy("/usr/share/hunspell/en_US.dic", &en_dic)
            .context("copy en_US.dic")?;
    }

    if !ru_aff.exists() {
        // SET KOI8-R → SET UTF-8 + iconv body
        let raw = std::fs::read("/usr/share/hunspell/ru_RU.aff")
            .context("read ru_RU.aff")?;
        let (cow, _, had_errors) = encoding_rs::KOI8_R.decode(&raw);
        if had_errors {
            anyhow::bail!("KOI8-R decode of ru_RU.aff produced errors");
        }
        let utf8 = cow.replace("SET KOI8-R", "SET UTF-8");
        std::fs::write(&ru_aff, utf8.as_bytes()).context("write ru_RU.aff")?;
    }
    if !ru_dic.exists() {
        let raw = std::fs::read("/usr/share/hunspell/ru_RU.dic")
            .context("read ru_RU.dic")?;
        let (cow, _, had_errors) = encoding_rs::KOI8_R.decode(&raw);
        if had_errors {
            anyhow::bail!("KOI8-R decode of ru_RU.dic produced errors");
        }
        std::fs::write(&ru_dic, cow.as_bytes()).context("write ru_RU.dic")?;
    }

    Ok(cache.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> DictClassifier {
        DictClassifier::new_default().expect("load dicts")
    }

    #[test]
    fn classify_valid_english_in_us() {
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "hello",
            active_layout: "us",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Keep);
    }

    #[test]
    fn classify_russian_typed_on_us_should_flip() {
        // "ghbdtn" набранное на us = "привет" в ru.
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "ghbdtn",
            active_layout: "us",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Flip);
    }

    #[test]
    fn classify_english_typed_on_ru_should_flip() {
        // "руддщ" набранное на ru = "hello" в en.
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "руддщ",
            active_layout: "ru",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Flip);
    }

    #[test]
    fn classify_valid_russian_in_ru() {
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "привет",
            active_layout: "ru",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Keep);
    }

    #[test]
    fn classify_short_word_uncertain() {
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "a",
            active_layout: "us",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Uncertain);
    }

    #[test]
    fn classify_garbage_uncertain() {
        // bogus invalid в обеих раскладках → Uncertain (не Flip)
        let d = dict();
        let v = d.classify(&ClassifyInput {
            word: "xqzkpw",
            active_layout: "us",
            recent_words: &[],
            window_class: None,
        });
        assert_eq!(v, Verdict::Uncertain);
    }
}
