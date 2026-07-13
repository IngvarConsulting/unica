---
name: extension-point-discovery
description: "Обязательный discovery/preflight типовых механизмов и точек расширения 1С. Используй перед планированием или реализацией изменения существующей типовой конфигурации, CFE-доработки, перехвата метода, изменения формы, обработки, документа, табличной части или другого объекта на поддержке."
---

# Extension Point Discovery

## MCP routing

- Follow MCP-first discipline: start with the public discovery tool and keep every fallback behind the same public Unica boundary.
- Preferred path: use MCP `unica` tool `unica.project.discover` before changing a typical 1C configuration or implementing its extension.
- Use public `unica.project.*`, `unica.meta.*`, `unica.form.*`, and `unica.code.*` tools only to close checks explicitly reported as missing by discovery.
- Do not call internal RLM, index, analyzer, SQLite, or package adapters directly. They are hidden behind MCP `unica`.

## Mandatory preflight policy

1. Выполни discovery до любого mutating MCP-вызова и до ручного изменения XML/BSL, если задача затрагивает существующий типовой механизм 1С.
2. Передай исходную формулировку задачи в `task`, а известные объекты — каноническими ссылками `Type.Name` в `objects`. Не выдумывай имена метаданных.
3. Передай `proposedExtensionPoints` только когда уже есть предполагаемые точки реализации. Это позволяет discovery проверить гипотезу и сформировать адресные warnings.
4. Разбери candidates, evidence, warnings и missing checks. Отделяй факт из метаданных от лексического совпадения и от подтверждённого графа вызовов.
5. Выбери точку расширения и зафиксируй причину выбора. Не начинай изменения до выбора точки расширения и её подтверждения достаточными evidence.

## Discovery call

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.project.discover",
    "arguments": {
      "cwd": "<workspace>",
      "sourceDir": ".",
      "task": "При поступлении товаров контролировать остаточный срок годности серий"
    }
  }
}
```

The example assumes that the workspace root is the configuration source root. Replace `sourceDir` with the real source-set directory, or omit it when `unica.project.map` finds exactly one configuration source-set. `objects` and `proposedExtensionPoints` are optional. Add only references already confirmed by project evidence; omit both when discovery starts from the task alone. Never copy example object names as placeholders.

## Structured result

Read the machine payload from top-level `OperationResult.data`. Require `data.schemaVersion: 1` and inspect all fields:

```json
{
  "data": {
    "schemaVersion": 1,
    "status": "partial",
    "source": {
      "sourceDir": "src/configuration",
      "sourceSet": "main",
      "sourceFormat": "platform_xml"
    },
    "keywords": ["Серии", "ГоденДо", "ДатаПроизводства"],
    "candidateExtensionPoints": [
      {
        "object": "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
        "kind": "tabular_section",
        "score": 45,
        "confidence": "medium",
        "reasonCodes": ["keyword_match", "metadata_structure"],
        "reason": "Документ содержит отдельную табличную часть серий.",
        "evidence": [
          "platform_xml: src/configuration/Documents/ПриобретениеТоваровУслуг.xml#TabularSection.Серии"
        ]
      }
    ],
    "warnings": [
      {
        "code": "separate_tabular_section",
        "message": "Проверка только по ТЧ Товары может не покрыть типовой сценарий.",
        "objects": [
          "Document.ПриобретениеТоваровУслуг.TabularSection.Товары",
          "Document.ПриобретениеТоваровУслуг.TabularSection.Серии"
        ],
        "evidence": [
          "platform_xml: src/configuration/Documents/ПриобретениеТоваровУслуг.xml#TabularSection.Серии"
        ]
      }
    ],
    "evidence": [
      {
        "source": "platform_xml",
        "target": "Document.ПриобретениеТоваровУслуг.TabularSection.Серии",
        "path": "src/configuration/Documents/ПриобретениеТоваровУслуг.xml",
        "finding": "Platform XML declares tabular section `Серии`."
      }
    ],
    "missingChecks": [
      {
        "check": "unica.code.definition",
        "status": "not_run",
        "reason": "index_stale",
        "detail": "BSL index must be refreshed before method-flow confirmation."
      }
    ]
  }
}
```

- Treat `candidateExtensionPoints` as hypotheses ranked by evidence, not as permission to edit every listed object.
- Treat `warnings` about support state, a separate tabular section, or a conflicting proposed point as architecture risks requiring resolution. A definitive warning against a planned point requires that point in `proposedExtensionPoints`; without it, treat the wording as conditional.
- Treat `status: "partial"` as a usable partial result, not a clean preflight. Preserve metadata and lexical evidence already found, then close material `missingChecks` where possible. `status: "complete"` means the scheduled checks completed; it does not guarantee that candidates were found.
- Do not infer runtime flow from lexical evidence alone. Require definition/graph evidence when the choice depends on a caller, handler, or method interception.

## Fallback discipline

1. For every material entry in `missingChecks`, try the corresponding public Unica surface: `unica.project.map`, object-specific `unica.meta.info` or `unica.form.info`, then targeted `unica.code.search`, `unica.code.grep`, `unica.code.definition`, or `unica.code.graph`.
2. Use exact identifiers before broad business terms. Keep the query, path, and line anchor for each fallback finding.
3. Use local `rg` only after the public MCP checks did not close the gap or for repository files outside the Unica index. State what MCP check was attempted and label shell matches as lexical fallback.
4. Never query the RLM SQLite database or invoke bundled analyzers directly.
5. If a missing check can change the chosen architecture, stop before mutation and report the unresolved check. Do not silently downgrade it to a warning.

## Selection gate

Before implementation, record:

- the selected extension point and rejected alternatives;
- metadata/form/code evidence supporting the choice;
- whether the target is on support or locked and whether CFE is required;
- warnings that affect coverage, especially separate tabular sections or typical input forms;
- unresolved `missingChecks` and why none can invalidate the chosen point.

Only after this gate, continue with the relevant edit, CFE, form, metadata, or test skill.
