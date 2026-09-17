---
id: INV.WIRE.GUARANTEED-VERSIONS
check:
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_requires_both_lifecycles_and_the_exact_v13_compatibility_surface
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_requires_each_lifecycle_to_expose_each_public_tool
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_rejects_legacy_names_mixed_into_the_v13_surface
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_rejects_duplicate_names_in_the_v13_surface
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_rejects_discover_without_the_guaranteed_versions
  - crates/unica-bootstrap/tests/platform/verification_contract.rs::a_runtime_that_never_answers_is_a_timeout_not_a_defect
---

# Bootstrap проверяет протокол и состав инструментов перед приёмкой runtime

Проверка runtime проходит оба способа открытия MCP-соединения: `initialize`
с последующим `tools/list` и прямой `server/discover` с последующим
`tools/list`. В обоих случаях список должен точно соответствовать
[режиму совместимости](../mcp/mcp-tool-profiles.md). Отсутствующий, лишний
или повторяющийся инструмент приводит к отказу.

Ответ `server/discover` должен перечислять версии протокола `2025-06-18`,
`2025-11-25` и `2026-07-28`. Отсутствие любой из них также приводит к отказу.

Ожидание ответа ограничено переданным сроком. Runtime, который не отвечает,
даёт ошибку `Timeout` с кодом выхода `75`. Это ограничение отдельного ожидания,
а не общий срок всей установки и проверки.

Связанные проверки запускают подставной runtime на Unix. Они проверяют
работу bootstrap, но не доказывают предметное поведение инструментов или
поддержку всех возможностей перечисленных версий MCP.
