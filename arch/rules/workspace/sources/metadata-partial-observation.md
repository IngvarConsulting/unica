---
id: INV.APP.META-OBSERVATION
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_foreign_form_file_keeps_catalog_structure_and_reports_provider_unavailable
  - crates/unica-coder/src/infrastructure/meta_info_surface_tests.rs::info_preserves_local_structure_when_child_resource_evidence_is_unavailable
gap: https://github.com/IngvarConsulting/unica/issues/1933
---

# Ошибка дочернего ресурса не скрывает прочитанную структуру

Если типизированный читатель метаданных не может проверить дочерний ресурс,
он сохраняет сведения, уже прочитанные из исправного описания владельца.
Результат сообщает об ошибке `provider_unavailable`.

Например, неожиданный файл в каталоге формы не скрывает имя справочника
и наличие этой формы в его коллекции. Это частичный результат чтения,
а не подтверждение исправности формы.

Канонический `check` сообщает `provider_unavailable` и сохраняет структуру
в `view`; сам ответ `view` об ошибке пока не сообщает. Сохранение структуры
на внутреннем читателе проверено отдельно, ответ там собирает тест.
