---
id: INV.WIRE.SURFACE-PROSE-FOLLOWS-THE-DICTIONARY
status: active
governs: product
decision: DEC.2026-09-10.PUBLISHED-SURFACE-COUNTS-ITS-DICTIONARY
check: tests/ci/test_tool_surface_ledger.py::test_the_published_run_prose_counts_the_dictionary_it_describes
scope: [wire, product]
---

# Проза опубликованной поверхности не называет операции вне словаря

Обещание, план и сценарии `unica.run` называют только операции из реестра
покрытия, а счёт нереализованных операций в плане совпадает с этим реестром.
Имя операции в опубликованной поверхности — адрес вызова, и адреса, которого
нет, она не печатает.
