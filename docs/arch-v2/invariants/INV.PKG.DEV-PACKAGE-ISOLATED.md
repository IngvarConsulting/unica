---
id: INV.PKG.DEV-PACKAGE-ISOLATED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_local_debug_mode_remains_current_host_only_and_uses_unica_dev
scope: [pkg, product]
---

# Отладочный пакет отделён от публичной поставки

Локальная упаковка собирает только текущую цель, запускает её бинарник напрямую
и регистрируется в каталоге Codex под именем `unica-dev`.
