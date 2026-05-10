//! Виртуальная клавиатура для отправки KEY-events обратно в систему.
//!
//! WHY такой путь:
//!   - На Wayland нет универсального API «впечатать строку» в чужое окно. Только
//!     evdev/uinput на kernel-уровне работает с любым приложением одинаково.
//!   - Альтернативы:
//!     - `wtype` (virtual-keyboard-v1 protocol) — fork-exec на каждое слово,
//!       плохо с не-ASCII, не везде поддерживается. Отброшено.
//!     - `ydotool` — то же что мы тут, но через системный daemon. Лишний слой.
//!     - AT-SPI editable-text — лучше для приложений где есть, но не везде. Это
//!       Milestone 6 (накроет ~70% случаев) поверх uinput-fallback'а.
//!
//! WHY re-emit keycodes (не char→keycode reverse):
//!   - WordBuffer хранит сырые evdev keycodes, которые юзер физически нажал.
//!   - При flip мы (a) переключаем системную раскладку, (b) шлём те же keycodes
//!     заново. Они переинтерпретируются compositor'ом в новой раскладке и дают
//!     корректные глифы. Никакого ручного char→keycode reverse-mapping не нужно.
//!
//! WHY EVIOCGRAB опционален:
//!   - На системе пользователя стоит keyd, который сам грабит physical клаву и
//!     создаёт virtual. Если matea ещё раз grab'ит keyd's virtual — конфликт.
//!   - В v0.1 идём без grab'а. Race condition (юзер успевает нажать клавишу
//!     посреди rewrite) принимаем — это вопрос на M5b (короткая блокировка
//!     через grab keyd-virtual именно перед emit).

use anyhow::{Context, Result};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use tracing::{debug, info};

/// Единая virtual-клавиатура matea, используется для emit corrections.
pub struct Rewriter {
    device: VirtualDevice,
}

impl Rewriter {
    /// Создать virtual keyboard `matea` с full keymap. После этого в системе
    /// появляется `/dev/input/event<N>` с этим именем — приложения видят его как
    /// обычную клаву.
    pub fn new() -> Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        // Объявляем поддержку всех «обычных» key-codes (1..=255 покрывает буквы,
        // цифры, modifiers, punkt, F-клавиши, multimedia). 0 — KEY_RESERVED, skip.
        for code in 1..=255u16 {
            keys.insert(KeyCode::new(code));
        }

        let device = VirtualDeviceBuilder::new()
            .context("uinput: create builder (нужен RW на /dev/uinput; обычно достаточно группы input)")?
            .name("matea virtual keyboard")
            .with_keys(&keys)
            .context("uinput: with_keys")?
            .build()
            .context("uinput: build (если EACCES — проверь /dev/uinput права)")?;

        info!("uinput virtual keyboard создан: matea virtual keyboard");
        Ok(Self { device })
    }

    /// Эмитим одно нажатие+отпускание keycode'а. SYN_REPORT шлёт evdev сам в emit().
    pub fn tap(&mut self, keycode: u16) -> Result<()> {
        let press = InputEvent::new(EventType::KEY.0, keycode, 1);
        let release = InputEvent::new(EventType::KEY.0, keycode, 0);
        self.device
            .emit(&[press, release])
            .context("uinput: emit tap")?;
        Ok(())
    }

    /// Backspace × n.
    pub fn backspace(&mut self, n: usize) -> Result<()> {
        // KEY_BACKSPACE = 14
        let bs = KeyCode::KEY_BACKSPACE.code();
        debug!(n, "uinput: backspace");
        for _ in 0..n {
            self.tap(bs)?;
        }
        Ok(())
    }

    /// Re-emit последовательности keycodes (которые юзер уже нажимал). Compositor
    /// проинтерпретирует их с учётом текущей системной раскладки.
    pub fn replay_keycodes(&mut self, keycodes: &[u16]) -> Result<()> {
        debug!(?keycodes, "uinput: replay");
        for &kc in keycodes {
            self.tap(kc)?;
        }
        Ok(())
    }
}
