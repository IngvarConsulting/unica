---
id: INV.SOURCE.TAIL-INSERT
check:
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_writes_the_first_body_of_an_empty_or_bom_only_module
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_first_body_materializes_a_missing_module_chain
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_missing_module_chain_keeps_a_foreign_appearance
---

# Первый текст модуля не требует отдельного создания файла

Вставка BSL в допустимый модуль зарегистрированного владельца создаёт
отсутствующий файл при применении. Предпросмотр ничего не создаёт.
Недопустимая для вида владельца роль не становится доступной из-за
переданного текста. Отдельная операция инициализации не нужна.

Пустой модуль и модуль только с BOM принимают первый текст тем же способом;
BOM сохраняется. Это проверено внутренним редактором.

Модуль может не иметь файла и его каталогов. В выгрузке платформы у пустого
общего модуля остаётся пустой `Ext/` без `Module.bsl`, а у объектов без
модулей нет и каталога владельца. `object.borrow` регистрирует общий модуль
расширения без каталога модуля. Публикация создаёт вместе с файлом
недостающие `Ext/` и каталог модуля. Файл, появившийся после планирования,
не перезаписывается: публикация отказывает. Проверки охватывают планировщик
BSL и публикацию через актора; откат новой цепочки каталогов описан
в [правиле отката](../retained-apply-rollback.md).
