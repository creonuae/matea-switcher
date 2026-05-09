//! WordBuffer и Context — что собираем по ходу набора, для классификатора и LLM.

use std::collections::VecDeque;

/// Накапливает текущее «слово» из приходящих UTF-8 событий клавиатуры. Завершает
/// слово на word-boundary (space/enter/punct), отдаёт через [`WordBuffer::take`].
///
/// На v0.1 — поддерживает push (символ), pop (для backspace), take (на boundary).
/// Учитываем что русские буквы — multibyte UTF-8, поэтому работа идёт через `chars`
/// итератор + грamateя на границах символов, не байтов.
#[derive(Debug, Default)]
pub struct WordBuffer {
    chars: String,
    /// Раскладка, в которой пользователь начал печатать это слово (us/ru/...).
    /// Если в середине слова раскладка сменилась — это уже редкий edge case,
    /// фиксируем первую — её и используем при классификации.
    started_in_layout: Option<String>,
}

impl WordBuffer {
    pub fn push(&mut self, ch: char, layout: &str) {
        if self.chars.is_empty() {
            self.started_in_layout = Some(layout.to_string());
        }
        self.chars.push(ch);
    }

    /// Удалить последний символ (для KEY_BACKSPACE). Если буфер пуст — no-op.
    pub fn pop(&mut self) -> Option<char> {
        let c = self.chars.pop();
        if self.chars.is_empty() {
            self.started_in_layout = None;
        }
        c
    }

    pub fn take(&mut self) -> TakenWord {
        let chars = std::mem::take(&mut self.chars);
        let layout = self.started_in_layout.take().unwrap_or_default();
        TakenWord { word: chars, layout }
    }

    pub fn len(&self) -> usize {
        self.chars.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.chars
    }

    pub fn layout(&self) -> Option<&str> {
        self.started_in_layout.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct TakenWord {
    pub word: String,
    pub layout: String,
}

/// Разделители слов. Не считаем символом слова: пробелы, табы, enter,
/// большинство ASCII-пунктуации (но **не** apostrophe/dash — они часто внутри слов).
pub fn is_word_boundary_char(ch: char) -> bool {
    if ch.is_whitespace() {
        return true;
    }
    matches!(
        ch,
        '.' | ',' | ';' | ':' | '!' | '?' |
        '(' | ')' | '[' | ']' | '{' | '}' |
        '<' | '>' | '"' | '`' | '|' | '\\' |
        '/' | '@' | '#' | '$' | '%' | '^' |
        '&' | '*' | '+' | '=' | '~'
    )
}

/// История последних слов в активном окне — для контекста LLM (proactive prediction).
/// На смену окна — отдельный буфер. На уровне процесса — `HashMap<window_id, WordHistory>`.
#[derive(Debug)]
pub struct WordHistory {
    cap: usize,
    words: VecDeque<String>,
}

impl WordHistory {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            words: VecDeque::with_capacity(cap),
        }
    }

    pub fn push(&mut self, word: String) {
        if self.words.len() == self.cap {
            self.words.pop_front();
        }
        self.words.push_back(word);
    }

    pub fn recent(&self) -> impl Iterator<Item = &String> {
        self.words.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_basic_push_take() {
        let mut b = WordBuffer::default();
        b.push('h', "us");
        b.push('i', "us");
        assert_eq!(b.as_str(), "hi");
        assert_eq!(b.layout(), Some("us"));
        let t = b.take();
        assert_eq!(t.word, "hi");
        assert_eq!(t.layout, "us");
        assert!(b.is_empty());
    }

    #[test]
    fn buffer_unicode_pop() {
        let mut b = WordBuffer::default();
        b.push('п', "ru");
        b.push('р', "ru");
        b.push('и', "ru");
        assert_eq!(b.len(), 3);
        let popped = b.pop();
        assert_eq!(popped, Some('и'));
        assert_eq!(b.as_str(), "пр");
    }

    #[test]
    fn buffer_pop_until_empty_clears_layout() {
        let mut b = WordBuffer::default();
        b.push('a', "us");
        assert_eq!(b.layout(), Some("us"));
        b.pop();
        assert_eq!(b.layout(), None);
    }

    #[test]
    fn boundary_chars() {
        assert!(is_word_boundary_char(' '));
        assert!(is_word_boundary_char('\n'));
        assert!(is_word_boundary_char('.'));
        assert!(is_word_boundary_char(','));
        assert!(!is_word_boundary_char('a'));
        assert!(!is_word_boundary_char('п'));
        assert!(!is_word_boundary_char('-')); // дефис внутри слов
        assert!(!is_word_boundary_char('\'')); // apostrophe в don't
    }
}
