---
id: INV.APP.CODE-DEFINITION-READINESS
check:
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::definition_readiness_matrix_never_reports_false_typed_success
---

# Недоступность индекса не означает, что определений нет

Если пригодного индекса нет, адаптер поиска определения через RLM возвращает
отказ без предметных данных, текстового результата и фиктивных артефактов.
Он не подменяет недоступность успешным пустым списком определений.

В тесте эту гарантию проверяют случаи `Missing`, `Failed` и `Unavailable`.
Его прежние ветви `Stale`, `Building` и `Incomplete` не закрепляются этим
правилом: доступ к пригодному старому индексу определяется
[правилом чтения устаревших данных](rlm-stale-results.md).
