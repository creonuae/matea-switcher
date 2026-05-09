//! qwerty ↔ йцукен mapping.
//!
//! Таблица символов одной физической клавиши в двух раскладках. На вход —
//! строка, на выход — флипнутая. Не зависит от регистра в источнике, но
//! сохраняет регистр на выходе.
//!
//! Замечание: маппинг **не симметричен** в обе стороны (у некоторых клавиш в RU
//! есть symbols которых нет в EN). Поэтому делаем две таблицы, не обратимая пара.

const QWERTY_TO_JCUKEN: &[(char, char)] = &[
    ('q', 'й'), ('w', 'ц'), ('e', 'у'), ('r', 'к'), ('t', 'е'), ('y', 'н'),
    ('u', 'г'), ('i', 'ш'), ('o', 'щ'), ('p', 'з'), ('[', 'х'), (']', 'ъ'),
    ('a', 'ф'), ('s', 'ы'), ('d', 'в'), ('f', 'а'), ('g', 'п'), ('h', 'р'),
    ('j', 'о'), ('k', 'л'), ('l', 'д'), (';', 'ж'), ('\'', 'э'),
    ('z', 'я'), ('x', 'ч'), ('c', 'с'), ('v', 'м'), ('b', 'и'), ('n', 'т'),
    ('m', 'ь'), (',', 'б'), ('.', 'ю'), ('/', '.'),
    ('`', 'ё'),
];

/// EN → RU.
pub fn en_to_ru(s: &str) -> String {
    flip(s, QWERTY_TO_JCUKEN, /* reverse */ false)
}

/// RU → EN.
pub fn ru_to_en(s: &str) -> String {
    flip(s, QWERTY_TO_JCUKEN, /* reverse */ true)
}

fn flip(s: &str, table: &[(char, char)], reverse: bool) -> String {
    s.chars()
        .map(|c| {
            let lower = c.to_lowercase().next().unwrap_or(c);
            let was_upper = c != lower;
            let mapped = table
                .iter()
                .find_map(|&(a, b)| {
                    let (from, to) = if reverse { (b, a) } else { (a, b) };
                    if lower == from { Some(to) } else { None }
                })
                .unwrap_or(lower);
            if was_upper {
                mapped.to_uppercase().next().unwrap_or(mapped)
            } else {
                mapped
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_en_to_ru_basic() {
        assert_eq!(en_to_ru("ghbdtn"), "привет");
        assert_eq!(en_to_ru("Ghbdtn"), "Привет");
    }

    #[test]
    fn test_ru_to_en_basic() {
        assert_eq!(ru_to_en("руддщ"), "hello");
        assert_eq!(ru_to_en("Руддщ"), "Hello");
    }

    #[test]
    fn test_passthrough_unknown() {
        // Цифры, пробелы и unicode-символы которых нет в таблице — без изменений
        assert_eq!(en_to_ru("123"), "123");
        assert_eq!(en_to_ru("hello world"), "руддщ цщкдв");
    }
}
