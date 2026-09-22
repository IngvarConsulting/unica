---
id: INV.CACHE.WORKSPACE-ROOT
check:
  - crates/unica-coder/src/infrastructure/workspace.rs::v8project_yaml_in_ancestor_defines_workspace_root
---

# Базовый кеш проекта находится в его каталоге .build/unica

Если `UNICA_CACHE_DIR` не задан, Unica выбирает базовый каталог кеша
`<корень проекта>/.build/unica`, даже при запуске из вложенного каталога.
Например, при запуске из `src/catalogs` корнем проекта служит каталог
с найденным в предках `v8project.yaml`.

Выбор отдельного каталога для анализатора описан в
[правиле о кеше вне исходников](analyzer-cache-outside-source.md).
