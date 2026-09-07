---
id: DEC.2026-09-03.FIND-ADDRESS-PATH-DIRECTORY
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_name_resolves_to_the_address_and_the_file_that_carries_it
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_path_resolves_back_to_its_object_address
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_synonym_resolves_to_its_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_holds_objects_and_never_code_symbols_or_inner_nodes
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_refuses_to_grow_past_its_entry_bound
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_cancellation
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_its_operation_deadline
  - crates/unica-coder/src/infrastructure/v13_find.rs::an_external_root_publishes_its_owner_and_never_the_dump_sidecar
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_that_is_not_an_owner_descriptor_never_becomes_an_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_descriptor_whose_attributes_start_on_a_new_line_is_still_an_object
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.FIND-IDENTITY-ONLY, INV.SOURCE.LOGICAL-READER-PARITY, CTR.WIRE.FIND-CANDIDATE-SHAPE]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-03-find-is-an-address-path-directory-design.md
---

# Find — двусторонний словарь адреса и файла, а не снимок дерева

**Решение.** `find` отвечает на два вопроса: какому объекту принадлежит файл
и каким файлом представлен объект. Словарь строится перечислением физической
раскладки допущенных корней: каталоги коллекций, их прямые элементы и
вложенные формы, макеты и команды. Запись — квалифицированный адрес,
канонический вид, программное имя и путь к его месту в раскладке: файл
дескриптора, а у команды — её каталог, потому что отдельного дескриптора
команда не имеет. Синоним читается только у кандидатов ответа. Аргумент `query` принимает имя, фрагмент адреса или путь к
файлу или каталогу и разрешается в обе стороны; ответ несёт путь рядом с
адресом.

Модули не разбираются: методы, области и прочие символы кода принадлежат
`unica.search`. Реквизиты, элементы форм, права ролей и прочие внутренние
узлы в словарь не входят — они доступны через `unica.view` по адресу объекта.

`find` не берёт аренду точной ревизии, не проходит дерево полностью, не
выполняет финального подтверждения и не возвращает `rev`: словарь соответствий
не является снимком состояния. Расхождение раскладки обнаруживает ограда
`view` или `apply`, а не `find`.

**Почему.** На дампе вендорской конфигурации прежний обход складывал свыше
полумиллиона узлов при собственном пределе 65 536 и платил 26–43 секунды за
аренду ревизии на каждый вызов, тогда как назначению инструмента отвечают
тринадцать тысяч объектов, читаемых за доли секунды. Путь к файлу, ради
которого инструмент и нужен, наружу не отдавался вовсе.

**Цена.** Поиск метода или области по имени пропадает до появления
символического режима в `unica.search`. Ответ `find` больше не содержит
ревизию.
