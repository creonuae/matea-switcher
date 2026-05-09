//! WordBuffer и Context — что собираем по ходу набора, для классификатора и LLM.

use std::collections::VecDeque;

/// Накапливает текущее «слово» из приходящих keysym/char событий. Завершает на
/// word boundary (space, enter, punctuation), отдаёт слово целиком наверх.
#[derive(Debug, Default)]
pub struct WordBuffer {
    chars: String,
}

impl WordBuffer {
    pub fn push(&mut self, c: char) {
        self.chars.push(c);
    }

    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.chars)
    }

    pub fn len(&self) -> usize {
        self.chars.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }
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
        Self { cap, words: VecDeque::with_capacity(cap) }
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
