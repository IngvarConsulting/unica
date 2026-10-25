---
id: INV.APP.APPLY-ANSWER-SIZE
check:
  - crates/unica-coder/src/infrastructure/daemon/server/apply_answer_size_tests.rs::apply_answer_does_not_grow_with_the_number_of_operations
  - crates/unica-coder/src/infrastructure/daemon/server/apply_answer_size_tests.rs::apply_answer_names_only_the_operation_that_warns
  - crates/unica-source-contract/src/proposal/planning.rs::reconciled_effects_name_only_the_operations_that_warned
---

# Ответ apply не повторяет запрос

Предпросмотр и исполнение `apply` не перечисляют операции запроса. Ответ
сообщает их число и называет только операции, к которым относится
предупреждение. Поэтому объём ответа задают изменения, предупреждения
и сведения плана, а не число операций запроса.
