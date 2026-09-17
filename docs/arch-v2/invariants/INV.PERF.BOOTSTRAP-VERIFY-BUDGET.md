---
id: INV.PERF.BOOTSTRAP-VERIFY-BUDGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_bootstrap_launch_path.py::test_verify_performs_the_handshake_within_a_fixed_budget
scope: [host, pkg, product]
---

# Проверка runtime имеет явный бюджет рукопожатия

Команда `unica-bootstrap verify` вызывает MCP-проверку установленного runtime с
фиксированным числовым бюджетом времени, а обычный запуск её не повторяет.
