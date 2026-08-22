---
id: INV.PKG.PACKAGED-PUBLIC-SURFACE
status: active
governs: product
decision: DEC.2026-08-22.EVIDENCE-BOUNDED-PRESERVATION
check: crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_requires_each_lifecycle_to_expose_each_public_tool
scope: [host, pkg, product, wire]
---

# Bootstrap проверяет два MCP lifecycle и три опорных инструмента

Проверка runtime требует успешный legacy `initialize` с последующим
`tools/list`, а также direct-first `server/discover` и `tools/list`. Оба списка
должны содержать `unica.project.status`, `unica.standards.search` и
`unica.standards.explain`. Полный перечень инструментов, определения их
аргументов и результатов и предметное поведение этим правилом не объявляются
проверенными.
