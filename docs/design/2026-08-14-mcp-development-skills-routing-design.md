- Date: `2026-08-14`
- Status: `approved`
- Decision: `none` — no architectural contract changed

# Обязательная маршрутизация навыков разработки MCP

## Цель

Закрепить в `AGENTS.md` обязательное использование установленных навыков
`build-mcp-server`, `build-mcp-app` и `build-mcpb` при соответствующих работах
над MCP-сервером Unica, а также дать разработчику канонический источник и
однозначный способ установить отсутствующие навыки.

## Решение

`build-mcp-server` становится обязательной точкой входа для проектирования,
создания и развития MCP-сервера. Специализированные навыки подключаются не
безусловно, а по выбранной ветке работы:

- `build-mcp-app` обязателен для MCP Apps, UI-ресурсов и интерактивных виджетов;
- `build-mcpb` обязателен для MCPB, локальной упаковки и поставки MCP-сервера;
- работа, совмещающая интерактивный интерфейс и MCPB-поставку, использует все
  три навыка в порядке `build-mcp-server` → `build-mcp-app` → `build-mcpb`.

Правило не заставляет применять UI- или MCPB-навык к работе, которая не
затрагивает соответствующий контур. Это сохраняет специализацию навыков и не
создаёт фиктивных этапов при обычном изменении MCP-инструмента.

## Источник и установка

Канонический источник — официальный пакет Anthropic `mcp-server-dev`:

<https://github.com/anthropics/claude-plugins-official/tree/main/plugins/mcp-server-dev>

Если хотя бы один обязательный навык недоступен, агент не начинает MCP-работу,
а сначала устанавливает весь комплект. Для Codex используется системный навык
`skill-installer` со следующими координатами:

- repository: `anthropics/claude-plugins-official`;
- ref: `main`;
- paths:
  `plugins/mcp-server-dev/skills/build-mcp-server`,
  `plugins/mcp-server-dev/skills/build-mcp-app`,
  `plugins/mcp-server-dev/skills/build-mcpb`.

Для Claude Code указываются официальные команды установки плагина:

```text
/plugin marketplace add anthropics/claude-plugins-official
/plugin install mcp-server-dev
```

Для другого агента разработчик копирует каждый каталог навыка целиком —
`SKILL.md` вместе с `references/` — в каталог skills этого агента. Эти способы
соответствуют руководству MCP «Build with Agent Skills»:

<https://modelcontextprotocol.io/docs/2026-07-28/develop/build-with-agent-skills>

## Размещение

Правило добавляется отдельным разделом `Обязательные навыки разработки MCP`
перед разделом `Правила разработки` в корневом `AGENTS.md`. Другие документы,
код, тесты, публичная MCP-поверхность и упаковочный контракт не меняются.

## Верификация

- В `AGENTS.md` явно названы все три навыка и условия их обязательного
  применения.
- Указан порядок совместного применения `build-mcp-server`, `build-mcp-app` и
  `build-mcpb`.
- Указаны официальный источник, точные пути установки для Codex и команды
  установки для Claude Code.
- При отсутствии навыка правило требует установить весь комплект до начала
  MCP-работы.
- `git diff --check` не сообщает ошибок форматирования.
- `tests/ci/test_design_documents.py` принимает шапку проектной записки.
