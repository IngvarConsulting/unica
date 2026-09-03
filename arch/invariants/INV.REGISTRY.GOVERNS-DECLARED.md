---
id: INV.REGISTRY.GOVERNS-DECLARED
status: active
governs: process
decision: DEC.2026-08-19.PRODUCT-OR-PROCESS
check: tests/arch/test_registry.py::test_governs_is_known
scope: [docs]
---

# Сторона записи объявлена и известна

Каждая запись несёт сторону из словаря: `product` — нарушение заметит
потребитель, `process` — только мы. Значение вне словаря и отсутствие поля
одинаково недопустимы.
