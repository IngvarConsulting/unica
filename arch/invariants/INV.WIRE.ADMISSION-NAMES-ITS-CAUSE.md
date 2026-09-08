---
id: INV.WIRE.ADMISSION-NAMES-ITS-CAUSE
status: active
governs: product
decision: DEC.2026-09-09.TYPED-ADMISSION-REFUSAL
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_admission_names_why_no_source_set_was_admitted
scope: [wire, source]
---

# Отказ допуска называет причину и продолжение

Вызов, которому нужен допущенный набор PlatformXml, не отвечает недопуском как
таковым. Отказ несёт `diagnostics[0]` с кодом закрытого словаря и `next`, ведущий
в маршруты, отвечающие до допуска.

Незаведённая рабочая область и битый `v8project.yaml` отвечают `invalid_state`,
нечитаемый или чужеформатный набор — `invalid_source`, сорванный обход —
`provider_unavailable`, истёкший срок допуска — `deadline_exceeded`. Битую
настройку допуск называет тем же предложением, что и `unica.view {}`: причина
одна, и разойтись словам нельзя.
