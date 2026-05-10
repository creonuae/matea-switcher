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
use std::collections::HashMap;
use tracing::{debug, info};
use zbus::{Connection, proxy};

/// zbus по дефолту конвертирует Rust `fn fooBar` → DBus `FooBar` (PascalCase).
/// KDE же использует camelCase (`setLayout`, `getLayout`) — поэтому каждой
/// функции нужен явный `name = "..."` атрибут. Открыли это smoke-тестом 2026-05-10:
/// без `name` падало с `org.freedesktop.DBus.Error.UnknownMethod: SetLayout`.
use futures_util::Stream;

#[proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
trait KeyboardLayouts {
    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> zbus::Result<u32>;

    #[zbus(name = "setLayout")]
    fn set_layout(&self, index: u32) -> zbus::Result<bool>;

    #[zbus(name = "switchToNextLayout")]
    fn switch_to_next_layout(&self) -> zbus::Result<()>;

    /// Возвращает список layouts: `[(short_name, display_name, long_name), ...]`.
    /// `short_name` — это xkb-layout name (`us`, `ru`, `de`, ...).
    /// Индекс в массиве == индекс который ждёт setLayout.
    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;
}

pub struct KwinLayout {
    proxy: KeyboardLayoutsProxy<'static>,
    /// Карта `xkb-layout-name → index в LayoutList`. Дёрнутая один раз на старте
    /// через `getLayoutsList()`. `do_flip` использует её чтобы найти target_index
    /// по имени ("ru" / "us"), а не hard-code'ить порядок.
    name_to_index: HashMap<String, u32>,
}

impl KwinLayout {
    pub async fn new() -> Result<Self> {
        let conn = Connection::session()
            .await
            .context("zbus: connect to session bus")?;
        let proxy = KeyboardLayoutsProxy::new(&conn)
            .await
            .context("zbus: create org.kde.keyboard proxy")?;
        let layouts = proxy
            .get_layouts_list()
            .await
            .context("zbus: getLayoutsList")?;
        let mut name_to_index = HashMap::new();
        for (i, (short, _display, _long)) in layouts.iter().enumerate() {
            info!(
                index = i,
                short = %short,
                display = %_display,
                long = %_long,
                "KWin layout"
            );
            name_to_index.insert(short.clone(), i as u32);
        }
        Ok(Self {
            proxy,
            name_to_index,
        })
    }

    /// Найти индекс layout по xkb-имени ("us", "ru", "de", ...). None если
    /// у юзера такой раскладки нет в kxkbrc.
    pub fn index_of(&self, name: &str) -> Option<u32> {
        self.name_to_index.get(name).copied()
    }

    /// Текущий активный layout index (0 = первый в LayoutList = us, 1 = ru).
    pub async fn current(&self) -> Result<u32> {
        self.proxy.get_layout().await.context("getLayout")
    }

    /// Переключить активный layout. Возвращает true если успешно.
    /// **Внимание:** compositor применяет смену не моментально. Перед re-emit
    /// keycodes лучше дождаться следующего tick'а или коротко поспать (~30мс).
    pub async fn set(&self, index: u32) -> Result<bool> {
        debug!(index, "KWin: setLayout");
        self.proxy.set_layout(index).await.context("setLayout")
    }

    /// Switch to next в LayoutList. Простая альтернатива set(target_index)
    /// если у нас 2 layouts и достаточно «другая».
    pub async fn switch_next(&self) -> Result<()> {
        self.proxy.switch_to_next_layout().await.context("switchToNextLayout")
    }
}
