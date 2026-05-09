use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use evdev::{Device, EventSummary, KeyCode};
use std::path::PathBuf;
use tracing::{debug, info, warn};

use super::Platform;

/// Linux реализация (Wayland-first, X11 не поддерживаем).
///
/// План v0.1 — итерация за итерацией:
///   - [x] evdev reader: открыть `/dev/input/event*`, отфильтровать клавиатуры,
///         читать events асинхронно, логировать нажатия.  ← **ЭТА ИТЕРАЦИЯ**
///   - [ ] WordBuffer: накапливать символы, отдавать на word boundary.
///   - [ ] Classifier: hunspell + n-gram → Verdict.
///   - [ ] uinput rewriter: BS×N + replacement, EVIOCGRAB во время записи.
///   - [ ] zbus: переключение раскладки через KGlobalAccel/Layouts.
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

#[async_trait]
impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux (Wayland)"
    }

    async fn run(&self) -> Result<()> {
        // Каждой клавиатуре — свою задачу. tokio mpsc-канал собирает события в один поток.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<KeyEvent>(256);

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

        info!("evdev reader запущен. Нажми Ctrl+C для выхода. Печатай в любом окне — события будут логироваться.");

        // Главный loop: либо событие от клавиатуры (логируем), либо Ctrl+C (выходим).
        loop {
            tokio::select! {
                Some(ev) = rx.recv() => {
                    debug!(
                        kbd = %ev.kbd_name,
                        key = ?ev.key,
                        state = if ev.pressed { "↓" } else { "↑" },
                        "key"
                    );
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("получен Ctrl+C, завершаюсь");
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

#[derive(Debug)]
struct KeyEvent {
    kbd_name: String,
    key: KeyCode,
    pressed: bool,
}

async fn read_keyboard(
    device: Device,
    name: String,
    path: PathBuf,
    tx: tokio::sync::mpsc::Sender<KeyEvent>,
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
                    .send(KeyEvent {
                        kbd_name: name.clone(),
                        key,
                        pressed,
                    })
                    .await
                    .is_err()
                {
                    // receiver gone — main loop вышел
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Найти клавиатуры через evdev: filter by EV_KEY supported и наличие keycode KEY_A
/// (грубо, но достаточно — клавиатура без буквы A это не клавиатура).
async fn discover_keyboards() -> Result<Vec<KeyboardDevice>> {
    let mut found = Vec::new();
    for (path, dev) in evdev::enumerate() {
        let name = dev.name().unwrap_or("<unnamed>").to_string();
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
