use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use evdev::{Device, EventSummary, KeyCode};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::Platform;
use super::kwin::KwinLayout;
use super::uinput::Rewriter;
use super::xkb::XkbTranslator;
use crate::classifier::{ClassifyInput, DictClassifier, Verdict};
use crate::context::{WordBuffer, is_word_boundary_char};

/// Linux реализация (Wayland-first).
///
/// План v0.1:
///   - [x] M1 evdev reader
///   - [x] M2 xkbcommon translation
///   - [x] M3 WordBuffer + boundary
///   - [x] M4 Hunspell classifier
///   - [x] M5 uinput rewriter + KWin layout switch на Verdict::Flip   ← **СЕЙЧАС**
///   - [ ] M6 AT-SPI integration (uses editable-text где есть, fallback на uinput)
///   - [ ] M7 classifier hardening (digits/URL/capitalized — Keep)
pub struct LinuxPlatform {
    keyboards: Vec<KeyboardDevice>,
}

struct KeyboardDevice {
    path: PathBuf,
    name: String,
    device: Option<Device>,
}

impl LinuxPlatform {
    pub async fn new() -> Result<Self> {
        let keyboards = discover_keyboards()
            .await
            .context("failed to discover keyboards via evdev")?;
        if keyboards.is_empty() {
            bail!(
                "не нашёл ни одной клавиатуры через evdev. \
                 Проверь что юзер в группе `input` (groups | grep input) \
                 и что /dev/input/event* читаемы."
            );
        }
        info!("обнаружено клавиатур: {}", keyboards.len());
        for kb in &keyboards {
            info!("  • {} ({})", kb.name, kb.path.display());
        }
        Ok(Self { keyboards })
    }
}

#[async_trait(?Send)]
impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux (Wayland)"
    }

    async fn run(&self) -> Result<()> {
        let mut xkb = XkbTranslator::new().context("init xkbcommon")?;
        let mut buffer = WordBuffer::default();
        let classifier = DictClassifier::new_default()
            .context("init Hunspell словарей (en_US + ru_RU)")?;
        info!("classifier готов: en_US + ru_RU словари загружены");

        let mut rewriter = Rewriter::new().context("init uinput rewriter")?;
        let kwin = KwinLayout::new()
            .await
            .context("init KWin layout DBus client")?;
        info!("rewriter и kwin DBus готовы — переписывание включено");

        // M8: глобальный enabled-flag и tracking modifier-state. Aварийный
        // hotkey Ctrl+Shift+M toggle'ит rewrite ON/OFF (classifier продолжает
        // работать и логировать, но FLIP-action пропускается). Если matea сходит
        // с ума или попадаешь в окно где она опасна — нажми Ctrl+Shift+M.
        let mut enabled: bool = true;
        let mut ctrl_pressed = false;
        let mut shift_pressed = false;
        info!("hotkey: Ctrl+Shift+M — toggle rewrite ON/OFF (classifier продолжает крутиться)");

        let (tx, mut rx) = mpsc::channel::<RawKeyEvent>(256);

        let mut devices: Vec<KeyboardDevice> = self
            .keyboards
            .iter()
            .map(|k| KeyboardDevice {
                path: k.path.clone(),
                name: k.name.clone(),
                device: None,
            })
            .collect();
        for kb in devices.iter_mut() {
            let dev = Device::open(&kb.path)
                .with_context(|| format!("открыть {}", kb.path.display()))?;
            kb.device = Some(dev);
        }

        let mut tasks = Vec::new();
        for kb in devices {
            let tx = tx.clone();
            let name = kb.name.clone();
            let path = kb.path.clone();
            let dev = kb.device.expect("device opened above");
            tasks.push(tokio::spawn(async move {
                if let Err(e) = read_keyboard(dev, name, path, tx).await {
                    warn!("keyboard reader exited: {e:#}");
                }
            }));
        }
        drop(tx);

        info!("xkb инициализирован (us,ru, grp:alt_space_toggle)");
        info!("evdev reader запущен. Печатай — на Verdict::Flip matea сама перепишет слово.");

        loop {
            tokio::select! {
                Some(ev) = rx.recv() => {
                    update_modifiers(&ev, &mut ctrl_pressed, &mut shift_pressed);
                    if check_toggle_hotkey(&ev, ctrl_pressed, shift_pressed) {
                        enabled = !enabled;
                        info!(enabled, "matea toggle через Ctrl+Shift+M");
                        // Сброс буфера чтобы не «доцеплять» к недопечатанному слову
                        buffer.take();
                        continue;
                    }
                    if let Err(e) = handle_event(&mut xkb, &mut buffer, &classifier, &mut rewriter, &kwin, &ev, enabled).await {
                        warn!("handle_event error: {e:#}");
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("получен Ctrl+C, завершаюсь");
                    if !buffer.is_empty() {
                        let t = buffer.take();
                        info!(word = %t.word, layout = %t.layout, "недозавершённое слово на shutdown");
                    }
                    break;
                }
            }
        }

        for t in tasks {
            t.abort();
        }
        Ok(())
    }
}

