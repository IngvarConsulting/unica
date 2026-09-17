---
id: INV.SOURCE.WRITABLE-PROFILE
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check: crates/unica-coder/src/infrastructure/format_guard.rs::single_writable_platform_xml_profile_is_exact
scope: [source]
---

# Записывается только действующий профиль выгрузки

Нативная операция пишет platform XML только в профиль платформы `8.3.27`,
формата `2.20`: точный профиль допускается, а более старый отклоняется без
изменения байтов и с указанием на явную повторную выгрузку платформой.
