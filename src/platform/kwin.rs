//! Тонкий zbus-клиент для KDE Plasma KWin keyboard service.
//!
//! WHY:
//!   - При Verdict::Flip нам нужно переключить системную раскладку (us↔ru) ДО
//!     re-emit keycodes — иначе compositor проинтерпретирует их в старой
//!     раскладке и выдаст тот же текст что юзер уже стёр backspace'ом.
//!   - В Plasma 6 D-Bus name `org.kde.keyboard`, объект `/Layouts`, интерфейс
//!     `org.kde.KeyboardLayouts` имеет:
//!       - `getLayoutsList() -> a(ssss)` — массив layouts
//!       - `getLayout() -> u` — индекс активного
//!       - `setLayout(u)` — переключить на индекс
//!       - signal `layoutChanged(u)` — компилятор поменял (для proactive M6+)
//!
//! WHY async через zbus.tokio:
//!   - matea живёт в tokio current-thread runtime, всё IO async.
//!
//! WHY не xkb_state.update_layout(group):
//!   - Это бы поменяло **только наш локальный** xkb-state, а не системный
//!     compositor — приложения продолжали бы видеть прежнюю раскладку, наш
//!     re-emit бы дал тот же текст. Нужно дёргать именно KWin'овский layout.

use anyhow::{Context, Result};
use tracing::debug;
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
trait KeyboardLayouts {
    fn getLayout(&self) -> zbus::Result<u32>;
    fn setLayout(&self, index: u32) -> zbus::Result<bool>;
}

pub struct KwinLayout {
    proxy: KeyboardLayoutsProxy<'static>,
}

impl KwinLayout {
    pub async fn new() -> Result<Self> {
        let conn = Connection::session()
            .await
            .context("zbus: connect to session bus")?;
        let proxy = KeyboardLayoutsProxy::new(&conn)
            .await
            .context("zbus: create org.kde.keyboard proxy")?;
        Ok(Self { proxy })
    }

    /// Текущий активный layout index (0 = первый в LayoutList = us, 1 = ru).
    pub async fn current(&self) -> Result<u32> {
        self.proxy.getLayout().await.context("getLayout")
    }

    /// Переключить активный layout. Возвращает true если успешно.
    /// **Внимание:** compositor применяет смену не моментально. Перед re-emit
    /// keycodes лучше дождаться следующего tick'а или коротко поспать (~30мс).
    pub async fn set(&self, index: u32) -> Result<bool> {
        debug!(index, "KWin: setLayout");
        self.proxy.setLayout(index).await.context("setLayout")
    }
}
