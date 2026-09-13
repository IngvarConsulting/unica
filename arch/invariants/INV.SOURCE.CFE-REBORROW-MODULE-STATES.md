---
id: INV.SOURCE.CFE-REBORROW-MODULE-STATES
status: active
governs: product
decision: DEC.2026-09-14.CFE-BORROWED-STRUCTURE
check: crates/unica-coder/src/infrastructure/native_operations/cfe.rs::retained_cfe_reborrow_preserves_connected_modules_on_disk
scope: [source]
---

# Повторное внутреннее заимствование сохраняет подключённые модули

Два повторных вызова сохранённого внутреннего генератора для `Report`,
`InformationRegister`, `Constant`, `CommonModule`, `Bot`, `HTTPService`,
`WebService`, `IntegrationService` и `CommonCommand` сохраняют все заданные
`PropertyState=Extended` поддержанных модулей и байты их BSL-файлов.
Это проверка внутренней операции, не свидетельство её доступности через MCP.
