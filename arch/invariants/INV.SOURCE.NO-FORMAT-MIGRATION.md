---
id: INV.SOURCE.NO-FORMAT-MIGRATION
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check: crates/unica-coder/src/infrastructure/format_guard.rs::native_mutation_surface_and_format_refusal_are_exact
scope: [source]
---

# Нативная поверхность не мигрирует формат

Точные имена операций и отпечатки полных схем всех 25 публичных нативных и
типизированных XML-мутаторов замкнуты. Запись старого и нового профиля
отклоняется с сохранением исходных байтов; отдельного пути миграции нет.
