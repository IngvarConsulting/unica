---
id: INV.REGISTRY.NO-TRACKER-LINKS
status: active
decision: DEC.2026-08-19.DECISIONS-FORM-INSIDE
check: tests/arch/test_registry.py::test_no_record_points_at_the_tracker
scope: [docs]
---

# Реестр не ссылается на трекер

Ни одна запись не несёт номера задачи и ссылки на неё — ни коротким `#`, ни
полным адресом. Ссылка из задачи на символ реестра остаётся свободной: она
переживает переезд, а обратная — нет.
