---
id: INV.RLM.READ-CANCELLATION
check:
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::adapter_normalizes_client_cancellation_from_readiness_and_call
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::cancellation_after_readiness_wins_when_the_deadline_expires_at_the_same_time
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::cancellation_after_readiness_wins_before_any_readiness_state_is_interpreted
---

# Отмена поиска определения не превращается в ошибку индекса

Адаптер поиска определения через RLM возвращает отмену, если клиент отменён
при проверке готовности индекса или при чтении. Отмена, обнаруженная после
проверки готовности, имеет приоритет над сообщением о неготовом индексе
и одновременно истёкшим сроком выполнения.

Проверки охватывают результат адаптера, а не принудительное завершение
внешнего процесса.
