//! AT-SPI listener: на каждый focus-change event обновляет shared FocusContext.
//!
//! WHY:
//!   - Без знания **где сейчас фокус** matea-switcher слепа: не различает
//!     password field от обычного text input, не знает что активный
//!     terminal (Konsole/Alacritty) и rewrite в нём всё сломает.
//!   - AT-SPI (`org.a11y.Bus`) — единственный portable путь на Linux читать
//!     accessibility metadata активного widget'а на Wayland.
//!
//! ARCHITECTURE:
//!   - Отдельная tokio task subscribe'ится на `Object:StateChanged:focused`.
//!   - На каждое event получает Accessible через AccessibleProxy, читает
//!     role + walks parents до Application для wm_class.
//!   - Обновляет `tokio::sync::watch::Sender<FocusContext>`. Главный loop
//!     читает текущий через `watch::Receiver::borrow()` — O(1), без блокировки.
//!
//! GRACEFUL DEGRADATION:
//!   - Если a11y-bus не запущен — listener возвращает Ok early, watch
//!     остаётся с дефолтом, allows_flip() == true, matea работает как M5d.
//!
//! THROTTLING:
//!   - Plasma alt-tab даёт burst 5-15 events за ~30мс. 50мс debounce.
//!
//! ОГРАНИЧЕНИЯ atspi 0.30:
//!   - В этой версии **нет** `State::Protected` варианта (полный enum смотрел
//!     эмпирически в registry/atspi-common-0.14.0/src/state.rs). Поэтому
//!     password detection только по `Role::PasswordText`. Это покрывает Qt
//!     QLineEdit с EchoMode::Password и GTK Entry с visibility=false.
//!     Если в будущей версии atspi появится State::Protected — добавим
//!     дополнительную проверку.

use anyhow::Result;
use atspi::{
    connection::AccessibilityConnection,
    events::{Event, ObjectEvents},
    proxy::accessible::AccessibleProxy,
    Role, State,
};
use futures_lite::stream::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Default)]
pub struct FocusContext {
    pub window_class: String,
    pub is_password: bool,
}

impl FocusContext {
    pub fn allows_flip(&self) -> bool {
        if self.is_password {
            return false;
        }
        if is_blacklisted_class(&self.window_class) {
            return false;
        }
        true
    }
}

pub fn spawn_listener() -> watch::Receiver<FocusContext> {
    let (tx, rx) = watch::channel(FocusContext::default());
    // tokio::task::spawn — runtime current_thread, но Send-bound на future
    // компилятор всё равно требует. zbus 5 proxy объекты Send + Sync.
    tokio::task::spawn(async move {
        match run_listener(tx).await {
            Ok(()) => debug!("AT-SPI listener exited cleanly"),
            Err(e) => warn!("AT-SPI listener error: {e:#}"),
        }
    });
    rx
}

async fn run_listener(tx: watch::Sender<FocusContext>) -> Result<()> {
    let conn = match AccessibilityConnection::new().await {
        Ok(c) => c,
        Err(e) => {
            warn!(
                err = %e,
                "AT-SPI bus недоступна — work без window blacklist (запустите at-spi2-registryd)"
            );
            return Ok(());
        }
    };
    info!("AT-SPI connected");

    // Регистрируем match-rule на StateChangedEvent, иначе stream молчит.
    conn.register_event::<atspi::events::object::StateChangedEvent>()
        .await?;
    let mut events = conn.event_stream();

    let mut last_handled = Instant::now() - Duration::from_secs(1);
    while let Some(maybe_ev) = events.next().await {
        let ev = match maybe_ev {
            Ok(e) => e,
            Err(e) => {
                debug!(err = %e, "AT-SPI event recv error");
                continue;
            }
        };

        if last_handled.elapsed() < Duration::from_millis(50) {
            continue;
        }

        let Event::Object(ObjectEvents::StateChanged(sc)) = ev else {
            continue;
        };
        // Интересуют только focus-acquired (state = Focused, enabled = true).
        if sc.state != State::Focused || !sc.enabled {
            continue;
        }
        last_handled = Instant::now();

        if let Err(e) = handle_focus(&conn, &sc, &tx).await {
            debug!(err = %e, "focus handler error");
        }
    }
    Ok(())
}

