---
id: CTR.FORMAT.CFE-BORROWED-STRUCTURE
check:
  - crates/unica-coder/tests/support/cfe_structure.rs::canonical_stdio_checks_borrowed_cfe_structure_without_writes
  - crates/unica-coder/tests/support/cfe_structure.rs::canonical_stdio_borrow_refresh_preserves_module_and_identity
  - crates/unica-coder/src/infrastructure/native_operations/cfe.rs::cfe_validate_checks_every_platform_direct_and_owner_module_role
---

# Проверка расширения обнаруживает повреждённую структуру заимствованного объекта

При первом заимствовании `object.borrow` создаёт обязательный `ChildObjects`
по физическому профилю 8.3.27, даже если дочерних объектов нет.
На корне расширения `unica.check` отвергает отсутствие, повторение,
чужое пространство имён и расположение контейнера перед `Properties`.
Пустой контейнер и другой префикс того же пространства имён допустимы.

Для существующих BSL-файлов прямых модулей заимствованного объекта,
модуля владельца (общий модуль, бот, сервисы) и общей команды проверка
требует единственный корректный `PropertyState=Extended` для роли модуля.
Повреждённая запись, в том числе вложенные элементы или посторонний текст,
не считается подключённым модулем. Проверка сообщает `status: failed`
и диагностику валидатора `cfe`, сохраняя исходные файлы.

Запись BSL подключает модуль по [правилу записи](code-borrowed-module-state.md).
Повторное заимствование сохраняет локальный XML по
[правилу обновления](borrowed-object-refresh.md); исправление уже повреждённого
дескриптора не становится побочным эффектом повторного заимствования.
