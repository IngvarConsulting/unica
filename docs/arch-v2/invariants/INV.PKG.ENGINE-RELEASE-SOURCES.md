---
id: INV.PKG.ENGINE-RELEASE-SOURCES
status: active
governs: product
decision: DEC.2026-09-02.MAINTAINED-ENGINES-PUBLISH-AT-SOURCE
check: tests/ci/test_product_contracts.py::test_both_sides_of_the_wire_approve_the_same_release_origins
scope: [host, pkg]
---

# Источник релиза движка выбирается по его владению

Bootstrap принимает `v8-runner` из релизов сопровождаемого форка, остальные
движки — из `unica-toolchain`, и не открывает произвольный третий источник.
