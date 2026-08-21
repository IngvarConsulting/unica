---
id: INV.SURFACE.DCS-NAMING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_dcs_naming_contract.py::test_provenance_names_local_dcs_contract_but_preserves_donor_paths
scope: [wire, pkg]
---

# Provenance отделяет локальный DCS-контракт от донорских путей

Записи происхождения называют локальные DCS-скиллы и контракты действующими
именами, сохраняя оригинальные пути донорского корпуса отдельно.
