# v0.2 — встроенная локальная LLM (план)

> Дата: 2026-05-10. v0.1 закрыт, время заходить в LLM. Этот документ —
> якорь для следующей сессии: что делать, как, в какой последовательности.

## Зачем LLM

Hunspell + n-gram + recent_words context (M11) закрывают ~95% случаев:
полностью валидные слова → KEEP, полностью невалидные с валидным flip →
FLIP. Остаётся **5% UNCERTAIN** где dictionary lookup не помогает:

| Кейс | Hunspell | Что нужно от LLM |
|---|---|---|
| Короткие токены (`ye`/`не`, `cnj`/`сто`, `lf`/`да`) | UNCERTAIN (валидно в обеих) | Inferring из context recent_words / screen text |
| Имена собственные (`Anthropic`, `Москва`) | UNCERTAIN | Capitalized → бы**в**ает в обеих, нужен context |
| Опечатки + правильная раскладка (`helo` → en typo, не флипить) | UNCERTAIN | Понимание что это en typo, не cross-layout |
| Сленг / новояз / меmes (не в словаре, но валидный токен) | UNCERTAIN | LLM знает контекст |
| Mixed input (`Telegram-чат`) | KEEP via M7 | Уже закрыт rule'ом |

Plus три новых use case'а **только через LLM + AT-SPI**:

1. **Proactive layout prediction** на смену окна. AT-SPI читает текст
   видимый в активном окне → LLM решает «юзер скорее всего продолжит на
   ru или en?» → matea сразу выставляет нужную раскладку.
2. **Smart corrections**: если юзер сам частично исправил слово
   (backspace + новые символы), LLM может понять final intent.
3. **Tab-completion (v0.6+)**: подсказка продолжения слова/фразы по
   screen context.

## Стек (зафиксирован)

- **Модель:** `Qwen-2.5-0.5B-Instruct` GGUF, quantization `Q4_K_M`.
  - Размер: ~400MB.
  - Multilingual (RU + EN из коробки), хорошо работает на коротких
    промптах.
  - Latency: prefill 30-50мс на CPU 6 потоков (без GPU). Generation
    с GBNF grammar `keep|flip` — first token <30мс.
- **Inference engine:** `llama.cpp`. Два варианта подключения:
  - **A)** in-process через `llama-cpp-2` Rust crate.
  - **B)** sidecar — запускаем `llama-server` как child process,
    общаемся по HTTP (localhost:8080).
  - **Решение:** **B (sidecar)** для v0.2. Reasons:
    - llama-server — стандартный binary который llama.cpp сама поставляет;
      stable, multi-platform, model loading/cache management уже сделаны.
    - HTTP overhead localhost ~0.5мс — мизер на фоне 30-50мс inference.
    - Decoupled: matea процесс не блокируется на CMake build llama_cpp_2
      во время первой компиляции (cargo build пока ждёт CMake).
    - Migration на in-process в v0.3 если нужно урезать stratup latency
      или RAM footprint.
- **Скачивание модели:** `scripts/download-model.sh` — `curl` с
  HuggingFace `Qwen/Qwen2.5-0.5B-Instruct-GGUF`. Кешируется в
  `~/.local/share/matea-switcher/models/`. **НЕ** в git (LFS дорого).
- **Запуск llama-server:** auto-spawn как child process matea на старте,
  port задан в config.toml; alive весь процесс matea.
  - Альтернатива — отдельный systemd unit для llama-server.
    Решение в `M_v0.2c`.

## Roadmap v0.2 (по милстонам)

### v0.2-M1 — model download script + binary install

- `scripts/download-model.sh` — pure curl, проверка sha256.
- `scripts/install-llama-server.sh` — собрать llama.cpp из source или
  скачать релизный binary с GitHub. Решить какой путь после research.
- `~/.local/share/matea-switcher/models/qwen2.5-0.5b-q4_k_m.gguf`.
- `~/.local/bin/llama-server`.

