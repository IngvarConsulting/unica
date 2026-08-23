---
id: INV.SURFACE.PARITY-HARNESS-COVERAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_mcp_script_parity.py::test_every_in_scope_tool_has_a_parity_scenario
scope: [product, wire]
---

# Каждый инструмент контура parity имеет исполняемый сценарий

Точный набор инструментов, оставшихся в parity-контуре, совпадает с набором
исполняемых сценариев; типизированные инструменты после миграции в этот контур
не подмешиваются и доказываются собственными предметными тестами.
