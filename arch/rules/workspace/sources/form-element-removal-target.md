---
id: INV.SOURCE.FORM-ELEMENT-REMOVAL-TARGET
check:
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_preview_plans_exact_subtree_and_reports_contained_nodes
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_matches_element_names_exactly_without_trimming
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_scopes_extension_targets_to_the_working_tree
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_never_targets_a_baseline_only_element
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_preview_rejects_protected_root_and_nested_targets
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_preview_rejects_overlapping_requested_subtrees
---

# Удаление элемента формы выбирает точное имя в редактируемой части

Редактор удаляет выбранный элемент целиком с его вложенными элементами.
Похожие имена соседей не затрагиваются. Отсутствующий элемент, в том числе
при повторном удалении, вызывает отказ; пробелы в имени не отбрасываются.

В расширении поиск не выбирает элементы сохранённой базовой формы `BaseForm`.
Контекстное меню и подсказку нельзя удалить отдельно от элемента-владельца.
Удаление родителя и его потомка одним списком также отклоняется.

Проверки проходят внутренний редактор формы: сравнивают результат,
содержимое базовой формы и сохранение исходных байтов при отказе.
