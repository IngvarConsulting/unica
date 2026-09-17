---
id: INV.PKG.THIN-PACKAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_generated_marketplace_is_thin_pinned_and_target_neutral
scope: [pkg]
---

# Публичный пакет тонкий

В поставку не уезжают собранные бинарники, внутренние материалы сопровождения и отладочная
упаковка: пакет несёт то, что нужно потребителю, и ничего сверх.
