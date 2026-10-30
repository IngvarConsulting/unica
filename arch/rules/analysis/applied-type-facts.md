---
id: INV.ANALYSIS.APPLIED-TYPE-FACTS
check:
  - crates/unica-bsl-applied/src/tests.rs::managers_objects_and_references_carry_typed_members_and_their_templates
  - crates/unica-bsl-applied/src/tests.rs::registers_split_dimensions_from_resources_and_partial_facts_mark_members
  - crates/unica-bsl-applied/src/tests.rs::comment_names_resolve_to_declared_objects_only
  - crates/unica-bsl-applied/src/tests.rs::a_model_of_one_object_resolves_comment_names_through_the_declared_objects
  - crates/unica-bsl-applied/src/tests.rs::nested_array_alternatives_preserve_parameter_and_return_types
  - crates/unica-bsl-applied/src/tests.rs::ordinary_arrays_unknown_names_and_prose_keep_their_boundaries
  - crates/unica-bsl-applied/src/tests.rs::method_addresses_and_areas_are_preserved
  - crates/unica-bsl-applied/src/tests.rs::collection_partial_is_the_union_of_each_distinct_object
---

# Прикладная модель сохраняет подтверждённые факты

При построении по подтверждённым фактам одной конфигурации модель сохраняет принадлежность членов объекту и грани, канонические имена, известные типы свойств, параметров и результатов, а также исходные Method.at и нормативные area. Переданы уникальные объекты и члены с каноническими адресами; источник Declared разрешает объявления и за пределами текущего набора objects. Templates предоставляет подтверждённые каталожные факты; соответствие этого источника выбранной платформе — предусловие.

Каждая известная альтернатива типа сохраняется независимо от непредставимой вложенности другой альтернативы. Неизвестные типы не угадываются, а имя чужого объекта из комментария остаётся неразрешённым. Предоставленная ссылка на платформенный шаблон сохраняется, общие члены шаблона не копируются как прикладные.

Признак partial сохраняется у построенных граней объекта и объединяется по OR у платформенного менеджера коллекции для переданных объектов. Он не удостоверяет полноту описи: отсутствующие во входе объекты и готовность источников этой гарантией не устанавливаются.
