---
id: INV.ANALYSIS.CALL-GRAPH-ON-DEMAND
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::method_view_names_call_graph_branches_without_asking_the_analyzer
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_selected_engine_routes_request_delivery_before_provider_start
  - crates/unica-coder/src/infrastructure/daemon/server.rs::call_graph_branch_answers_working_while_the_analyzer_is_being_delivered
  - crates/unica-coder/src/infrastructure/daemon/server.rs::a_retired_read_filter_routes_next_to_the_address_that_replaced_it
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_method_node_names_both_branches_without_a_count
  - crates/unica-coder/src/infrastructure/daemon/server.rs::every_route_refuses_a_missing_engine_with_one_typed_refusal
---

# Граф вызовов читается только адресом ветви

Граф вызовов метода читается адресом ветви `<метод>.Caller` или
`<метод>.Callee`. Другого входа нет: фильтра секций у `unica.view` нет,
и вызов с `filter` получает отказ неизвестного аргумента.

Обычное чтение метода не обращается к анализатору. Узел метода BSL-модуля
называет ветви `Caller` и `Callee` в `branches` без `count` и без состояния
графа: их сообщает только ответ ветви. Метод HTTP-сервиса и метод модуля
роли, которую граф не покрывает, этих ветвей не называют. Чтение такой ветви
отказывает по адресу, до обращения к анализатору и заказа его поставки.

Если анализатора на машине нет, а установка может его заказать, чтение ветви
заказывает поставку этим же вызовом, как `check` и `search` с ролью. Пока
поставка идёт, ответ называет её состояние `working` и просит повторить тот
же вызов (`delivery_in_progress`). Провал поставки приходит её
типизированным отказом «нет движка», а не ветвью с состоянием `unavailable`.
Если заказывать нечего или движка нет и после поставки, ветвь отвечает тем же
[отказом «нет движка»](../mcp/missing-engine-refusal.md) с шагом, на котором
движка не оказалось. Если анализатор есть, а граф ещё строится, ветвь
отвечает состоянием `indexing`.
