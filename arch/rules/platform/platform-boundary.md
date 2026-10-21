---
id: INV.PLATFORM.OS-BEHIND-FACADE
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_platform_constructs_outside_facade_with_stable_lines
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_platform_constructs_only_in_facades_and_nested_platform_tests
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_lifetimes_labels_and_chars_do_not_hide_code
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_component_platform_facades_have_closed_owners_and_no_path_exemptions
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_platform_implementation_module_is_private_even_for_an_authorized_owner
---

# Выбор кода под операционную систему сосредоточен в отдельных каталогах

Условия компиляции Rust, выбирающие код под платформу (`cfg`, `cfg_attr`),
и обращения к Windows API через `windows_sys` разрешены только в приватных
платформенных модулях закрытого набора владельцев:

- `crates/unica-coder/src/infrastructure/platform/`;
- `crates/unica-bootstrap/src/platform/`;
- `crates/unica-process/src/platform/` — процессы;
- `crates/unica-workspace/src/platform/` — доступ к ресурсам и публикация;
- `crates/unica-artifact-delivery/src/platform/` — установка артефактов;
- `crates/*/tests/platform/`.

Другой компонент не получает это право созданием каталога `platform`.
Фасад экспортирует необходимые операции владельца; сам платформенный модуль
не становится публичным API. Исключения для отдельных файлов не допускаются.

Например, ветку `#[cfg(windows)]` нельзя добавить в обычный обработчик
инструмента: код для Windows должен находиться в одном из этих каталогов.
Проверка ищет такие конструкции в исходном коде и не учитывает их упоминания
в строках и комментариях.
