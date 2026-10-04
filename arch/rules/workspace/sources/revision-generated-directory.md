---
id: INV.SOURCE.REVISION-EXCLUDES-GENERATED-DIRECTORY
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::source_generated_guard_uses_each_retained_parent_case_policy
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::source_role_rejects_platform_equivalent_generated_components
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_allows_exact_generated_cache_descendant
---

# План исходников не включает служебный каталог .build

Участник `apply`, изменяющий исходники, не получает доступ к `.build`
на любом уровне дерева. При перечислении нужного поддерева этот каталог
и его содержимое исключаются из входов плана. Имя сравнивается по правилам
файловой системы содержащего каталога.

Запись служебного кеша требует отдельных полномочий на точный каталог
кеша. Совпадение корня набора исходников с корнем рабочей области не
позволяет выдать такие полномочия обычному участнику исходников.

Проверка готовности отдельно сообщает о `.build` в корне набора;
исключение из плана не делает каталог частью исходников.
