---
id: INV.APP.DOMAIN-INPUTS
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_domain_std_io_in_direct_grouped_and_common_alias_forms
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_explicit_path_ufcs_and_common_import_aliases
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_business_instance_methods_and_pure_path_operations
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_masks_domain_io_text_in_comments_and_literals
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
---

# Домен не импортирует файловые API, окружение и управление процессами

Доменный код `unica-coder` не использует `std::fs`, `std::env`
и `std::process`. Явные файловые вызовы через стандартные типы `Path`
и `PathBuf`, например `Path::exists(path)`, также запрещены.

Чистые операции над путём, например соединение компонентов, допустимы:
они не обращаются к файловой системе. Страж различает их и явно адресованные
файловые вызовы, в том числе через псевдонимы стандартных типов. Определение
типа получателя у вызова вида `path.exists()` эта проверка не выполняет.
