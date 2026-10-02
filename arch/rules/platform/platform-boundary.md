---
id: INV.PLATFORM.OS-BEHIND-FACADE
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_platform_constructs_outside_facade_with_stable_lines
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_platform_constructs_only_in_facades_and_nested_platform_tests
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_lifetimes_labels_and_chars_do_not_hide_code
---

# Выбор кода под операционную систему сосредоточен в отдельных каталогах

Условия компиляции Rust, выбирающие код под платформу (`cfg`, `cfg_attr`),
и обращения к Windows API через `windows_sys` разрешены только в каталогах:

- `crates/unica-coder/src/infrastructure/platform/`;
- `crates/unica-bootstrap/src/platform/`;
- `crates/*/tests/platform/`.

Например, ветку `#[cfg(windows)]` нельзя добавить в обычный обработчик
инструмента: код для Windows должен находиться в одном из этих каталогов.
Проверка ищет такие конструкции в исходном коде и не учитывает их упоминания
в строках и комментариях.
