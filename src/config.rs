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

/// Парсенный hotkey: набор модификаторов + один main keycode.
///
/// WHY такая структура: при каждом keypress нам нужна O(1) проверка «соответствует
/// ли event текущей конфигурации». Парсим строку из config один раз на старте,
/// храним предкомпилированный struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    /// evdev keycode основной клавиши (KEY_M = 50, KEY_SPACE = 57, ...).
    pub keycode: u16,
}

impl Hotkey {
    /// Парсить строку формата `"Ctrl+Shift+M"`. Регистр не важен:
    /// `ctrl+shift+m` == `Ctrl+Shift+M`.
    /// Поддерживаемые модификаторы: Ctrl, Shift, Alt, Meta/Super/Win.
    /// Поддерживаемые клавиши: A-Z, 0-9, F1-F12, Space, Tab, Enter, Escape,
    /// Pause, любая буква русского алфавита НЕ поддерживается (используем по
    /// physical keycode который не зависит от раскладки).
    pub fn parse(s: &str) -> Result<Self> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut meta = false;
        let mut keycode = None;

        for part in s.split('+').map(|p| p.trim()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                "meta" | "super" | "win" | "windows" => meta = true,
                key => {
                    let kc = parse_keyname(key)
                        .with_context(|| format!("неизвестная клавиша '{key}' в hotkey '{s}'"))?;
                    if keycode.is_some() {
                        anyhow::bail!("в hotkey '{s}' больше одной не-модификаторной клавиши");
                    }
                    keycode = Some(kc);
                }
            }
        }

        let keycode = keycode
            .with_context(|| format!("в hotkey '{s}' не указана главная клавиша"))?;
        Ok(Self { ctrl, shift, alt, meta, keycode })
    }
}

fn parse_keyname(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        // Letters A-Z (KEY_A=30, KEY_B=48, KEY_C=46, ..., KEY_Z=44 — порядок не алфавитный!)
        "A" => Some(30), "B" => Some(48), "C" => Some(46), "D" => Some(32),
        "E" => Some(18), "F" => Some(33), "G" => Some(34), "H" => Some(35),
        "I" => Some(23), "J" => Some(36), "K" => Some(37), "L" => Some(38),
        "M" => Some(50), "N" => Some(49), "O" => Some(24), "P" => Some(25),
        "Q" => Some(16), "R" => Some(19), "S" => Some(31), "T" => Some(20),
        "U" => Some(22), "V" => Some(47), "W" => Some(17), "X" => Some(45),
        "Y" => Some(21), "Z" => Some(44),
        // Digits row
        "0" => Some(11), "1" => Some(2),  "2" => Some(3),  "3" => Some(4),
        "4" => Some(5),  "5" => Some(6),  "6" => Some(7),  "7" => Some(8),
        "8" => Some(9),  "9" => Some(10),
        // Common
        "SPACE" => Some(57),
        "TAB" => Some(15),
        "ENTER" | "RETURN" => Some(28),
        "ESC" | "ESCAPE" => Some(1),
        "PAUSE" => Some(119),
        "BACKSPACE" => Some(14),
        "CAPSLOCK" | "CAPS" => Some(58),
        // F-keys
        "F1" => Some(59), "F2" => Some(60), "F3" => Some(61), "F4" => Some(62),
        "F5" => Some(63), "F6" => Some(64), "F7" => Some(65), "F8" => Some(66),
        "F9" => Some(67), "F10" => Some(68), "F11" => Some(87), "F12" => Some(88),
        _ => None,
    }
}

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

    #[test]
    fn hotkey_parse_default() {
        let h = Hotkey::parse("Ctrl+Shift+M").unwrap();
        assert!(h.ctrl);
        assert!(h.shift);
        assert!(!h.alt);
        assert!(!h.meta);
        assert_eq!(h.keycode, 50); // KEY_M
    }

    #[test]
    fn hotkey_parse_case_insensitive() {
        let h = Hotkey::parse("ctrl+shift+m").unwrap();
        assert!(h.ctrl);
        assert!(h.shift);
        assert_eq!(h.keycode, 50);
    }

    #[test]
    fn hotkey_parse_meta_aliases() {
        for s in ["Meta+Space", "Super+Space", "Win+Space"] {
            let h = Hotkey::parse(s).unwrap();
            assert!(h.meta);
            assert_eq!(h.keycode, 57); // KEY_SPACE
        }
    }

    #[test]
    fn hotkey_parse_pause() {
        let h = Hotkey::parse("Pause").unwrap();
        assert!(!h.ctrl && !h.shift && !h.alt && !h.meta);
        assert_eq!(h.keycode, 119);
    }

    #[test]
    fn hotkey_parse_unknown_key() {
        assert!(Hotkey::parse("Ctrl+Foo").is_err());
    }

    #[test]
    fn hotkey_parse_no_main_key() {
        assert!(Hotkey::parse("Ctrl+Shift").is_err());
    }
}
