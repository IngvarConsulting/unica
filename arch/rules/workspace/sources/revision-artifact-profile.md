---
id: CTR.SOURCE.REVISION-ARTIFACT-PROFILE
check:
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::active_platform_actor_cannot_select_the_legacy_revision_corpus
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_service_construction_retains_the_validated_root_across_substitution
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_external_resource_drift_rotates_subsequent_admission
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_ignores_huge_unrelated_binary_while_bounding_targeted_resource
---

# Ревизия учитывает ресурсы выбранного профиля исходников

Для актора Platform XML `8.3.27` / `2.20` ревизия учитывает содержимое
ресурсов XDTO, поддержки, макетов, справки и элементов форм наряду
с исходными текстовыми форматами. Профиль берётся из привязки актора
к удерживаемому корню; выбрать вместо него прежний профиль нельзя.
Отдельный режим совместимости v0.12 сохраняет прежний состав файлов.

Ресурс должен лежать в предусмотренном месте у допустимого владельца.
Для ресурсов отдельных объектов конфигурации и расширения путь начинается
с известной коллекции и непосредственного владельца; у внешней обработки или отчёта —
с непосредственного владельца. Произвольный префикс и смешанная цепочка
`Forms` / `Templates` не превращают посторонний файл в ресурс.

Изменение содержимого такого ресурса меняет ревизию. Поставки
`Ext/ParentConfigurations/*.cf` учитываются по пути и наличию;
изменение их байтов само по себе ревизию не меняет. Посторонние бинарные
файлы не входят в список содержимого ревизии и не расходуют бюджет его байтов.
