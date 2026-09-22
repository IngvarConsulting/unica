---
id: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-COMPATIBLE-DEVELOPMENT-CYCLE
check: crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::runner_one_refuses_unsupported_semantics_and_old_names_before_admission
scope: [wire, app, product]
---

# Недоказанная семантика раннера недоступна до запуска

Словарь разделяет полную поддержку опубликованной схемы и ограниченное
подмножество через `support`. Все 13 операций исполнимы на адаптере 0.11.1;
шесть операций разработки имеют `limited`, точную схему `supportedArgs`
и описание отсутствующих гарантий. Неподдержанные аргументы отклоняются
типизированным обработчиком до запуска. Старые имена не становятся алиасами.
`ifRev` не является подтверждением неизменности поколения базы.
