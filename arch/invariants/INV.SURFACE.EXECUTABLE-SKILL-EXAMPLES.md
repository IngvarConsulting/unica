---
id: INV.SURFACE.EXECUTABLE-SKILL-EXAMPLES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_mcp_script_parity.py::test_every_skill_tools_call_example_executes_by_tool_mode
scope: [wire]
---

# Примеры вызовов скиллов исполняются по своему режиму

Каждый JSON-пример `tools/call` из поставляемых скиллов исполняется на
детерминированной фикстуре как чтение либо как предпросмотр мутации.
