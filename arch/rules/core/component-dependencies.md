---
id: INV.APP.COMPONENT-DEPENDENCIES
check:
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_real_workspace_retains_component_direction
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_owner_edges_reject_hidden_concrete_dependencies
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_target_specific_alias_cannot_hide_a_composition_backedge
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_workspace_inherited_parser_alias_keeps_the_upstream_boundary
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_workspace_owner_depends_only_on_a_closed_list
  - tests/ci/test_component_boundaries.py::ComponentBoundaryTests.test_cycles_through_the_workspace_owner_are_rejected_for_every_edge_kind
---

# Компоненты не зависят от продуктовой сборки и реализаций своих портов

Предметные семейства используют общие контракты и явно названные семантические
зависимости. Они не импортируют workspace, форматные кодеки или движки.
Общий реестр операций также не импортирует реализации семейств.

Семейный Designer-кодек использует контракты чтения и предложения изменений,
своё семейство и общий layout Designer. Полномочия публикации принадлежат
workspace; сам workspace не зависит от его потребителей и кодеков. Его
зависимости — закрытый список общих контрактов, процессы допускаются только
в тестах, и ни один вид зависимости не замыкает цикл через workspace.
Исполнение вызывает порты движков и не импортирует конкретный адаптер.
Daemon владеет транспортом и идентичностью процесса, использует execution,
workspace, process, bootstrap и artifact delivery через их контракты.
Предметная маршрутизация остаётся в продуктовой composition. Execution
не импортирует Daemon или bootstrap; состояния попытки и её исхода принадлежат
execution, а не транспортному слою.

ProjectHealth наблюдает исходники и ресурсы через source contracts, общий
Designer layout, workspace и process. Он не импортирует исполнение,
предметные семейства или их кодеки и движки; связывание с исполнением
остаётся в продуктовой composition.

Ни компонент, ни его unit target не зависят от `unica-coder`. Межкомпонентные
проверки, которым нужна продуктовая композиция, принадлежат её integration
target. Ограничение действует и на build/dev и платформенные зависимости:
псевдоним зависимости не меняет её владельца.

MCP SDK принадлежит `unica-coder`. Внешние Rust-типы parser/syntax скрывает
`unica-bsl-parser`; потребители используют его нормализованные факты.
