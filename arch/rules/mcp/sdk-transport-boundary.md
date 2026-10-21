---
id: INV.WIRE.SDK-TRANSPORT
check:
  - tests/ci/test_product_contracts.py::ProductContractTests.test_rmcp_dependency_is_owned_by_unica_coder_without_macro_features
  - tests/ci/test_product_contracts.py::ProductContractTests.test_rmcp_transport_is_confined_to_mcp_interface
  - tests/ci/test_product_contracts.py::ProductContractTests.test_unica_coder_production_library_satisfies_rmcp_handler_bound
  - tests/ci/test_product_contracts.py::ProductContractTests.test_production_rmcp_handler_assertion_rejects_test_only_or_gated_bound
  - tests/ci/test_ci_rust_lanes.py::ArchiveFixtureBinaryTests.test_build_binds_only_the_existing_ordinary_archive_non_test_executable
---

# MCP SDK принадлежит транспортному слою

Только Cargo-пакет `unica-coder` прямо зависит от `rmcp` из crates.io.
Зависимость не переименовывается, отключает default features и включает
ровно `server` и `transport-io`.

Продуктивные ссылки на `rmcp` во всех отслеживаемых Git Rust-файлах `src`
пакетов workspace находятся только в `crates/unica-coder/src/interfaces/`.
Комментарии, литералы и элементы с точным `#[cfg(test)]` в эту границу
не входят.

Производственная library-сборка подтверждает, что `UnicaServer` реализует
`rmcp::ServerHandler`: соответствие SDK не должно существовать только
в тестовой сборке. Эту сборку выполняет обязательный ordinary-архив Rust,
а не Python-набор.