/// Главный switch. На press с непустым utf8 — пушим в буфер. На boundary —
/// classify, и если Verdict::Flip — переключаем системную раскладку через KWin
/// DBus, ждём 50мс, эмитим backspace × N + replay keycodes (uinput интерпретирует
/// их в новой раскладке и даёт корректный текст в активном окне).
async fn handle_event(
    xkb: &mut XkbTranslator,
    buffer: &mut WordBuffer,
    classifier: &DictClassifier,
    rewriter: &mut Rewriter,
    kwin: &KwinLayout,
    ev: &RawKeyEvent,
    enabled: bool,
) -> Result<()> {
    if ev.pressed {
        let utf8 = xkb.key_to_utf8(ev.evdev_code);
        let keysym_name = xkb.key_to_keysym_name(ev.evdev_code);
        let group = xkb.active_group();
        let layout = match group {
            0 => "us",
            1 => "ru",
            _ => "??",
        };

        // Backspace — снимаем последний символ из буфера.
        if keysym_name == "BackSpace" {
            buffer.pop();
            debug!(buf = %buffer.as_str(), "← backspace");
            xkb.update_key(ev.evdev_code, true);
            return Ok(());
        }

        if !utf8.is_empty() {
            let mut chars = utf8.chars();
            if let Some(first) = chars.next() {
                if is_word_boundary_char(first) {
                    if !buffer.is_empty() {
                        let t = buffer.take();
                        let verdict = classifier.classify(&ClassifyInput {
                            word: &t.word,
                            active_layout: &t.layout,
                            recent_words: &[],
                            window_class: None,
                        });
                        let flipped = match t.layout.as_str() {
                            "us" => crate::mapper::en_to_ru(&t.word),
                            "ru" => crate::mapper::ru_to_en(&t.word),
                            _ => String::new(),
                        };
                        let verdict_str = match verdict {
                            Verdict::Keep => "KEEP",
                            Verdict::Flip => "FLIP",
                            Verdict::Uncertain => "UNCERTAIN",
                        };
                        info!(
                            word = %t.word,
                            flipped = %flipped,
                            layout_started = %t.layout,
                            current_layout = layout,
                            verdict = verdict_str,
                            "WORD"
                        );

                        if matches!(verdict, Verdict::Flip) {
                            if enabled {
                                do_flip(rewriter, kwin, &t).await?;
                            } else {
                                debug!(word = %t.word, "FLIP suppressed (matea disabled)");
                            }
                        }
                    }
                } else {
                    buffer.push(first, layout, ev.evdev_code);
                }
            }

            debug!(
                kbd = %ev.kbd_name,
                keycode = ?ev.key,
                utf8 = %utf8,
                keysym = %keysym_name,
                layout,
                buf = %buffer.as_str(),
                "key"
            );
        } else {
            debug!(
                kbd = %ev.kbd_name,
                keycode = ?ev.key,
                keysym = %keysym_name,
                layout,
                "key (no glyph)"
            );
        }
    }

    xkb.update_key(ev.evdev_code, ev.pressed);
    Ok(())
}

/// Выполнить flip-action: переключить системную раскладку и переписать слово.
///
/// Семантика:
///   1. Стираем напечатанное BS×N. Боундари-чарик (space/punct) который только
///      что юзер ввёл — НЕ стираем; он останется на месте, а перед ним будет
///      переписанное слово.
///   2. Переключаем системную раскладку через KWin DBus.
///   3. Ждём 50мс — даём compositor'у применить layout до того как мы шлём
///      keycodes (иначе первая буква вылетит в старой раскладке).
///   4. Re-emit keycodes — те же физические клавиши даём в новой раскладке,
///      получаем корректный текст в активном окне.
async fn do_flip(rewriter: &mut Rewriter, kwin: &KwinLayout, t: &crate::context::TakenWord) -> Result<()> {
    let target_index: u32 = match t.layout.as_str() {
        "us" => 1,  // ru — assuming kxkbrc LayoutList = ru,us... но typically [0]=us,[1]=ru
        "ru" => 0,
        _ => {
            warn!(layout = %t.layout, "do_flip: unknown layout, skip");
            return Ok(());
        }
    };

    info!(
        word = %t.word,
        keycodes = ?t.keycodes,
        target_layout_index = target_index,
        "FLIP: переписываю"
    );

    // 1. Стираем то что юзер уже напечатал.
    rewriter
        .backspace(t.keycodes.len())
        .context("flip: backspace")?;

    // 2. Переключаем раскладку.
    let _ok = kwin
        .set(target_index)
        .await
        .context("flip: KWin setLayout")?;

    // 3. Даём compositor'у применить.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 4. Re-emit keycodes — теперь они дадут глифы новой раскладки.
    rewriter
        .replay_keycodes(&t.keycodes)
        .context("flip: replay")?;

    Ok(())
}

