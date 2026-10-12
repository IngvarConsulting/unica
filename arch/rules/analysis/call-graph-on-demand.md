---
id: INV.ANALYSIS.CALL-GRAPH-ON-DEMAND
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::method_view_names_call_graph_branches_without_asking_the_analyzer
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_selected_engine_routes_request_delivery_before_provider_start
  - crates/unica-coder/src/infrastructure/daemon/server.rs::call_graph_branch_answers_working_while_the_analyzer_is_being_delivered
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::the_call_graph_is_not_a_view_section
  - crates/unica-coder/src/infrastructure/daemon/v13_call_graph.rs::the_method_node_names_both_branches_without_a_count
---

# Граф вызовов читается только адресом ветви

Граф вызовов метода читается адресом ветви `<метод>.Caller` или
`<метод>.Callee`. Другого входа нет: секции `callGraph` в `filter.sections`
у `unica.view` нет, и её запрос получает тот же отказ, что любая неизвестная
секция.

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
Установка, которой нечего заказывать, по-прежнему отвечает ветвью
`unavailable`; общий отказ «нет движка» для этого случая задаёт
[#549](https://github.com/IngvarConsulting/unica/issues/549). Если анализатор
есть, а граф ещё строится, ветвь отвечает состоянием `indexing`.
