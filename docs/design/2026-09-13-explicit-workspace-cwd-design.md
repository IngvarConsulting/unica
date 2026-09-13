- Date: `2026-09-13`
- Status: `approved`
- Decision: `DEC.2026-09-13.EXPLICIT-WORKSPACE-CWD`

# Сохранение явного cwd после удаления каталога запуска

PR #622 сохраняет абсолютный выбор рабочего пространства без вычисления
`env::current_dir()`. Дефект остаётся в upstream/main: `unwrap_or` вычисляет
fallback даже при `Some` с абсолютным путём. Общая политика resolver закреплена
в `INV.APP.EXPLICIT-WORKSPACE-CWD`.

После перехода на v0.13 публичные инструменты не принимают `cwd`: frontend
сохраняет абсолютный workspace hint при запуске и передаёт его daemon.
Поэтому старые вызовы project.map, meta.info и code.search заменены в smoke
на `unica.view {}` с проверкой `structuredContent`, фактического корня и
конфигурации проекта. Публичный контракт не меняется.

Smoke использует порождённый packaged launcher, тестовую замену bootstrap,
которая запускает собранное ядро, и настоящий daemon с отдельным состоянием.
Он удаляет каталог frontend, затем заменяет внешний каталог provider-state,
сохраняя inode и исходные пути его дочерних файлов и каталогов. Старый пустой
каталог удаляется: daemon теряет process cwd, но сохраняет ledger и endpoint.
Проверка PID и instanceId исключает успешный ответ нового daemon.

На точном workspace.rs из upstream/main smoke падает после удаления cwd
daemon с `provider_unavailable: failed to read current directory`. С
исправлением он проходит. Абсолютный, относительный и отсутствующий выбор
также проверены отдельным процессом с физически удалённым cwd; unit-тесты
проверяют те же ветви через внедряемый current-dir resolver и нормальное
разрешение относительного пути. Удаление активного cwd не поддерживается
Windows, поэтому процессный smoke там пропускается; unit-тесты переносимы.
