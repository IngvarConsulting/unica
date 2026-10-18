---
id: INV.WIRE.RESOLVED-SOURCE-LOCATION
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_bsl_nodes_across_module_roles
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_method_spans_annotations_through_its_closing_line
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_module_body_numbers_lines_like_view
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_tree_children_use_their_containing_xml
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_module_read_failures_are_never_successful_locations
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::resolve_exact_cancelled_read_never_claims_a_location
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::view_nested_regions_never_substitutes_the_final_name
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
---

# Resolve называет существующий ресурс и его действительный диапазон

При успешном ответе предмет существует; результат называет его адрес, вид и
место в исходниках без `rev`. Отсутствие даёт `not_found`, ошибка чтения или
отмена — отказ. `lines.state` всегда присутствует: `range` с `from` и `to`
либо `notLineBased`. Для древовидного ресурса диапазон не выдумывается.

Метод или область BSL указывает на файл модуля, а не дескриптор владельца.
Метод начинается с первой аннотации, директивы компиляции или `Асинх`,
без них — с объявления, и заканчивается строкой `КонецПроцедуры` или
`КонецФункции` включительно. У пустого метода также есть диапазон.
`Method.<имя>.Body` включает прикреплённый документирующий комментарий,
если он есть, иначе начинается с начала метода. Диапазон совпадает с текстом
`view` по [контракту Body](../workspace/sources/module-compilation-and-body.md).
Область включает `#Область` и `#КонецОбласти`; `Body` модуля — все строки файла.
Вложенная область выбирается полным путём, не только последним именем.
