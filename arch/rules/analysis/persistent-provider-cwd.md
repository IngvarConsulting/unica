---
id: INV.APP.PERSISTENT-PROVIDER-CWD
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_analyzer_child_does_not_run_from_inside_the_workspace
gap: https://github.com/IngvarConsulting/unica/issues/961
---

# Постоянный поставщик запускается вне рабочего дерева

Процесс поставщика, который остаётся жить после вызова, не получает рабочий
каталог внутри дерева проекта. Набор исходников передаётся явно; запуск
постоянного процесса не должен удерживать дерево только из-за его `cwd`.

Связанная проверка подтверждает выбор каталога BSL-анализатора на Unix.
Она не проверяет Windows и RLM и не доказывает освобождение всех иных
дескрипторов, которые могут мешать удалению рабочего дерева.
