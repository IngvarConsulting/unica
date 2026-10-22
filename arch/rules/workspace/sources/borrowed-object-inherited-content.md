---
id: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/form.rs::inherited_form_metadata_is_local_without_claiming_a_local_form_payload
  - crates/unica-coder/src/infrastructure/v13_read/tests/form.rs::borrowed_form_with_its_own_form_xml_reads_as_local_content
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::borrowed_objects_without_companion_files_read_their_content_as_inherited
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_companion_file_stays_a_source_error_outside_borrowing
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::borrowed_children_of_a_borrowed_owner_validate_without_payload
  - crates/unica-coder/src/application/v13/check.rs::every_node_kind_owns_its_validators_without_a_caller_choice
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/inherited_content_tests.rs::writes_into_inherited_content_refuse_with_the_way_to_the_parent
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/inherited_content_tests.rs::inherited_refusal_needs_a_borrowed_object_without_its_file
  - crates/unica-coder/src/domain/apply.rs::content_writers_name_the_companion_file_they_write
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::inherited_node_can_hides_content_writers
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unknown_facts_of_borrowed_objects_are_named_as_node_limits
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Заимствованный объект без своего содержимого читается по описанию

Платформа 8.3.27 импортирует и выгружает заимствованные расширением роль,
общую форму, форму объекта, макет объекта и пакет XDTO без `Rights.xml`,
`Form.xml`, `Template.xml` и `Package.bin`. Содержимое такого объекта
унаследовано от родителя, если объект в наборе вида `extension` указывает
родителя через `ExtendedConfigurationObject`, а у формы или макета владельца
родителя указывает и описание владельца.

`view` читает такой объект по описанию: заимствование и
`content: "inherited"` без счётчиков и ветвей содержимого. Форма или
макет владельца не разрешают родителя сами: `parentReason` называет объект
расширения, чей `props.extends` указывает родителя. Содержимое
родителя не подставляется и не считается пустым. Вид макета равен `null`:
без `TemplateType` платформа записывает умолчание, а не вид родителя.
Адрес внутри содержимого, в том числе модуль такой формы, который платформа
без `Form.xml` не хранит, отвечает `not_found` и называет объект расширения,
чей `props.extends` указывает родителя.

У формы отдельный адрес `Metadata` читает локальный дескриптор и доступен
из корня формы без файлов содержимого. Эта ветвь декларации не обещает
локальных элементов, реквизитов, команд или модуля формы.

`apply` не пишет в унаследованное содержимое: права, элементы и модуль
формы, тело макета, типы пакета. Отказ `not_found` называет файл и ведёт
`next` к объекту расширения; словарь `can` такого узла эти операции
не предлагает. Описание объекта пишется как прежде.

`check` такого узла не запускает валидатор отсутствующего файла и отвечает
`readable`. Проверка описания владельца принимает заимствованного ребёнка
без файлов содержимого, только если корень набора — расширение,
заимствование доказывают описания владельца и ребёнка, а каталога
содержимого у ребёнка нет.

У объекта конфигурации, у собственного объекта расширения и у ребёнка
собственного владельца отсутствие файла остаётся ошибкой источника.

Отсутствие свойств в самом описании такого объекта читается по
[incomplete-metadata-read.md](incomplete-metadata-read.md).
