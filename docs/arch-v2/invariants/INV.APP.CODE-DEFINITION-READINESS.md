---
id: INV.APP.CODE-DEFINITION-READINESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/rlm_navigation.rs::definition_readiness_matrix_never_reports_false_typed_success
scope: [app, product]
---

# Definition не публикует ложный типизированный успех

`unica.code.definition` возвращает `index_pending` для building/incomplete и
`index_unavailable` для остальных неготовых состояний RLM. Типизированный
успех возможен только при готовом текущем индексе; пустой готовый ответ означает
доказанное отсутствие совпадений.
