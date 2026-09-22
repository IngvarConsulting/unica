---
id: INV.SOURCE.GENERATED-BUILD-READINESS
check:
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::build_and_cache_are_independent_facts_for_a_nested_root
  - crates/unica-coder/src/domain/project_health.rs::project_health_serializes_independent_source_and_repository_readiness
---

# Каталог .build внутри исходников делает проект неготовым

Если непосредственно в корне набора исходников есть каталог `.build`,
проверка готовности возвращает `ready: false` и диагностику
`source_set.generated_build_present` с его путём. Например, для набора
в `src/` нарушение — `src/.build/`. Кто создал каталог, значения не имеет.

Это нарушение чистоты исходников, а не доказательство повреждения файлов.
Проверка не удаляет содержимое и не выдаёт готовую команду удаления.
Отрицательный результат готовности сам по себе не означает запрета всех
операций чтения и редактирования.
