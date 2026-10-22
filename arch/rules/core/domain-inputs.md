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
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_file_io_free_sources_refuse_third_party_file_crates
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_file_io_free_manifests_refuse_file_crate_dependencies
  - tests/ci/test_rust_platform_boundary.py::RustPlatformBoundaryTests.test_file_io_free_package_list_follows_the_source_boundary
  - tests/ci/test_file_io_methods.py::FileIoMethodsTests.test_probe_marks_every_disallowed_method_once
  - tests/ci/test_file_io_methods.py::FileIoMethodsTests.test_probe_must_have_every_marked_call_refused
  - tests/ci/test_file_io_methods.py::FileIoMethodsTests.test_packages_report_only_their_own_src_once
---

# Домен не импортирует файловые API, окружение и управление процессами

Доменный код `unica-coder`, самостоятельные семантические пакеты и модули
`src/model/` компонентов со сценарием приложения не используют `std::fs`, `std::env`
и `std::process`. Явные файловые вызовы через стандартные типы `Path`
и `PathBuf`, например `Path::exists(path)`, также запрещены.

Чистые операции над путём, например соединение компонентов, допустимы:
они не обращаются к файловой системе. Текстовый страж различает их и явно
адресованные файловые вызовы, в том числе через псевдонимы стандартных типов.
Вызов методом вида `path.exists()` требует определения типа получателя:
его ловит отдельный проход Clippy `disallowed_methods`
(`scripts/ci/check-file-io-methods.py`). Он проходит `src/` самостоятельных
пакетов с этим ограничением, включая `#[cfg(test)]` и код под признаками. Модули внутри
`unica-coder` и `src/model/` других компонентов этот проход не охватывает:
для них действует текстовый страж. Перед проверкой пакетов проход
доказывает каждый запрет на пробном крейте.

Designer-кодеки (`unica-<семейство>-designer`) и пакеты `unica-operations`,
`unica-navigation`, `unica-diagnostics`, `unica-code-intelligence`,
`unica-xml-format`, `unica-work-control` также не используют `std::fs`
и явные файловые вызовы через `Path` и `PathBuf` в `src/`, включая модули
`#[cfg(test)]`. Исходники они читают через порты чтения, которые выдаёт
владелец workspace, а не открывают файлы в обход него. `std::env`
и `std::process` им доступны. Тесты, которым нужна файловая система,
находятся в `tests/` пакета.

Сторонние файловые крейты (`walkdir`, `fs-err`, `tempfile`, `tokio`
с признаком `fs` и подобные) в этих пакетах тоже запрещены. Обычная или
build-зависимость на них отвергается по манифесту, в том числе когда
признак `fs` открывает `[features]`. Путь к ним в `src/` отвергается даже
через dev-зависимость, а переименованная dev-зависимость на такой крейт
отвергается по манифесту. Dev-зависимости остаются доступны тестам в `tests/`.
