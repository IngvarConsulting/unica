---
id: DEC.2026-08-21.LEGACY-UNKNOWN-NATIVE-SOURCE-FORMAT
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/tool_context.rs::native_platform_xml_source_format_public_gate_is_closed_over_public_operations
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.PLATFORM-XML-ONLY]
---

# Исторически неразмеченный источник допускается отдельно

**Решение.** Контекстный гейт физически адресованных нативных операций
принимает не только явный `platform_xml`, но и исторический `unknown`; EDT и
неоднозначная классификация отклоняются. Логически адресованные `role.edit`,
`code.patch`, `xdto.info` и `xdto.edit` не имеют физического пути на этой
границе и поэтому перечислены отдельно, а не считаются прошедшими этот гейт.
Это фиксирует действующую совместимость вместо переноса более строгой
универсальной формулировки v1.

**Почему.** Старые проекты без маркера формата уже поддерживаются, а общий
контекстный гейт не видит путь логической цели до диспетчеризации обработчика.

**Цена.** `unknown` не означает доказанный platform XML; его допуск остаётся
узкой совместимостью, а не новым форматом источников.
