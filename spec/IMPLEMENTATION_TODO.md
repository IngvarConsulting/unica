# Открытые задачи `unica`

## Роль этого файла

Файл — короткая очередь немедленно исполнимой работы, а не полный трекер
проекта. Крупные изменения планируются задачами в GitHub, а их проектные
записки и планы лежат вне `spec/` — в [`docs/design/`](../docs/design/README.md)
и [`docs/plans/`](../docs/plans/README.md). Пустой список ниже означает пустую
очередь, а не отсутствие работы в проекте.

Источником архитектурных норм этот файл не является: нормы живут в реестре
[`architecture/invariants.md`](architecture/invariants.md) и в записях решений
[`decisions/README.md`](decisions/README.md).

## Текущие задачи

- Нет.

## Правила

- Only work that can start immediately belongs in this list; anything that
  needs multi-step design gets a dated plan outside `spec/` instead.
- A task that changes a public or architectural contract updates the owning
  decision record and the invariant registry in the same change set
  (`INV-MCP-08`, `INV-DOC-04`).

## Критерии завершённости

- The behavior is covered by a focused test that can be named from the
  invariant or the contract it protects.
- The invariant registry entry and the owning decision record are updated
  whenever the public contract changes (`INV-MCP-08`).
- `cargo run --quiet --bin unica -- --help` still identifies the binary as
  `unica`, and `initialize` still returns `serverInfo.name = "unica"`
  (`INV-MCP-03`).
- The public marketplace package keeps its thin launcher: its `.mcp.json`
  enters through the command-scoped Git shell alias that resolves the plugin
  root for both hosts and hands it to `bootstrap/launch.sh` (`INV-PKG-02`).
- The direct `bin/<target>/unica` launcher stays confined to the local debug
  package and is never published (`INV-PKG-07`).
- The verification commands below pass.

## Проверка

Набор команд не дублируется здесь: он живёт в разделе «Проверка»
[чек-листа изменений](architecture/change-checklist.md) и сверен с
`.github/workflows/unica-plugin-release.yml`. У этого списка один владелец,
иначе копии расходятся молча (`INV-DOC-08`).
