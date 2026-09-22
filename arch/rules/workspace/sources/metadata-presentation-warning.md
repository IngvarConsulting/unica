---
id: INV.APP.META-FINDINGS
check:
  - crates/unica-coder/src/application/meta_info_surface_tests.rs::info_warns_when_list_presentation_duplicates_the_synonym
  - crates/unica-coder/src/application/meta_add_surface_tests.rs::info_uses_nonempty_list_presentation_for_command_text_finding
---

# Предупреждение о представлении называет поле и язык

При проверке длины текста команды валидатор выбирает непустое представление
списка `ListPresentation` вместо синонима `Synonym`.
Предупреждение содержит код нарушения, поле `properties.ListPresentation`
и язык проверенного текста; оно не приписывает ту же ошибку синониму.

Если представление списка повторяет синоним, валидатор возвращает
`redundant_list_presentation` с теми же полем и языком. Обе находки остаются
предупреждениями; их код не извлекается из текста сообщения.

Проверки проходят внутренний читатель метаданных: русское представление
длиннее 38 символов и повторение синонима. Они не проверяют весь путь `unica.check`.
