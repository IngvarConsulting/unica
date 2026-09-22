---
id: INV.RUNTIME.RUNNER-ONE-CAPABILITIES
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check: crates/unica-coder/src/infrastructure/daemon/v13_run_dictionary.rs::runner_one_refuses_unsupported_semantics_and_old_names_before_admission
scope: [wire, app, product]
---

# Недоказанная семантика раннера недоступна до запуска

Словарь разделяет поддержанную опубликованную схему, ограниченное подмножество
и недоступную операцию через `support`. На адаптере 0.11 `push` допускает
только удаление расширения; отправка, `pull`, `upload`, `apply`, `reset` и создание
базы с новой семантикой отклоняются до source admission и запуска процесса.
`ifRev` не является подтверждением неизменности поколения базы.
