---
id: INV.PLATFORM.OS-BEHIND-FACADE
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_platform_constructs_outside_facade_with_stable_lines
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_platform_constructs_only_in_facades_and_nested_platform_tests
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_lifetimes_labels_and_chars_do_not_hide_code
---

# Условная компиляция под ОС остаётся в платформенных адаптерах

Платформенные условия Rust `cfg`/`cfg_attr` и обращения к `windows_sys` допустимы в
`crates/unica-coder/src/infrastructure/platform/`,
`crates/unica-bootstrap/src/platform/` и `crates/*/tests/platform/`.
За этими границами такие конструкции запрещены.

Проверка лексическая: распознаёт конструкции, отделяя их от строк и комментариев.
