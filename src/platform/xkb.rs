//! Тонкая обёртка вокруг libxkbcommon: keycode → character с учётом активной раскладки.
//!
//! Создаём keymap с двумя layouts (us, ru) и опцией `grp:alt_space_toggle`, чтобы
//! xkb сама переключала группу когда пользователь жмёт Alt+Space. Это упрощает MVP —
//! не нужно сразу подписываться на KWin DBus signal'ы. Когда приделаем proactive
//! prediction (v0.3), подключим actual KWin layout state.
//!
//! # ВАЖНО про keycode offset
//!
//! evdev и xkb используют **разные** keycode-схемы:
//! - evdev: KEY_A = 30 (linux/input-event-codes.h)
//! - xkb:   AC01 = 38
//!
//! Разница ровно +8. На каждом translation evdev keycode → xkb keycode добавляем 8.

use anyhow::{Context, Result};
use xkbcommon::xkb;

pub struct XkbTranslator {
    state: xkb::State,
}

impl XkbTranslator {
    /// Создать translator с дефолтной двух-раскладочной раскладкой us,ru.
    pub fn new() -> Result<Self> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",                          // rules — дефолт "evdev"
            "",                          // model — дефолт "pc105"
            "us,ru",                     // layouts: основной + второй
            "",                          // variants
            Some("grp:alt_space_toggle".to_string()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .context("xkb_keymap_new_from_names: не удалось скомпилировать раскладку us,ru")?;
        let state = xkb::State::new(&keymap);
        Ok(Self { state })
    }

    /// Учесть нажатие/отпускание физической клавиши в state-машине xkb (для того,
    /// чтобы modifier-state и group switching работали корректно).
    ///
    /// `evdev_code` — это `KeyCode::code()` из evdev crate.
    pub fn update_key(&mut self, evdev_code: u16, pressed: bool) {
        let xkb_code = xkb::Keycode::new((evdev_code as u32) + 8);
        let direction = if pressed {
            xkb::KeyDirection::Down
        } else {
            xkb::KeyDirection::Up
        };
        self.state.update_key(xkb_code, direction);
    }

    /// Получить UTF-8 строку для нажатой клавиши с учётом текущей раскладки/модификаторов.
    /// Возвращает пустую строку для модификаторов (Shift/Ctrl/Alt) и клавиш без glyph
    /// (стрелки, F1, и т.п.).
    ///
    /// Вызывать **до** `update_key(_, false)`: семантика xkb — symbol соответствует
    /// **нажатому** состоянию.
    pub fn key_to_utf8(&self, evdev_code: u16) -> String {
        let xkb_code = xkb::Keycode::new((evdev_code as u32) + 8);
        self.state.key_get_utf8(xkb_code)
    }

    /// Имя символа (`Cyrillic_er`, `a`, `Return`, …) — пригодится для отладки и для
    /// детекта word-boundary punctuation независимо от группы.
    pub fn key_to_keysym_name(&self, evdev_code: u16) -> String {
        let xkb_code = xkb::Keycode::new((evdev_code as u32) + 8);
        let sym = self.state.key_get_one_sym(xkb_code);
        xkb::keysym_get_name(sym)
    }

    /// Индекс активной группы (0 = us, 1 = ru при текущем layout-list).
    pub fn active_group(&self) -> u32 {
        self.state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// KEY_A в evdev = 30. В us-layout даёт `a`. После переключения на ru даст `ф`.
    #[test]
    fn translate_key_a_in_us_layout() {
        let t = XkbTranslator::new().unwrap();
        // KEY_A = 30 evdev
        let s = t.key_to_utf8(30);
        assert_eq!(s, "a");
    }
}
