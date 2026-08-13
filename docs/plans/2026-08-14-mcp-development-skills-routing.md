# MCP Development Skills Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Закрепить в корневом `AGENTS.md` обязательную маршрутизацию трёх навыков `mcp-server-dev`, их официальный источник и способы установки для Codex, Claude Code и других совместимых агентов.

**Architecture:** Меняется только инструкция будущим агентам и разработчикам. Один новый раздел в `AGENTS.md` задаёт обязательную точку входа, условные специализированные ветки и fail-closed установку отсутствующего комплекта; публичная MCP-поверхность, код и `spec/` не меняются.

**Tech Stack:** Markdown, Python 3.12 для структурной проверки, Git.

## Global Constraints

- `build-mcp-server` обязателен для проектирования, создания и развития MCP-сервера Unica.
- `build-mcp-app` обязателен только для MCP Apps, UI-ресурсов и интерактивных виджетов.
- `build-mcpb` обязателен только для MCPB, локальной упаковки и поставки MCP-сервера.
- Совмещённая MCP App + MCPB работа использует порядок `build-mcp-server` → `build-mcp-app` → `build-mcpb`.
- Канонический источник — `https://github.com/anthropics/claude-plugins-official/tree/main/plugins/mcp-server-dev`.
- Если хотя бы один навык недоступен, MCP-работа не начинается до установки всего комплекта.
- Архитектурный контракт Unica не меняется; ADR и правки `spec/` не создаются.

---

### Task 1: Добавить обязательную маршрутизацию и установку skills

**Files:**
- Modify: `AGENTS.md` перед разделом `## Правила разработки`
- Reference: `docs/design/2026-08-14-mcp-development-skills-routing-design.md`
- Test: одноразовая структурная проверка содержимого `AGENTS.md`

**Interfaces:**
- Consumes: три установленных навыка `build-mcp-server`, `build-mcp-app`, `build-mcpb` и системный Codex-навык `skill-installer`.
- Produces: один видимый будущим агентам раздел `## Обязательные навыки разработки MCP` с маршрутизацией, источником и инструкциями установки.

- [x] **Step 1: Запустить структурную проверку и подтвердить RED**

Run:

```bash
python3.12 - <<'PY'
from pathlib import Path

text = Path("AGENTS.md").read_text(encoding="utf-8")
required = [
    "## Обязательные навыки разработки MCP",
    "`build-mcp-server`",
    "`build-mcp-app`",
    "`build-mcpb`",
    "anthropics/claude-plugins-official",
    "/plugin install mcp-server-dev",
]
missing = [value for value in required if value not in text]
assert not missing, f"missing MCP skill policy entries: {missing}"
assert text.index("## Обязательные навыки разработки MCP") < text.index("## Правила разработки")
PY
```

Expected: FAIL с `missing MCP skill policy entries`, потому что раздел ещё не добавлен.

- [x] **Step 2: Вставить минимальный нормативный раздел в `AGENTS.md`**

Перед `## Правила разработки` добавить дословно:

````markdown
## Обязательные навыки разработки MCP

При любой задаче, которая проектирует, создаёт или развивает MCP-сервер Unica,
до проектирования и реализации обязательно используйте `build-mcp-server`.
После выбора ветки работы применяйте специализированные навыки:

- `build-mcp-app` обязателен для MCP Apps, UI-ресурсов и интерактивных виджетов;
- `build-mcpb` обязателен для MCPB, локальной упаковки и поставки MCP-сервера;
- работа, совмещающая интерактивный интерфейс и MCPB-поставку, использует
  навыки в порядке `build-mcp-server` → `build-mcp-app` → `build-mcpb`.

Не применяйте `build-mcp-app` или `build-mcpb` к работе, которая не затрагивает
их контур. Канонический источник комплекта — официальный пакет Anthropic
[`mcp-server-dev`](https://github.com/anthropics/claude-plugins-official/tree/main/plugins/mcp-server-dev).

Если хотя бы один обязательный навык недоступен, не начинайте MCP-работу, пока
не установлен весь комплект. В Codex используйте системный `skill-installer`
с repository `anthropics/claude-plugins-official`, ref `main` и путями:

- `plugins/mcp-server-dev/skills/build-mcp-server`;
- `plugins/mcp-server-dev/skills/build-mcp-app`;
- `plugins/mcp-server-dev/skills/build-mcpb`.

В Claude Code установите официальный плагин:

```text
/plugin marketplace add anthropics/claude-plugins-official
/plugin install mcp-server-dev
```

Для другого совместимого агента скопируйте каждый каталог навыка целиком —
`SKILL.md` вместе с `references/` — в каталог skills этого агента. Официальное
руководство: [Build with Agent Skills](https://modelcontextprotocol.io/docs/2026-07-28/develop/build-with-agent-skills).
````

- [x] **Step 3: Повторить структурную проверку и подтвердить GREEN**

Run: команда из Step 1.

Expected: exit code 0 без вывода.

- [x] **Step 4: Проверить документационные контракты и формат diff**

Run:

```bash
python3.12 -m unittest tests.ci.test_design_documents
git diff --check
git diff -- AGENTS.md
```

Expected: 9 тестов проходят; `git diff --check` не выводит ошибок; diff содержит только согласованный новый раздел в `AGENTS.md`.

- [x] **Step 5: Зафиксировать реализацию отдельным коммитом**

```bash
git add AGENTS.md docs/plans/2026-08-14-mcp-development-skills-routing.md
git commit -m "docs(agents): require MCP development skills"
```
