//! Детектор keyd на старте matea-switcher с инструкцией если нужна
//! настройка чтобы избежать echo-loop.
//!
//! ## Проблема
//!
//! keyd (https://github.com/rvaiya/keyd) — популярный keyboard remapper на
//! Linux. Он подписывается на ВСЕ input devices в системе (через udev), включая
//! наш `matea-switcher virtual keyboard`. Когда matea-switcher делает FLIP-emit
//! через uinput, keyd обрабатывает эти events и пере-эмитит их через свою
//! `keyd virtual keyboard`. В результате compositor видит наши events дважды:
//!   1. Напрямую с нашего virtual device (event14)
//!   2. После keyd-обработки через keyd's virtual (event15)
//!
//! Юзер видит **дубль** каждого rewrite'нутого символа в окне.
//!
//! ## Решение
//!
//! keyd config поддерживает `[ids]` секцию с blacklist'ом по vendor:product:
//!
//! ```
//! [ids]
//! -6d61:7465
//! ```
//!
//! (минус = ignore это устройство). Наш virtual keyboard идёт с
//! vendor=0x6d61, product=0x7465 (см. `uinput.rs`). После добавления и
//! `sudo systemctl reload keyd` echo-loop закрыт.
//!
//! Этот модуль детектит keyd на старте и **печатает warning** с инструкцией.
//! Сами не трогаем чужой config — это юзерская система.

use std::path::Path;
use tracing::{info, warn};

/// Возвращает true если в системе обнаружен keyd. Проверяем:
///   - Есть ли бинарь по типичным путям.
///   - Запущен ли процесс (через /proc/*/comm).
///
/// Не падает на error'ах — детекция best-effort, неуспех = «не keyd»,
/// продолжаем без warning.
pub fn is_keyd_present() -> bool {
    if keyd_binary_exists() {
        return true;
    }
    keyd_process_running()
}

fn keyd_binary_exists() -> bool {
    const PATHS: &[&str] = &[
        "/usr/local/bin/keyd",
        "/usr/bin/keyd",
        "/opt/keyd/bin/keyd",
    ];
    PATHS.iter().any(|p| Path::new(p).exists())
}

fn keyd_process_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        // /proc/<pid> — pid это число; пропускаем не-числовые.
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            if comm.trim() == "keyd" {
                return true;
            }
        }
    }
    false
}

/// Если keyd обнаружен — печатает warning с инструкцией. Идемпотентно
/// (один раз на старт matea-switcher).
pub fn warn_if_keyd_active() {
    if !is_keyd_present() {
        return;
    }
    warn!("keyd detected — без настройки будет echo-loop (дубли символов в окне после FLIP)");
    info!(
        "Для починки: добавь в ~/.config/keyd/default.conf секцию `[ids]` со \
         строкой `-6d61:7465`, затем `sudo systemctl reload keyd`. \
         Подробнее — docs/keyd-setup.md в репе."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Проверка что функция не паникует если /proc недоступен или пуст.
    /// Не можем mock'нуть filesystem без extra crate'ов; полагаемся на то
    /// что unit-тест запускается на Linux где /proc есть.
    #[test]
    fn detect_does_not_panic() {
        let _ = is_keyd_present();
    }
}
