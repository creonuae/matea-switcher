use anyhow::Result;
use async_trait::async_trait;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod xkb;

/// Platform abstraction. Реализаций три (по таргетам):
/// - Linux: evdev/uinput + KWin DBus + AT-SPI
/// - macOS: CGEventTap + CGEvent + AXUIElement (заглушка в v0.1)
/// - Windows: WH_KEYBOARD_LL + SendInput + UI Automation (заглушка в v0.1)
///
/// Ядро (классификатор/LLM/маппер) платформо-независимое и общается с Platform
/// через этот trait.
///
/// `#[async_trait(?Send)]` — позволяет держать non-Send state (например xkb::State)
/// прямо в платформной impl. tokio запущен в `flavor = "current_thread"`, поэтому
/// Send-bound на future не нужен.
#[async_trait(?Send)]
pub trait Platform {
    /// Человеческое имя для логов.
    fn name(&self) -> &'static str;

    /// Запустить event loop. Платформа сама читает клавиши, дёргает классификатор
    /// и пишет коррекции обратно. Возвращается на graceful shutdown (Ctrl+C / SIGTERM).
    async fn run(&self) -> Result<()>;
}

pub async fn current() -> Result<Box<dyn Platform>> {
    #[cfg(target_os = "linux")]
    {
        let p = linux::LinuxPlatform::new().await?;
        Ok(Box::new(p))
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("only linux is supported in v0.1 — macOS/Windows in v0.4+")
    }
}
