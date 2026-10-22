---
id: INV.SOURCE.XDTO-PROPERTY-PATH
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_property_chain_comes_from_the_address
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_args_are_closed_typed_and_report_exact_paths
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::xdto_properties_are_read_at_any_depth
  - crates/unica-coder/src/domain/apply.rs::retired_names_with_a_dot_route_to_escaped_addresses
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_rejects_every_invalid_property_path_shape
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_addresses_a_dotted_property_identity_with_an_escape
  - crates/unica-xdto-designer/src/writer.rs::xdto_writer_separates_only_unescaped_property_path_dots
---

# Вложенное свойство XDTO задаётся адресом

Свойство любой вложенности адресуется цепочкой `Property` после типа:
`…Type.T.Property.A.Property.B`. Каждый переход проходит через вложенное
определение типа `typeDef` свойства; чтение и `unica.apply` понимают одну
и ту же цепочку. Точка в имени свойства — часть имени и пишется `\.` по
[грамматике логического адреса](../../mcp/qualified-logical-address.md):
`…Property.A\.B.Property.Child` выбирает свойство `Child` внутри `A.B`.

`property.add` адресует тип или свойство, во вложенный тип которого
добавляется новое; `property.remove` — само удаляемое свойство. Аргументов
`propertyPath` и `name` у этих операций нет: вызов с ними отклоняется
`bad_value`; `next` ведёт к адресу свойства, а если путь назван отдельно
от имени удаляемого свойства — к коллекции свойств по этому пути. Внутренний писатель получает
тот же путь, где `\.` — буквальная точка, а пустые сегменты и иное
экранирование отклоняются.
