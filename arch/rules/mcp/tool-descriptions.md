---
id: INV.SURFACE.ARGUMENTS-DESCRIBED
check:
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_tools_are_described_within_wire_budget
  - crates/unica-coder/src/application/v13/tool_catalog.rs::canonical_arguments_are_described_within_wire_budget
---

# Каталог инструментов описывает аргументы и укладывается в бюджет клиента

Каждый опубликованный инструмент и каждое объявленное поле его аргументов
имеют непустое описание назначения. Это относится и к вложенным полям,
и к аргументам трёх инструментов совместимости с заданиями.

Описание отдельного инструмента не превышает 2 KiB. Каталог `tools/list`
профиля совместимости укладывается в 16 KiB; проверка измеряет его
сериализованный JSON-RPC-ответ с числовым идентификатором запроса.
