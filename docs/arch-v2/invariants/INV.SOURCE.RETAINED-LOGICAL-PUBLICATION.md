---
id: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
---

# Неподтверждённые границы публикации логического чтения

Число проходов не зависит от числа прочитанных логических узлов.
После final confirmation source I/O нет.
Общий абсолютный срок logical read — 120 секунд без пополнения бюджета.

Подмена вложенного каталога или файла во время final confirmation должна
отклонять результат и при совпадающих байтах. Эта граница не выполнена:
после однократной подмены код может повторно стабилизировать дерево и
подтвердить прежнюю семантическую ревизию. Сценарии — варианты A→A проверок
`review_final_confirmation_rejects_nested_directory_replacement_after_retention`
и `review_final_confirmation_rejects_file_replacement_after_retention`
в `crates/unica-coder/src/infrastructure/source_revision.rs`.
