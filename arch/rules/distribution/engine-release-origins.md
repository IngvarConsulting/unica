---
id: INV.PKG.ENGINE-RELEASE-SOURCES
check:
  - crates/unica-bootstrap/tests/manifest_contract.rs::a_maintained_engine_source_is_approved_without_opening_other_origins
  - crates/unica-bootstrap/tests/manifest_contract.rs::an_engine_is_accepted_from_the_toolchain_release
  - crates/unica-bootstrap/tests/manifest_contract.rs::an_engine_from_an_unapproved_origin_is_refused
  - crates/unica-bootstrap/tests/manifest_contract.rs::an_engine_may_not_borrow_the_core_origin
  - tests/ci/test_product_contracts.py::ProductContractTests.test_both_sides_of_the_wire_approve_the_same_release_origins
---

# Источник загрузки закреплён за конкретным движком

Bootstrap принимает поставки `v8-runner` только из релизов GitHub-репозитория
`IngvarConsulting/v8-runner-rust`. Остальные движки должны приходить из
`IngvarConsulting/unica-toolchain`.

Разрешение связано с именем артефакта: другой движок не может использовать
репозиторий `v8-runner`, а `v8-runner` — общий тулчейн. Другие источники
в манифесте отклоняются.