async fn handle_focus(
    conn: &AccessibilityConnection,
    sc: &atspi::events::object::StateChangedEvent,
    tx: &watch::Sender<FocusContext>,
) -> Result<()> {
    let dbus = conn.connection();
    // ObjectRefOwned: name() → Option<&UniqueName>, path() → &ObjectPath.
    let Some(item_name) = sc.item.name() else {
        return Ok(());
    };
    let item_path = sc.item.path();

    let acc = AccessibleProxy::builder(dbus)
        .destination(item_name.to_owned())?
        .path(item_path.to_owned())?
        .build()
        .await?;

    let role = acc.get_role().await.unwrap_or(Role::Invalid);
    // В этой версии atspi нет State::Protected. Password detection только
    // через Role::PasswordText. Покрывает Qt QLineEdit(Password) и GTK Entry.
    let is_password = role == Role::PasswordText;

    let mut window_class = String::new();
    let mut cur = acc;
    for _ in 0..16 {
        let cur_role: Role = cur.get_role().await.unwrap_or(Role::Invalid);
        if cur_role == Role::Application {
            let nm: String = cur.name().await.unwrap_or_default();
            window_class = nm.to_lowercase();
            break;
        }
        let parent = match cur.parent().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let Some(p_name) = parent.name() else {
            break;
        };
        let p_path = parent.path();
        if p_path.as_str() == "/" {
            break;
        }
        cur = AccessibleProxy::builder(dbus)
            .destination(p_name.to_owned())?
            .path(p_path.to_owned())?
            .build()
            .await?;
    }

    let ctx = FocusContext {
        window_class,
        is_password,
    };
    debug!(?ctx, "AT-SPI focus changed");
    let _ = tx.send(ctx);
    Ok(())
}

fn is_blacklisted_class(class: &str) -> bool {
    if class.is_empty() {
        return false;
    }
    const BLACKLIST: &[&str] = &[
        // Terminals
        "konsole",
        "yakuake",
        "alacritty",
        "kitty",
        "wezterm",
        "gnome-terminal",
        "xterm",
        "tilix",
        "terminator",
        // IDE
        "code",
        "codium",
        "vscodium",
        "jetbrains",
        "intellij",
        "pycharm",
        "webstorm",
        "rider",
        "clion",
        "sublime_text",
        // Password managers / sec-sensitive
        "keepassxc",
        "bitwarden",
        "1password",
    ];
    BLACKLIST.iter().any(|p| class.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_terminals() {
        assert!(is_blacklisted_class("konsole"));
        assert!(is_blacklisted_class("alacritty"));
        assert!(is_blacklisted_class("org.kde.konsole"));
    }

    #[test]
    fn blacklist_ide() {
        assert!(is_blacklisted_class("jetbrains-pycharm"));
        assert!(is_blacklisted_class("code-oss"));
    }

    #[test]
    fn blacklist_password_managers() {
        assert!(is_blacklisted_class("keepassxc"));
    }

    #[test]
    fn blacklist_normal_apps_pass() {
        assert!(!is_blacklisted_class("firefox"));
        assert!(!is_blacklisted_class("telegram-desktop"));
        assert!(!is_blacklisted_class("kate"));
        assert!(!is_blacklisted_class("gedit"));
    }

    #[test]
    fn empty_class_passes() {
        assert!(!is_blacklisted_class(""));
    }

    #[test]
    fn focus_context_allows_normal() {
        let c = FocusContext {
            window_class: "firefox".into(),
            is_password: false,
        };
        assert!(c.allows_flip());
    }

    #[test]
    fn focus_context_denies_password() {
        let c = FocusContext {
            window_class: "firefox".into(),
            is_password: true,
        };
        assert!(!c.allows_flip());
    }

    #[test]
    fn focus_context_denies_terminal() {
        let c = FocusContext {
            window_class: "konsole".into(),
            is_password: false,
        };
        assert!(!c.allows_flip());
    }

    #[test]
    fn focus_context_default_allows() {
        let c = FocusContext::default();
        assert!(c.allows_flip());
    }
}
