---
id: INV.SOURCE.FIND-IDENTITY-ONLY
status: active
governs: product
decision: DEC.2026-09-03.FIND-ADDRESS-PATH-DIRECTORY
check:
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
scope: [app, product, source, wire]
---

# Find — двусторонний словарь адреса и файла

Словарь строится перечислением физической раскладки допущенных actor-owned
корней: каталоги коллекций, их прямые элементы и вложенные формы, макеты и
команды объекта. Запись — квалифицированный адрес, канонический вид,
программное имя и путь к его месту в раскладке: файл дескриптора, а у команды
— её каталог. Синоним читается только у кандидатов ответа.
Перечисление ограничено числом source sets, числом записей и суммарными
байтами фактов и проверяет cancellation и deadline.

`query` принимает программное имя, фрагмент квалифицированного адреса или
путь к файлу или каталогу и разрешается в обе стороны: путь даёт адрес, имя и
адрес дают путь. Каждый кандидат несёт `at`, `kind`, `title`, `path` и `reason`; nearest
использует только имя, адрес или путь.

Модули не читаются: методы, области и прочие символы кода принадлежат
`unica.search`. Реквизиты, табличные части, элементы форм, права ролей и
прочие внутренние узлы в словарь не входят и остаются доступны через `view`
по адресу владельца.

Find не захватывает retained revision lease, не проходит logical tree
типизированным читателем, не выполняет final retained confirmation и не
возвращает `rev`. Несуществующая раскладка не становится записью словаря;
malformed дескриптор кандидата не роняет операцию, а исключает синоним.
Расхождение раскладки обнаруживает ограда `view` или `apply`.
