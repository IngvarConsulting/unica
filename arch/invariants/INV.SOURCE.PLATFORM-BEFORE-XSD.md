---
id: INV.SOURCE.PLATFORM-BEFORE-XSD
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check: crates/unica-coder/src/infrastructure/native_operations/dcs.rs::dcs_compile_rejects_defined_type_that_platform_8_3_27_drops_without_write
scope: [source]
---

# Наблюдаемая платформа строже разрешительного XSD

Компилятор DCS отклоняет без записи определённый тип, который допускает схема,
но удаляет платформа 8.3.27 при контрольном цикле импорта и выгрузки.
