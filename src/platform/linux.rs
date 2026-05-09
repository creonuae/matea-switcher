use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};

use super::Platform;

/// Linux реализация (Wayland-first, X11 не поддерживаем).
///
/// План v0.1 внутри этого файла:
///   1. Открыть `/dev/input/event*` через `evdev::Device::open_all()`,
///      отфильтровать только клавиатуры (`EV_KEY` + наличие keycodes букв/space/enter).
///   2. Создать `uinput`-устройство для записи коррекций.
///   3. На каждый ev из evdev — добавить keycode в `WordBuffer`. На space/enter/punct —
///      запустить classifier.
///   4. Если classifier вернул `Flip { backspace: N, replacement: String, layout_to: ... }`:
///      - `EVIOCGRAB` физическую клавиатуру (чтоб юзер не успел напечатать ещё)
///      - в uinput: `KEY_BACKSPACE` × N, потом replacement как UTF-8 (через text-input)
///      - переключить раскладку через `qdbus org.kde.keyboard /Layouts setLayout`
///        или прямой DBus call через `zbus`
///      - `EVIOCGRAB` release
///
/// Параллельно (v0.3+):
///   5. AT-SPI listener в фоне читает контекст активного окна (для proactive prediction).
///   6. KWin DBus signal `org.kde.KWin.activeWindowChanged` — на смену окна выставлять
///      раскладку по контексту.
pub struct LinuxPlatform {
    // TODO: handle на uinput-устройство, evdev devices, AT-SPI connection, DBus connection
}

impl LinuxPlatform {
    pub async fn new() -> Result<Self> {
        // TODO: проверить что мы в группе input (`/dev/input/event*` доступны на чтение)
        // TODO: создать uinput-устройство
        // TODO: подключиться к session DBus (KWin)
        Ok(Self {})
    }
}

#[async_trait]
impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux (Wayland)"
    }

    async fn run(&self) -> Result<()> {
        warn!("v0.1 platform not implemented yet — exiting cleanly");
        info!("next steps: see TODO in src/platform/linux.rs");
        // Жди Ctrl+C чтобы было видно что бинарь живой
        tokio::signal::ctrl_c().await?;
        info!("shutdown signal received");
        Ok(())
    }
}
