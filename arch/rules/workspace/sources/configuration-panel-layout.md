---
id: INV.SOURCE.EXPLICIT-PANEL-TOPOLOGY
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/panel_layout.rs::panel_topology_preserves_all_root_order_and_home_page_facts_in_both_readers
  - crates/unica-coder/src/infrastructure/v13_read/tests/panel_layout.rs::panel_observation_distinguishes_missing_empty_unreadable_and_unsupported_documents
  - crates/unica-coder/src/infrastructure/v13_read/tests/panel_layout.rs::deeply_nested_panel_groups_reach_public_view_without_a_depth_quota
  - crates/unica-metadata-designer/src/client_interface/observation.rs::definitions_resolve_by_identity_and_repeated_placements_keep_distinct_instances
---

# Карточка конфигурации сохраняет явную топологию панелей

`Configuration.props.panelLayout` показывает сохранённые области, группы,
размещения панелей и пустые определения из `ClientApplicationInterface.xml`.
Порядок областей и записей сохраняется, включая повторные области и пустые
группы. Отсутствующая область отличается от явно пустой. Идентификатор
размещения не подменяется ссылкой на определение; несколько размещений
одного определения сохраняются, неразмещённые определения также видны.

В каждой области `items` идут в порядке XML. `parent` — индекс родительской
группы от нуля в том же массиве текущего ответа; `null` означает прямой элемент
области. Этот индекс не является постоянным идентификатором или адресом.
Отсутствующий `id` группы не придумывается. Ссылка разрешается только по
идентификаторам `panelDef`; таблица имён стандартных панелей её не заменяет.
Необъявленная цель сохраняется с `resolved: false` и ограничением профиля.

Отсутствующий файл не объявляет раскладку. Корректная пустая раскладка имеет
`state: complete`, ссылка на необъявленную цель — `limited`, неподдержанная структура —
`unsupported` с причинами и без успешно-пустых массивов. Повреждённый XML
или чужой документ остаётся отказом чтения. Данные раскладки не скрывают
остальные свойства конфигурации, порядок подсистем и начальную страницу.

Чтение не вычисляет платформенную раскладку и не расширяет профиль записи
`panels.set`. Прежнее свойство `panels` сохраняет свой смысл там, где этот
профиль способен представить исходный документ.
