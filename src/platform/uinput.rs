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
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Единая virtual-клавиатура matea, используется для emit corrections.
///
/// Tracking self-echo: keyd на этой системе grab'ит ВСЕ клавиатуры (включая
/// наш matea virtual keyboard) и шлёт events обратно через keyd virtual. Наш
/// `discover_keyboards` фильтрует устройства с "matea" в имени, но keyd
/// virtual в имени matea не содержит → events наших rewrites возвращаются и
/// reader их видит как user input → self-loop, хаос.
///
/// Решение: после каждого emit запоминаем (timestamp, expected_echo_count).
/// reader перед обработкой каждого press-event спрашивает
/// `maybe_consume_self_echo()` — если да (echo окно ещё не истекло, counter > 0),
/// event пропускается и counter--.
pub struct Rewriter {
    device: VirtualDevice,
    pending_echo: Option<(Instant, usize)>,
}

const SELF_ECHO_WINDOW: Duration = Duration::from_millis(500);

impl Rewriter {
    /// Создать virtual keyboard `matea` с full keymap. После этого в системе
    /// появляется `/dev/input/event<N>` с этим именем — приложения видят его как
    /// обычную клаву.
    pub fn new() -> Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=255u16 {
            keys.insert(KeyCode::new(code));
        }

        let device = VirtualDeviceBuilder::new()
            .context("uinput: create builder (нужен RW на /dev/uinput; обычно достаточно группы input)")?
            .name("matea-switcher virtual keyboard")
            .with_keys(&keys)
            .context("uinput: with_keys")?
            .build()
            .context("uinput: build (если EACCES — проверь /dev/uinput права)")?;

        info!("uinput virtual keyboard создан: matea-switcher virtual keyboard");
        Ok(Self {
            device,
            pending_echo: None,
        })
    }

    /// Reader спрашивает: «текущий press-event — это echo нашего собственного
    /// rewrite, который через keyd virtual keyboard вернулся к нам?»
    ///
    /// Возвращает true — игнорируй event. Возвращает false — обычный user input.
    /// Decrement'ит counter после каждого true. По истечении SELF_ECHO_WINDOW
    /// сбрасывает state (эхо могло потеряться, не висим вечно).
    pub fn maybe_consume_self_echo(&mut self) -> bool {
        if let Some((ts, count)) = self.pending_echo {
            if ts.elapsed() > SELF_ECHO_WINDOW {
                self.pending_echo = None;
                return false;
            }
            if count > 0 {
                self.pending_echo = Some((ts, count - 1));
                return true;
            }
        }
        false
    }

    fn arm_self_echo(&mut self, count: usize) {
        // Если предыдущая порция echo не успела догнаться — суммируем; в худшем
        // случае пропустим лишний user input в течение window, но это безопаснее
        // чем пропустить наш собственный echo и зациклить rewrite.
        let prev = match self.pending_echo {
            Some((_, c)) => c,
            None => 0,
        };
        self.pending_echo = Some((Instant::now(), prev + count));
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

    /// Backspace × n. Arm'ит self-echo counter ровно на n.
    pub fn backspace(&mut self, n: usize) -> Result<()> {
        let bs = KeyCode::KEY_BACKSPACE.code();
        debug!(n, "uinput: backspace");
        for _ in 0..n {
            self.tap(bs)?;
        }
        self.arm_self_echo(n);
        Ok(())
    }

    /// Re-emit последовательности keycodes (которые юзер уже нажимал). Arm'ит
    /// self-echo counter ровно на len(keycodes).
    pub fn replay_keycodes(&mut self, keycodes: &[u16]) -> Result<()> {
        debug!(?keycodes, "uinput: replay");
        for &kc in keycodes {
            self.tap(kc)?;
        }
        self.arm_self_echo(keycodes.len());
        Ok(())
    }
}
