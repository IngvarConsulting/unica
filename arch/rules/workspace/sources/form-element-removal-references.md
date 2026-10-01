---
id: INV.SOURCE.FORM-ELEMENT-REMOVAL-REFERENCES
check:
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::form_edit_remove_rejects_surviving_supported_xml_references
---

# Удаление элемента не оставляет поддержанные ссылки формы без цели

Редактор отказывает, если в оставшейся части формы сохраняется ссылка
на удаляемый элемент или его вложенный элемент. Проверяются пути
`Items.<имя>`, стандартные команды `Form.Item.<имя>.StandardCommand`
и связи `AdditionSource/Item`, включая имена с точками.

Это проверка поддержанных связей XML формы, а не поиск всех обращений
в BSL-коде. Проверка проходит внутренний редактор и сравнивает отказ
предпросмотра и применения с сохранением исходных байтов.
