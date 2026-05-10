//! Конфиг matea.
//!
//! Читается из `~/.config/matea/config.toml` при старте. Если файла нет —
//! пишем дефолтный шаблон туда же (чтобы юзер сразу видел что можно крутить).
//!
//! WHY один TOML файл, а не env-vars / cli flags:
//!   - Хоткеи и blacklist'ы — длинные списки, в env-var неудобно.
//!   - Юзер часто хочет «забыть и не настраивать» — дефолтный config.toml
//!     с комментариями = живая документация.
//!   - Hot-reload (M9b) добавим через `notify` crate когда понадобится.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub layouts: Layouts,
    #[serde(default)]
    pub hotkeys: Hotkeys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct General {
    /// Глобальный enable. Если false — matea читает клавиши и логирует
    /// verdict, но никогда не делает FLIP. Полезно для debug-режима.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layouts {
    /// xkb-имена пары для flip. Сейчас поддерживается ровно одна пара
    /// (us↔ru), но архитектура готова к большему.
    #[serde(default = "default_layout_pair")]
    pub pair: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotkeys {
    /// Toggle matea ON/OFF. Зарезервировано на M9b — пока в коде hardcoded
    /// Ctrl+Shift+M (`KEY_M = 50`). Парсинг строки в keycode + modifier mask
    /// будет когда станет нужно вторая комбинация.
    #[serde(default = "default_toggle_hotkey")]
    pub toggle: String,
}

fn default_true() -> bool { true }
fn default_layout_pair() -> Vec<String> { vec!["us".into(), "ru".into()] }
fn default_toggle_hotkey() -> String { "Ctrl+Shift+M".into() }

impl Default for General {
    fn default() -> Self { Self { enabled: default_true() } }
}
impl Default for Layouts {
    fn default() -> Self { Self { pair: default_layout_pair() } }
}
impl Default for Hotkeys {
    fn default() -> Self { Self { toggle: default_toggle_hotkey() } }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            layouts: Layouts::default(),
            hotkeys: Hotkeys::default(),
        }
    }
}

/// Загрузить config из `~/.config/matea/config.toml`. Если файла нет —
/// записать дефолт и вернуть его. Если файл есть но битый — лог warning и
/// fallback на дефолты (НЕ падать — daemon должен жить).
pub fn load() -> Result<Config> {
    let path = config_path().context("определить config path")?;
    if !path.exists() {
        let default = Config::default();
        write_default(&path, &default)?;
        info!(path = %path.display(), "config: создан дефолтный");
        return Ok(default);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read config {}", path.display()))?;
    match toml::from_str::<Config>(&text) {
        Ok(c) => {
            info!(path = %path.display(), "config: загружен");
            Ok(c)
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                err = %e,
                "config: parse failed, использую дефолты"
            );
            Ok(Config::default())
        }
    }
}

fn config_path() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("BaseDirs::new")?;
    Ok(dirs.config_dir().join("matea-switcher").join("config.toml"))
}

fn write_default(path: &std::path::Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("mkdir config dir")?;
    }
    let header = "# matea-switcher config — см. AGENTS.md в репе для полного списка опций\n\
                  # Этот файл создан автоматически при первом запуске.\n\
                  # Удали — будет пересоздан с дефолтами.\n\n";
    let body = toml::to_string_pretty(cfg).context("serialize config")?;
    std::fs::write(path, format!("{header}{body}")).context("write config")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let c = Config::default();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert!(back.general.enabled);
        assert_eq!(back.layouts.pair, vec!["us", "ru"]);
        assert_eq!(back.hotkeys.toggle, "Ctrl+Shift+M");
    }

    #[test]
    fn parse_partial_uses_defaults() {
        // Юзер написал только [general] enabled=false — остальное дефолт
        let s = "[general]\nenabled = false\n";
        let c: Config = toml::from_str(s).unwrap();
        assert!(!c.general.enabled);
        assert_eq!(c.layouts.pair, vec!["us", "ru"]);
        assert_eq!(c.hotkeys.toggle, "Ctrl+Shift+M");
    }
}
