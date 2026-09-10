---
id: INV.WIRE.COMMAND-INTERFACE-WRITE
status: active
governs: product
decision: DEC.2026-09-09.COMMAND-INTERFACE-WRITE-FAMILY
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::command_visibility_edits_common_and_leaves_role_values_alone
scope: [wire, product]
---

# Операция интерфейса правит свою секцию и ничью больше

Каждая из пяти операций семейства меняет ровно одну секцию
`CommandInterface.xml`. Соседние секции и соседние элементы внутри секции
остаются нетронутыми.

Правка видимости касается только общего значения. Значения по ролям читатель
не показывает, и писатель обязан их сохранить: невидимое не значит
несуществующее, а тихая потеря обнаружится только у пользователя.

Порядок задаётся полной последовательностью. Частичный список отвергается:
порядок — отношение между всеми элементами.
