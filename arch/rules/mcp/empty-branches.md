---
id: CTR.SOURCE.EMPTY-BRANCHES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/empty_owner_branches.rs::unnamed_form_command_and_template_entries_are_not_reported_as_empty
  - crates/unica-coder/src/infrastructure/v13_read/tests/empty_owner_branches.rs::incomplete_registrations_keep_known_rows_and_independent_empty_siblings
  - crates/unica-coder/src/infrastructure/v13_read/tests/empty_owner_branches.rs::modeled_empty_owner_branches_are_advertised_and_readable_in_platform_order
  - crates/unica-coder/src/infrastructure/v13_read/tests/empty_owner_branches.rs::damaged_service_children_keep_owner_facts_without_claiming_empty_branches
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
gap:
  - https://github.com/IngvarConsulting/unica/issues/2223
---

# Корень и объект перечисляют и пустые ветви

`view` корня конфигурации или расширения перечисляет ветвь каждого вида
метаданных, который допускает профиль платформы, в порядке дерева
Конфигуратора. Вид без объектов выдаётся ветвью со счётом 0, поэтому «пусто»
отличается от «вид не поддержан», а пустая ветвь служит местом первого объекта.

Объект верхнего уровня так же перечисляет в порядке дерева Конфигуратора
каждый вид подчинённых объектов, допустимый для его вида, в том числе пустой.

Вида, которого у узла не бывает, среди ветвей нет. Корень внешней обработки
или отчёта перечисляет только своих владельцев.
