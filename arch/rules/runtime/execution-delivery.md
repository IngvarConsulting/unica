---
id: INV.APP.EXECUTION-DELIVERY
check:
  - crates/unica-coder/src/application/execution/recovery_acceptance_tests.rs::acceptance_pending_ack_and_expose_follow_commit_order_without_resurrection
  - crates/unica-coder/src/application/execution/tests.rs::publication_requires_committed_outcome_exact_key_and_live_direct_right
  - crates/unica-coder/src/application/execution/acceptance_tests.rs::acceptance_primary_ack_does_not_drop_an_active_alias_or_factual_result
  - crates/unica-coder/src/application/execution/acceptance_tests.rs::acceptance_both_ack_expose_orders_obey_confirmed_public_promises
  - crates/unica-coder/src/application/execution/acceptance_tests.rs::acceptance_observer_and_wrong_owner_cannot_release_primary_payload
  - crates/unica-coder/src/application/execution/acceptance_tests.rs::acceptance_delivery_choice_does_not_ack_a_late_direct_or_lose_built_direct
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Принятое право получить результат переживает перезапуск

Принятый вызов доступен по заранее выделенному TaskId в своей области
сборки, в том числе до привязки к рабочему пространству. Право восстановления
сохраняется при потере ответа Submit или Recover. Наблюдение по TaskId не
даёт полномочия подтвердить чужой прямой ответ.

Владелец Submit обладает отдельной capability, которая не входит в ключ
вызова и не выдаётся наблюдателям. Точный Submit с другой capability
отклоняется без изменения состояния. ACK проверяет полный ключ, capability
владельца и хеш его конечного результата. Право публичного Task надёжно
фиксируется до выдачи обещания и не снимается ACK владельца Direct. После разрешённого
сжатия новая выдача Task не воскрешает payload.

Frontend необратимо выбирает подготовленный Direct либо выданный Task.
Только подготовленный Direct разрешает ACK; ошибка ACK не отнимает готовый
ответ. После выбора Task поздний Direct не подтверждается. ACK означает
подготовку ответа frontend, а не доставку конечному хосту.

У общего чтения результат producer и конечный вид первоначального держателя
различаются. Если личная отмена победила до завершения, она остаётся тем же
результатом после перезапуска, даже когда producer успешно завершился для
других держателей. Такой override не применяется к мутациям. Временные aliases
после перезапуска неизвестны; это не отменяет долговечные права первоначального
вызова и публичного Task.

Это принятый в #1743 усиленный контракт. Чистая модель не доказывает durable
storage, private protocol или native/compat проекции: их приёмка остаётся
в `gap` до подключения продуктивных маршрутов.
