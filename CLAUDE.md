# CLAUDE.md

> Этот файл загружается Claude Code при работе с репо. Все универсальные
> правила вынесены в [`AGENTS.md`](AGENTS.md) — **сначала прочитай его**.
> Здесь только Claude-специфичные дополнения.

## Сначала прочитай

1. [`AGENTS.md`](AGENTS.md) — обзор проекта, стек, грабли, что не делать
2. [`docs/NEXT_STEPS.md`](docs/NEXT_STEPS.md) — детальный план каждого
   milestone'а с архитектурой, подводными камнями, шагами реализации
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — модули, конфиг, граф
   потока данных

## Claude-specific

- **Память пользователя**: ключевые feedback-правила про этот проект могут
  быть в `~/.claude/projects/-home-gold/memory/project_matea.md` и
  `feedback_no_claude_coauthor.md`. Они подгружаются автоматически на старте
  сессии у владельца — но если ты Claude в другой сессии (CI, fork, другой
  user), их не будет. Все load-bearing правила продублированы в `AGENTS.md`.
- **Skill'ы**: для этого проекта специальных skill'ов не нужно. Стандартные
  Read/Edit/Write/Bash/Agent — достаточно.
- **Длинные операции** (cargo build с llama_cpp_2 в v0.2 будет минут 5):
  использовать Bash `run_in_background` + ждать notification, не sleep loop.
- **Live smoke-тестирование** (запустить matea и наблюдать ключи): использовать
  `Monitor` с `tail -F /tmp/matea.log`, не прямой pipe (tracing-subscriber
  батчит в pipe + ANSI escape codes ломают grep по field-name).
- **Never** добавлять `Co-Authored-By: Claude ...` в commit messages. Прямой
  запрос владельца, перекрывает дефолтные правила Claude Code.
