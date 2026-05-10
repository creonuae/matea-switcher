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
//! WHY EVIOCGRAB на источниках events во время rewrite (M5d):
//!   - Без grab между нашим backspace и replay юзер успевает напечатать новые
//!     символы. Они попадают в окно посреди rewrite → мешанина (наблюдалось
//!     2026-05-10 — `mctuj` от смеси нашего `всего` и юзерского `m`).
//!   - EVIOCGRAB на keyboard fd блокирует ВСЕХ клиентов (включая reader) от
//!     получения events. На время grab'а юзер «печатает в пустоту»
//!     (события буферизуются в evdev и приходят после ungrab).
//!   - Открытый вопрос с keyd: keyd сам grab'ит physical клаву. Наш grab на
//!     keyd virtual (event15) — отдельная цепочка, не должен конфликтовать.
//!     Если конфликт всё-таки — graceful degradation: warning + продолжаем
//!     без grab (старое поведение M5).

use anyhow::{Context, Result};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, BusType, Device, EventType, InputEvent, InputId, KeyCode};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Custom vendor/product ID для нашей virtual keyboard. Юзеры с keyd
/// используют эти hex-значения чтобы блэклистить matea в keyd config:
///
///   ~/.config/keyd/default.conf:
///   [ids]
///   -6d61:7465
///
/// Без этого keyd обрабатывает наши uinput-emit'ы и пере-эмитит их через
/// свою virtual keyboard — compositor видит дубль, юзер видит дубль слов
/// в окне. См. docs/keyd-setup.md.
///
/// Значения выбраны как ASCII-коды букв «ma» (0x6d61) и «te» (0x7465) — для
/// memorability. BUS_VIRTUAL (0x06) точнее по семантике чем BUS_USB.
const MATEA_VENDOR_ID: u16 = 0x6d61;
const MATEA_PRODUCT_ID: u16 = 0x7465;
const MATEA_VERSION: u16 = 0x0001;

/// Единая virtual-клавиатура matea-switcher для emit corrections + дополнительные
/// handles на физические/виртуальные клавы для EVIOCGRAB на время rewrite.
///
/// **pending_echo** — second line of defense на случай если grab упал/частичный:
/// keyd может пропустить наши emit'ы через свою virtual обратно к нам, мы
/// игнорируем по counter (см. `maybe_consume_self_echo`).
pub struct Rewriter {
    device: VirtualDevice,
    grab_devices: Vec<Device>,
    pending_echo: Option<(Instant, usize)>,
}

const SELF_ECHO_WINDOW: Duration = Duration::from_millis(500);

impl Rewriter {
    /// Создать virtual keyboard `matea` с full keymap. После этого в системе
    /// появляется `/dev/input/event<N>` с этим именем — приложения видят его как
    /// обычную клаву.
    pub fn new(grab_paths: Vec<PathBuf>) -> Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for code in 1..=255u16 {
            keys.insert(KeyCode::new(code));
        }

        // BUS_VIRTUAL=0x06 — точное обозначение для programmatic-only клав.
        // BusType публичный конструктор отсутствует; используем from(u16).
        let bus_virtual = BusType(0x06);
        let id = InputId::new(bus_virtual, MATEA_VENDOR_ID, MATEA_PRODUCT_ID, MATEA_VERSION);

        let device = VirtualDeviceBuilder::new()
            .context("uinput: create builder (нужен RW на /dev/uinput; обычно достаточно группы input)")?
            .name("matea-switcher virtual keyboard")
            .input_id(id)
            .with_keys(&keys)
            .context("uinput: with_keys")?
            .build()
            .context("uinput: build (если EACCES — проверь /dev/uinput права)")?;

        // Дополнительные handles на target клавы — нужны для EVIOCGRAB. Это
        // ОТДЕЛЬНЫЕ fd от тех что reader держит open для чтения; grab на нашем
        // fd блокирует events и для reader'а тоже (kernel-level).
        let mut grab_devices = Vec::new();
        for path in &grab_paths {
            match Device::open(path) {
                Ok(dev) => grab_devices.push(dev),
                Err(e) => warn!(path = %path.display(), err = %e, "не удалось open для grab — этот источник не будет блокироваться"),
            }
        }

        info!(
            grab_targets = grab_devices.len(),
            "uinput virtual keyboard создан: matea-switcher virtual keyboard"
        );
        Ok(Self {
            device,
            grab_devices,
            pending_echo: None,
        })
    }

    /// EVIOCGRAB на всех target devices. Возвращает количество успешно
    /// захваченных. После grab все остальные клиенты этих устройств перестают
    /// получать events до ungrab.
    pub fn grab_all(&mut self) -> usize {
        let mut grabbed = 0;
        for dev in &mut self.grab_devices {
            match dev.grab() {
                Ok(()) => grabbed += 1,
                Err(e) => debug!(err = %e, "grab failed (вероятно keyd конфликт или уже grabbed)"),
            }
        }
        debug!(grabbed, total = self.grab_devices.len(), "grab_all");
        grabbed
    }

    pub fn ungrab_all(&mut self) {
        for dev in &mut self.grab_devices {
            let _ = dev.ungrab();
        }
        debug!("ungrab_all");
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
