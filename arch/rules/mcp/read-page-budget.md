---
id: INV.WIRE.READ-PAGE-BUDGET
check:
  - crates/unica-coder/src/application/v13/view.rs::addressed_collection_prefers_64_kib_pages_and_replays
  - crates/unica-coder/src/application/v13/view.rs::one_item_over_the_page_target_is_returned_whole
  - crates/unica-coder/src/application/v13/view.rs::snapshot_over_byte_quota_refuses_before_promising_a_continuation
  - crates/unica-coder/src/application/v13/view.rs::a_late_oversized_item_refuses_before_the_first_page
  - crates/unica-coder/src/application/v13/view.rs::supplied_graph_collection_uses_stable_pages_bound_to_its_owner
---

# Страница чтения сообщает полноту и сохраняет доступ к элементам

Для страничных ответов канонической поверхности `limit` по умолчанию равен
20, максимум — 50. Ответ сообщает, завершена ли выдача полученных элементов
и почему страница остановилась: по числу или объёму. Следующие доступные
элементы получают применимое продолжение; содержимое элемента не обрезается.
Конец полученного окна ранжирующего поставщика отличается от конца всего
корпуса: неполнота сообщается отдельно, курсор за пределы окна не обещается.
Буквальный поиск роли `lexical` окна не имеет и продолжается до конца обхода.

Для адресной коллекции `view` 64 КиБ — целевой размер страницы. Неделимый
элемент или метаданные узла возвращаются целиком, даже если страница больше
цели. Если сохранённый результат или неделимый элемент не проходят
применимую техническую границу, отказ приходит до первого курсора, включая
случай большого элемента в конце коллекции. Основание ограничений задаёт
[правило ресурсов](../runtime/resource-limit-evidence.md).

Сохранённый ответ и точный повтор `view` подчиняются
[правилу курсора](view-cursor-binding.md) и
[хранения снимка](view-cursor-capacity.md). Отдельные гарантии определяют
[вердикт check](check-diagnostic-pages.md),
[живой текстовый поиск и поиск имён](search-corpus-schema.md),
[окно поставщика кода](../analysis/search-result-evidence.md),
[продолжение буквального поиска](../analysis/lexical-search-continuation.md),
[поиск справки](../analysis/documentation/search-pages.md) и
[страницы документа](../analysis/documentation/document-text-pages.md).