### v0.2-M2 — async HTTP client wrapper

- `src/llm.rs` — обёртка над `reqwest` (или `hyper`) к llama-server
  HTTP API.
- POST `/completion` с `prompt`, `grammar` (GBNF), `n_predict: 5`,
  `cache_prompt: true` (для prefix-caching).
- Тайм-аут 100мс (если LLM не успел — fallback Verdict::Uncertain как
  раньше = matea не флипает).
- 1-2 unit-теста с mock HTTP-server.

### v0.2-M3 — start/stop llama-server child process

- На init `Llm::new()` запускает `tokio::process::Command::new("llama-server")`
  с args `--model <path> --port 8080 --ctx-size 512 -t 4`.
- Health check: ждём `GET /health` 200 OK до 5 секунд, иначе fail с
  понятной ошибкой.
- На Drop matea — kill child.
- 1 unit-тест (mock spawn fail → graceful no-LLM mode).

### v0.2-M4 — integration в classifier slow-path

- Расширить `ClassifyInput` на новое поле: `screen_context: Option<&str>`
  (передаётся из AT-SPI после M6b).
- При `Verdict::Uncertain` от Hunspell → `LlmClassifier::classify(input)
  -> Verdict`.
- GBNF grammar:
  ```
  root ::= "keep" | "flip"
  ```
- Prompt template (тестируем варианты, выбираем по точности):
  ```
  Ты определяешь правильную раскладку клавиатуры. Билингв пишет на RU+EN.
  Контекст последних слов: {recent}
  Активная раскладка: {layout}
  Кандидат: {word}
  Альтернатива при flip: {flipped}
  Ответь одним словом: keep или flip.
  ```
- Бенчмарк latency: цель — p50 < 50мс, p99 < 100мс на CPU.

### v0.2-M5 — proactive layout prediction (нужен M6b screen context)

- AT-SPI читает visible text в активном окне (последние ~500 chars).
- LLM получает `screen_context` + `current_layout` → предсказывает
  layout для следующего ввода.
- На `KWin activeWindowChanged` — pre-emptively вызываем
  `kwin.set_layout(predicted)`. matea **до того** как юзер начал печатать.
- Per-window memory (HashMap<window_id, last_layout>) — если LLM не
  уверен, fallback к last-used.

### v0.2-M6 — config

- `[llm]` секция в config.toml:
  ```toml
  [llm]
  enabled = true
  model_path = "~/.local/share/matea-switcher/models/qwen2.5-0.5b-q4_k_m.gguf"
  server_binary = "llama-server"
  port = 8080
  threads = 4
  timeout_ms = 100
  ```
- Если `enabled = false` — slow-path skip, всегда Uncertain → no flip
  (как v0.1). Можно держать matea без LLM в RAM-constrained системах.

## Что НЕ войдёт в v0.2

- **Tab-autocomplete** (v0.6+). Это другой prompt template + bigger context
  window + UI overlay.
- **Voice mode / dictation correction** — далекое будущее.
- **Multi-language (3+) layouts** — пара RU↔EN остаётся в фокусе.
- **Custom user training** (LoRA fine-tune на пользовательских поправках)
  — v1.0+.

## Что нужно сделать для v0.2 *перед* кодингом

1. **Live verification M6** на dev-машине — запустить matea, проверить:
   - В gedit FLIP работает.
   - В Konsole FLIP suppressed + log "focus context blocks".
   - В Firefox `<input type=password>` FLIP suppressed.
2. **M6b EditableText** реализован (опционально — можно сделать
   параллельно с v0.2-M1).
3. **Системные deps:** llvm-devel (для llama.cpp build), make/cmake.
   Уже стоят на dev-машине.

## Где это документировано в репе

- `docs/NEXT_STEPS.md → v0.2 LLM` — высокоуровневый список (старее).
- `docs/v02_llm_plan.md` — этот файл (детальный, актуальный 2026-05-10).
- AGENTS.md ссылается сюда после v0.1 closed.
