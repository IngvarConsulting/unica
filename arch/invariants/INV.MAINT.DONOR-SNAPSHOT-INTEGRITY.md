---
id: INV.MAINT.DONOR-SNAPSHOT-INTEGRITY
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_mcp_script_parity.py::test_donor_inventory_relations_preview_and_snapshot_are_closed
scope: [ci]
---

# Донорский снимок связан с проверенным происхождением

Паритетный набор проверяет байты донорского снимка, его дайджест и запись о
происхождении до сравнения с поведением Unica.