/// Обновить tracking-state модификаторов. Используем raw evdev keycodes — не
/// xkb keysym — потому что для Ctrl/Shift нужны именно физические клавиши, не
/// зависящие от раскладки.
fn update_modifiers(ev: &RawKeyEvent, ctrl: &mut bool, shift: &mut bool) {
    // KEY_LEFTCTRL = 29, KEY_RIGHTCTRL = 97
    // KEY_LEFTSHIFT = 42, KEY_RIGHTSHIFT = 54
    match ev.evdev_code {
        29 | 97 => *ctrl = ev.pressed,
        42 | 54 => *shift = ev.pressed,
        _ => {}
    }
}

/// Detect нажатие Ctrl+Shift+M (KEY_M = 50). Срабатывает только на pressed-event
/// самой M (чтобы repeat-key autorepeat = 2 не толкал toggle несколько раз).
fn check_toggle_hotkey(ev: &RawKeyEvent, ctrl: bool, shift: bool) -> bool {
    ev.pressed && ev.evdev_code == 50 && ctrl && shift
}

#[derive(Debug)]
struct RawKeyEvent {
    kbd_name: String,
    key: KeyCode,
    evdev_code: u16,
    pressed: bool,
}

async fn read_keyboard(
    device: Device,
    name: String,
    path: PathBuf,
    tx: mpsc::Sender<RawKeyEvent>,
) -> Result<()> {
    debug!("listening on {}", path.display());
    let mut stream = device
        .into_event_stream()
        .with_context(|| format!("в event stream {}", path.display()))?;
    loop {
        let ev = stream
            .next_event()
            .await
            .with_context(|| format!("read evdev {}", path.display()))?;
        if let EventSummary::Key(_, key, value) = ev.destructure() {
            // value: 0 = release, 1 = press, 2 = autorepeat
            if value == 0 || value == 1 {
                let pressed = value == 1;
                if tx
                    .send(RawKeyEvent {
                        kbd_name: name.clone(),
                        key,
                        evdev_code: key.code(),
                        pressed,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Найти физические/виртуальные клавиатуры. Игнорируем НАШЕ собственное virtual
/// устройство (matea virtual keyboard) — иначе self-loop при rewrite.
#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: u16, pressed: bool) -> RawKeyEvent {
        RawKeyEvent {
            kbd_name: "test".into(),
            key: KeyCode::new(code),
            evdev_code: code,
            pressed,
        }
    }

    #[test]
    fn modifiers_track_ctrl() {
        let mut ctrl = false;
        let mut shift = false;
        update_modifiers(&ev(29, true), &mut ctrl, &mut shift);
        assert!(ctrl);
        update_modifiers(&ev(29, false), &mut ctrl, &mut shift);
        assert!(!ctrl);
    }

    #[test]
    fn modifiers_track_shift() {
        let mut ctrl = false;
        let mut shift = false;
        update_modifiers(&ev(42, true), &mut ctrl, &mut shift); // left shift
        assert!(shift);
        update_modifiers(&ev(54, false), &mut ctrl, &mut shift); // right shift release
        assert!(!shift);
    }

    #[test]
    fn hotkey_requires_both_modifiers() {
        // KEY_M alone — false
        assert!(!check_toggle_hotkey(&ev(50, true), false, false));
        // M + Ctrl only — false
        assert!(!check_toggle_hotkey(&ev(50, true), true, false));
        // M + Shift only — false
        assert!(!check_toggle_hotkey(&ev(50, true), false, true));
        // M + Ctrl + Shift — true
        assert!(check_toggle_hotkey(&ev(50, true), true, true));
        // на release не реагируем (даже если все модификаторы зажаты)
        assert!(!check_toggle_hotkey(&ev(50, false), true, true));
    }
}

async fn discover_keyboards() -> Result<Vec<KeyboardDevice>> {
    let mut found = Vec::new();
    for (path, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or("<unnamed>").to_string();
        if name.contains("matea") {
            // Не слушаем сами себя — иначе наш uinput emit вернётся обратно через
            // evdev и мы получим бесконечный цикл backspace+replay.
            debug!("skip self-device: {}", name);
            continue;
        }
        let supported = dev.supported_keys();
        let is_keyboard = supported
            .map(|set| set.contains(KeyCode::KEY_A))
            .unwrap_or(false);
        if is_keyboard {
            found.push(KeyboardDevice {
                path,
                name,
                device: None,
            });
        }
    }
    Ok(found)
}
