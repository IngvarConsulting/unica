---
id: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_objects_without_companion_files_read_their_content_as_inherited
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_companion_file_stays_a_source_error_outside_borrowing
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::borrowed_children_of_a_borrowed_owner_validate_without_payload
  - crates/unica-coder/src/application/v13/check.rs::every_node_kind_owns_its_validators_without_a_caller_choice
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Заимствованный объект без своего содержимого читается по описанию

Платформа 8.3.27 импортирует и выгружает заимствованные расширением роль,
общую форму, форму объекта, макет и пакет XDTO без `Rights.xml`, `Form.xml`,
`Template.xml` и `Package.bin`. Такой объект набора вида `extension`
с `ExtendedConfigurationObject` в описании читается по описанию: `view`
показывает заимствование и `content: "inherited"` без счётчиков и ветвей
содержимого. Содержимое родителя не подставляется и не считается пустым.
Адрес внутри содержимого отвечает `not_found` и указывает на родителя
по `props.extends`. Модуль формы без `Form.xml` платформа не сохраняет,
поэтому ветви модуля у такой формы нет.

`check` такого узла не запускает валидатор отсутствующего файла и отвечает
`readable`. Проверка описания владельца принимает заимствованного ребёнка
без файлов содержимого, только если заимствование доказывают описания
владельца и ребёнка, а каталога содержимого у ребёнка нет.

У объекта конфигурации и у собственного объекта расширения отсутствие файла
остаётся ошибкой источника.
