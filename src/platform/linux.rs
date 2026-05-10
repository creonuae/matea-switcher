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
use crate::context::{WordBuffer, WordHistory, is_word_boundary_char};

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

    async fn run(&self, cfg: &crate::config::Config) -> Result<()> {
        let mut xkb = XkbTranslator::new().context("init xkbcommon")?;
        let mut buffer = WordBuffer::default();
        let mut history = WordHistory::new(10);
        let classifier = DictClassifier::new_default()
            .context("init Hunspell словарей (en_US + ru_RU)")?;
        info!("classifier готов: en_US + ru_RU словари загружены");

        // Список путей клавиатур передаём в Rewriter — там же будут открыты
        // отдельные fd для EVIOCGRAB на время do_flip (M5d). reader продолжает
        // держать свои fd; grab на наших новых fd блокирует events для всех
        // клиентов, в т.ч. reader'а (kernel-level), что закрывает race.
        let grab_paths: Vec<_> = self.keyboards.iter().map(|k| k.path.clone()).collect();
        let mut rewriter = Rewriter::new(grab_paths).context("init uinput rewriter")?;
        let kwin = KwinLayout::new()
            .await
            .context("init KWin layout DBus client")?;
        info!("rewriter и kwin DBus готовы — переписывание включено");

        // M8: глобальный enabled-flag и tracking modifier-state. Aварийный
        // hotkey Ctrl+Shift+M toggle'ит rewrite ON/OFF (classifier продолжает
        // работать и логировать, но FLIP-action пропускается). Если matea сходит
        // с ума или попадаешь в окно где она опасна — нажми Ctrl+Shift+M.
        let mut enabled: bool = cfg.general.enabled;
        // M9c: парсим hotkey из config один раз, затем O(1) проверка на каждом event.
        let toggle_hotkey = crate::config::Hotkey::parse(&cfg.hotkeys.toggle)
            .with_context(|| format!("invalid config.hotkeys.toggle = '{}'", cfg.hotkeys.toggle))?;
        let mut mod_state = ModState::default();
        info!(
            initial_enabled = enabled,
            toggle_hotkey = %cfg.hotkeys.toggle,
            "hotkey toggle ON/OFF из config (classifier продолжает крутиться при OFF)"
        );

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
                    mod_state.update(&ev);
                    if check_toggle_hotkey_v2(&ev, &mod_state, &toggle_hotkey) {
                        enabled = !enabled;
                        info!(enabled, "matea toggle через Ctrl+Shift+M");
                        // Сброс буфера чтобы не «доцеплять» к недопечатанному слову
                        buffer.take();
                        continue;
                    }
                    if let Err(e) = handle_event(&mut xkb, &mut buffer, &mut history, &classifier, &mut rewriter, &kwin, &cfg.layouts.pair, &ev, enabled).await {
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
    history: &mut WordHistory,
    classifier: &DictClassifier,
    rewriter: &mut Rewriter,
    kwin: &KwinLayout,
    pair: &[String],
    ev: &RawKeyEvent,
    enabled: bool,
) -> Result<()> {
    // M5c: если этот press — echo нашего собственного rewrite (через keyd
    // virtual keyboard вернулся к нам), пропускаем без обработки. Без этого
    // self-loop: наши emit'ы попадают в WordBuffer и вызывают новый FLIP →
    // повторный rewrite → ещё больше echo → хаос (наблюдалось в smoke 05:42).
    if ev.pressed && rewriter.maybe_consume_self_echo() {
        debug!(keycode = ?ev.key, "ignored self-echo");
        // Всё равно обновим xkb state — чтобы modifier/group учёт не отстал.
        xkb.update_key(ev.evdev_code, true);
        return Ok(());
    }

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
                        // M11: передаём контекст последних слов для bias на ambiguous case
                        let recent: Vec<String> = history.recent().cloned().collect();
                        let verdict = classifier.classify(&ClassifyInput {
                            word: &t.word,
                            active_layout: &t.layout,
                            recent_words: &recent,
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
                                do_flip(rewriter, kwin, pair, &t).await?;
                                // После flip — добавим в history именно flipped (юзер
                                // будет видеть это слово, и контекст должен это отражать).
                                history.push(flipped.clone());
                            } else {
                                debug!(word = %t.word, "FLIP suppressed (matea disabled)");
                                history.push(t.word.clone());
                            }
                        } else {
                            // Keep / Uncertain — в history идёт оригинал что юзер ввёл.
                            history.push(t.word.clone());
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
async fn do_flip(
    rewriter: &mut Rewriter,
    kwin: &KwinLayout,
    pair: &[String],
    t: &crate::context::TakenWord,
) -> Result<()> {
    // M9c: target_layout определяем по cfg.layouts.pair (динамически).
    // pair = ["us", "ru"] значит us↔ru. Если pair длиннее (3+ раскладки),
    // в v0.1 поддерживается только пара — берём первые два не-current'а.
    let target_name = match pair.iter().find(|name| name.as_str() != t.layout.as_str()) {
        Some(t) => t.as_str(),
        None => {
            warn!(layout = %t.layout, ?pair, "do_flip: в config.layouts.pair нет alternative, skip");
            return Ok(());
        }
    };
    let Some(target_index) = kwin.index_of(target_name) else {
        warn!(
            target = %target_name,
            "do_flip: target раскладка отсутствует в KWin LayoutList — добавь её в System Settings → Keyboard"
        );
        return Ok(());
    };

    info!(
        word = %t.word,
        keycodes = ?t.keycodes,
        target_layout_index = target_index,
        "FLIP: переписываю"
    );

    // M5d: EVIOCGRAB на все клавы перед rewrite. Юзерский ввод буферизуется
    // в evdev пока мы не отпустим — это закрывает race condition (юзерские
    // символы не вставляются между нашими backspace и replay).
    let grabbed = rewriter.grab_all();
    if grabbed == 0 {
        warn!("grab_all не сработал ни на одной клавиатуре — race окно открыто");
    }

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

    // 5. Release grab — юзер снова может печатать. Буферизованные events
    // (если он что-то нажал во время grab) придут к reader'у сейчас.
    rewriter.ungrab_all();

    Ok(())
}

/// State модификаторов в реальном времени. Используем raw evdev keycodes — не
/// xkb keysym — потому что для Ctrl/Shift/Alt/Meta нужны именно физические
/// клавиши, не зависящие от раскладки.
#[derive(Debug, Default, Clone, Copy)]
struct ModState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

impl ModState {
    fn update(&mut self, ev: &RawKeyEvent) {
        match ev.evdev_code {
            // KEY_LEFTCTRL=29, KEY_RIGHTCTRL=97
            29 | 97 => self.ctrl = ev.pressed,
            // KEY_LEFTSHIFT=42, KEY_RIGHTSHIFT=54
            42 | 54 => self.shift = ev.pressed,
            // KEY_LEFTALT=56, KEY_RIGHTALT=100
            56 | 100 => self.alt = ev.pressed,
            // KEY_LEFTMETA=125, KEY_RIGHTMETA=126
            125 | 126 => self.meta = ev.pressed,
            _ => {}
        }
    }
}

/// Detect совпадение event с конфигурируемым hotkey. Срабатывает только на
/// pressed-event main клавиши (чтобы autorepeat не толкал toggle N раз).
/// Все 4 модификатора должны точно совпадать (ни одного лишнего).
fn check_toggle_hotkey_v2(
    ev: &RawKeyEvent,
    state: &ModState,
    hotkey: &crate::config::Hotkey,
) -> bool {
    ev.pressed
        && ev.evdev_code == hotkey.keycode
        && state.ctrl == hotkey.ctrl
        && state.shift == hotkey.shift
        && state.alt == hotkey.alt
        && state.meta == hotkey.meta
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
    fn mod_state_tracks_all_modifiers() {
        let mut s = ModState::default();
        s.update(&ev(29, true));     // left ctrl
        assert!(s.ctrl);
        s.update(&ev(42, true));     // left shift
        assert!(s.shift);
        s.update(&ev(56, true));     // left alt
        assert!(s.alt);
        s.update(&ev(125, true));    // left meta
        assert!(s.meta);
        s.update(&ev(97, false));    // right ctrl release
        assert!(!s.ctrl);
        // Right variants работают как один логический modifier
        s.update(&ev(54, false));    // right shift release
        assert!(!s.shift);
    }

    #[test]
    fn hotkey_v2_exact_modifier_match() {
        let hotkey = crate::config::Hotkey::parse("Ctrl+Shift+M").unwrap();
        let none = ModState::default();
        let ctrl_only = ModState { ctrl: true, ..Default::default() };
        let ctrl_shift = ModState { ctrl: true, shift: true, ..Default::default() };
        let ctrl_shift_alt = ModState { ctrl: true, shift: true, alt: true, ..Default::default() };

        assert!(!check_toggle_hotkey_v2(&ev(50, true), &none, &hotkey));
        assert!(!check_toggle_hotkey_v2(&ev(50, true), &ctrl_only, &hotkey));
        assert!(check_toggle_hotkey_v2(&ev(50, true), &ctrl_shift, &hotkey));
        // Лишний Alt — НЕ срабатывает (exact match)
        assert!(!check_toggle_hotkey_v2(&ev(50, true), &ctrl_shift_alt, &hotkey));
        // На release не реагируем
        assert!(!check_toggle_hotkey_v2(&ev(50, false), &ctrl_shift, &hotkey));
    }

    #[test]
    fn hotkey_v2_pause_no_modifiers() {
        let hotkey = crate::config::Hotkey::parse("Pause").unwrap();
        let none = ModState::default();
        let ctrl_only = ModState { ctrl: true, ..Default::default() };
        // Только KEY_PAUSE без модификаторов
        assert!(check_toggle_hotkey_v2(&ev(119, true), &none, &hotkey));
        // С Ctrl — НЕ срабатывает
        assert!(!check_toggle_hotkey_v2(&ev(119, true), &ctrl_only, &hotkey));
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
