---
id: INV.APP.DEPENDENCY-DIRECTION
check:
  - tests/ci/test_product_contracts.py::ProductContractTests.test_application_layer_does_not_spawn_git_directly
  - tests/ci/test_product_contracts.py::ProductContractTests.test_direct_git_command_guard_matches_calls_before_masking_literals
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_direct_layer_references
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_grouped_layer_references_in_use_trees
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_layer_references_through_root_aliases
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Внутренние слои не зависят от транспорта и инфраструктурных реализаций

В `unica-coder` слой `domain` не ссылается на `application`, `infrastructure`
или `interfaces`. Слой `application` не ссылается на `infrastructure`
или `interfaces`. Инфраструктурный код не обращается к `interfaces`.

Группировка импортов и псевдонимы не меняют направление зависимости.
Граница проверяется по Rust-коду.

Продуктивный `application` получает состояние Git через инфраструктуру
и не запускает процесс Git самостоятельно. Проверка прямого запуска пока
распознаёт полное имя `std::process::Command::new` с литералом `git`;
обход через импорт, псевдоним или переменную остаётся в `gap`.
