---
id: CTR.SOURCE.REVISION-ARTIFACT-PROFILE
check:
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_unknown_staged_artifact_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_lookalike_resource_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::same_name_root_changed_format_or_platform_profile_rotates_actor
---

# Запланированные изменения соответствуют профилю исходников

`apply` проверяет изменяемые файлы по профилю источника, допущенному актором.
Для Platform XML `8.3.27` / `2.20` допустимые ресурсы XDTO, поддержки,
макетов, справки и элементов форм определяются вместе с текстовыми
форматами. Произвольный бинарный файл нельзя опубликовать как ресурс
этого профиля.

Ресурс должен лежать в предусмотренном месте у допустимого владельца.
Для объектов конфигурации и расширения путь начинается с известной
коллекции и непосредственного владельца; у внешней обработки или отчёта —
с непосредственного владельца. Произвольный префикс и смешанная цепочка
`Forms` / `Templates` не превращают посторонний файл в ресурс.

Изменение формата или профиля источника меняет привязку актора.
Классификация проверяет запланированные изменения и не требует обхода
всех ресурсов ради общей ревизии. Прочитанные входы и ожидаемые результаты
связываются с конкретным планом предпросмотра.
