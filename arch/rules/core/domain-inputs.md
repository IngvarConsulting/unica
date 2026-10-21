---
id: INV.APP.DOMAIN-INPUTS
check:
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_domain_std_io_in_direct_grouped_and_common_alias_forms
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_rejects_explicit_path_ufcs_and_common_import_aliases
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_allows_business_instance_methods_and_pure_path_operations
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_masks_domain_io_text_in_comments_and_literals
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_moved_semantic_packages_keep_domain_io_and_dependency_guards
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_codecs_and_engines_reach_files_only_through_workspace
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_repository_currently_complies_with_platform_boundary
---

# Домен не импортирует файловые API, окружение и управление процессами

Доменный код `unica-coder`, самостоятельные семантические пакеты и модули
`src/model/` компонентов со сценарием приложения не используют `std::fs`, `std::env`
и `std::process`. Явные файловые вызовы через стандартные типы `Path`
и `PathBuf`, например `Path::exists(path)`, также запрещены.

Чистые операции над путём, например соединение компонентов, допустимы:
они не обращаются к файловой системе. Страж различает их и явно адресованные
файловые вызовы, в том числе через псевдонимы стандартных типов. Определение
типа получателя у вызова вида `path.exists()` эта проверка не выполняет.

Designer-кодеки (`unica-<семейство>-designer`) и пакеты `unica-operations`,
`unica-navigation`, `unica-diagnostics`, `unica-code-intelligence`,
`unica-xml-format`, `unica-work-control` также не используют `std::fs`
и явные файловые вызовы через `Path` и `PathBuf` в `src/`, включая модули
`#[cfg(test)]`. Исходники они читают через порты чтения, которые выдаёт
владелец workspace, а не открывают файлы в обход него. `std::env`
и `std::process` им доступны. Тесты, которым нужна файловая система,
находятся в `tests/` пакета.
