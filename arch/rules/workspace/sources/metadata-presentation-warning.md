---
id: INV.APP.META-FINDINGS
check:
  - crates/unica-coder/src/application/meta_add_surface_tests.rs::info_uses_nonempty_list_presentation_for_command_text_finding
---

# Предупреждение указывает поле, из которого взят текст команды

При проверке длины текста команды непустое представление списка
`ListPresentation` имеет приоритет перед синонимом `Synonym`.
Предупреждение содержит код нарушения, поле `properties.ListPresentation`
и язык проверенного текста; оно не приписывает ту же ошибку синониму.

Проверка подтверждает `command_text_upper_limit` для русского представления
длиннее 38 символов.
