- Date: `2026-08-13`
- Status: `approved`
- Decision: `ADR-0059`

# Обновление RLM до `v1.33.0` через новую генерацию индекса

## Контекст

Unica поставляет две точки входа одного upstream-продукта
[`Dach-Coin/rlm-tools-bsl`](https://github.com/Dach-Coin/rlm-tools-bsl): MCP-процесс
чтения `rlm-tools-bsl` и CLI обслуживания индекса `rlm-bsl-index`. Обе записи
синхронно закреплены на `v1.29.1`, commit
`8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed` и неизменяемом release
`rlm-tools-bsl-v1.29.1-build.2` в `IngvarConsulting/unica-toolchain`.

Upstream `v1.33.0` закреплён аннотированным tag, который разворачивается в
commit `3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`. Релиз меняет builder индекса с
14 на 15. Старый индекс продолжает отвечать, а автоматическая полная
пересборка запускается не при старте MCP, а только внутри явного
`rlm-bsl-index index update`. Пересборка идёт in-place: прежнее содержимое
удаляется до наполнения, прерванная операция оставляет состояние `incomplete`.

Это делает прямую миграцию рискованной без пользы для Unica. На большой
реальной конфигурации холодный build `v1.29.1` занимал 141,28 секунды, что
приемлемо как одноразовая фоновая цена новой версии, но не помещается в текущие
provider- и search-дедлайны. Поэтому обновление не должно блокировать MCP и не
должно публиковать частично построенный результат.

Существующие имена бинарников также асимметричны: одно называет upstream-пакет,
другое — роль. До создания новых неизменяемых assets можно исправить это без
совместимого alias: текущий runtime Unica является единственным потребителем
внутренних путей и обновляется вместе с lock-файлом.

## Цели

1. Воспроизводимо собрать `v1.33.0` через `IngvarConsulting/unica-toolchain` и
   опубликовать обе точки входа для `darwin-arm64`, `linux-x64` и `win-x64`.
2. Назвать поставляемые бинарники по общей схеме: `rlm-bsl-mcp` и
   `rlm-bsl-index`.
3. Не мигрировать builder 14 в builder 15: новая версия получает отдельный
   namespace состояния и всегда строит индекс с нуля.
4. Сохранить старый namespace физически для безопасного отката и исключить его
   автоматическое удаление из upgrade-path.
5. Повторить замеры issue #485 на том же большом workspace и отделить скорость
   алгоритма upstream от поведения реально поставляемого PyInstaller asset.
6. Обновить Unica только после проверки опубликованных assets и измерений.

## Неграницы

1. Публичная поверхность `unica.*`, идентичность публичного MCP-сервера
   `unica` и provider-neutral формы результата не меняются.
2. Обновление не реализует event-driven broker, prewarm из `initialize` или
   provider-owned maintenance из #485.
3. Старый индекс не переносится, не обновляется и не удаляется автоматически.
   Безопасный сборщик оставшихся поколений проектируется отдельно.
4. Release identity upstream-продукта и manifest в `unica-toolchain` остаётся
   `rlm-tools-bsl`; переименовываются два выпускаемых executable и их записи в
   потребительском lock.
5. Новые helper-возможности `v1.33.0` не становятся новыми инструментами
   `unica.*`. Меняются только адаптеры контрактов, которые Unica уже потребляет.
6. Дефект размещения provider state внутри `sourceRoot` из #487 не маскируется
   новым именем каталога. Он закрывается отдельным предварительным изменением,
   если ещё не закрыт к началу consumer PR.
7. Версия самого плагина Unica этим изменением не определяется: обновляется
   закреплённая runtime-зависимость.

## Рассмотренные варианты

### 1. Новая генерация состояния и новые имена — выбран

Toolchain сначала публикует `rlm-bsl-mcp-*` и `rlm-bsl-index-*`. Unica
переключает обе записи lock одновременно и направляет `v1.33.0` в отдельный
namespace `rlm-bsl/index-v15`. Старый builder 14 никогда не открывается новой
версией и остаётся доступен предыдущей версии Unica при откате.

Вариант исключает смешанное чтение, in-place миграцию рабочей базы и удаление
данных, которые может использовать ещё живой старый процесс.

### 2. Обновить существующий namespace через `index update`

Upstream умеет запустить full rebuild, но старый индекс до этого выглядит
готовым, а rebuild очищает его in-place. Unica пришлось бы доказывать точное
распознавание builder mismatch, исключать все чтения и сохранять
восстановимость после обрыва. Вариант отклонён: одноразовый cold build новой
генерации проще и оставляет рабочий rollback.

### 3. Сначала физически удалить старый кеш

Удаление гарантирует cold build, но параллельная старая сессия может ещё читать
или обслуживать этот namespace. Кроме того, неудачный upgrade лишает откат
готового индекса. Вариант отклонён; логический cutover происходит сразу, а
физическая сборка мусора не входит в upgrade.

### 4. Сохранить `rlm-tools-bsl` как имя MCP-бинарника

Это минимизирует diff, но переносит асимметрию в новый неизменяемый release и
потребительские контракты. Вариант отклонён до публикации assets; выбранная
пара `rlm-bsl-mcp` / `rlm-bsl-index` явно называет общий продукт и роль.

## Контракты идентичности

### Источник и release group

- upstream repository: `Dach-Coin/rlm-tools-bsl`;
- source tag: `v1.33.0`;
- source commit: `3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`;
- toolchain manifest и immutable release tag:
  `rlm-tools-bsl-v1.33.0-build.1`.

### Поставляемые инструменты

| Роль | Старое имя | Новое имя |
| --- | --- | --- |
| stdio MCP reader | `rlm-tools-bsl` | `rlm-bsl-mcp` |
| index CLI | `rlm-bsl-index` | `rlm-bsl-index` |

Новый runtime не содержит совместимый alias `rlm-tools-bsl`. Toolchain release
публикует `rlm-bsl-mcp-<target>` и `rlm-bsl-index-<target>`; Windows assets
несут суффикс `.exe`. `plugins/unica/third-party/tools.lock.json`, runtime
manifest, bundled-tool resolver, проверки и атрибуции называют ту же пару.

### Состояние индекса

`RLM_INDEX_DIR` для `v1.33.0` указывает на
`<safe-provider-state-root>/rlm-bsl/index-v15`. Безопасный корень выводится из
нормализованных `workspaceRoot + sourceRoot`, остаётся вне индексируемого
`sourceRoot` и сохраняет worktree-изоляцию ADR-0018. Конкретный project hash
внутри `RLM_INDEX_DIR` остаётся частным форматом RLM.

Маркеры оркестратора также принадлежат поколению: status и lock builder 15
лежат соответственно под
`<safe-provider-state-root>/caches/rlm-bsl/index-v15/` и
`<safe-provider-state-root>/locks/rlm-bsl/index-v15/`. Маркеры прежней версии
не могут заблокировать cold build, объявить новую генерацию готовой или быть
перезаписаны новым процессом.

Namespace `rlm-tools-bsl`, использованный `v1.29.1`, не передаётся ни
`rlm-bsl-mcp`, ни `rlm-bsl-index` `v1.33.0`. Его наличие не делает новую
генерацию готовой и не запускает миграцию. Прежние status и lock также не
переиспользуются.

## Последовательность поставки

### Фаза A — `unica-toolchain`

1. В отдельном PR тесты manifest сначала закрепляют ожидаемые `v1.33.0`, commit,
   build revision, неизменные upstream `sourceName` и два новых `assetBase`.
2. `manifests/rlm-tools-bsl.json` получает `v1.33.0`, build revision `1`, новый
   commit, а `assetBase` — имена `rlm-bsl-mcp` / `rlm-bsl-index`.
   `sourceName` остаются именами опубликованных upstream `console_scripts`:
   `rlm-tools-bsl` и `rlm-bsl-index`. Поля `package` и модули upstream остаются
   соответственно `rlm_tools_bsl`, `rlm_tools_bsl.server` и
   `rlm_tools_bsl.cli`.
3. Source validation подтверждает tag, commit, frozen `uv.lock` и MIT license;
   тесты и Python compilation проходят до merge.
4. После merge вручную запускается `Build tool release` на `main`.
5. Release принимается только при полном наборе assets для трёх платформ,
   provenance и checksum-файлов, успешном `--help` обеих точек входа и
   независимом повторном скачивании байтов.
6. Частичный либо несовпавший release не становится входом Unica. Поскольку tag
   и assets неизменяемы, исправление создаёт следующий build revision.

### Фаза B — воспроизводимые замеры

После публикации и до consumer lock выполняются два прогона на неизменённом
workspace из issue #485:

1. опубликованный `rlm-bsl-index-darwin-arm64` измеряет поставляемые байты;
2. CLI из точного source commit `v1.33.0` в Python 3.12 измеряет алгоритм в
   форме, сопоставимой с прежним source-прогоном `v1.29.1`.

Оба прогона используют один harness и одинаковые сценарии: cold build, пять
no-op update, изменения 1/10/100 BSL-файлов, одной XML-формы и десяти корневых
XML. После каждого синтетического изменения файл восстанавливается из `HEAD`,
затем выполняется обратный update. Harness требует чистое tracked-дерево до
каждого сценария и после прогона, держит `RLM_INDEX_DIR` вне workspace и пишет
локальный JSON с raw durations, SHA-256 executable, source commit, выбранными
repo-relative файлами, `git_fast_path`, RSS, размером и статистикой индекса.

В issue #485 публикуется обезличенная сводка: число прогонов, медиана, диапазон,
сравнение `v1.29.1`/`v1.33.0` и различие source/packaged. Абсолютные пути и
имена клиентских объектов не публикуются. Quiet period, max batch delay и
provider deadline меняются только если измерения опровергают текущие значения.

### Фаза C — Unica

1. Если #487 ещё открыт, сначала независимо исправляется размещение persistent
   provider state вне `sourceRoot`; RLM upgrade не маскирует этот существовавший
   дефект и не становится дочерним PR.
2. Контрактные тесты переводятся на `rlm-bsl-mcp`, `rlm-bsl-index`, `v1.33.0`,
   toolchain tag и фактически опубликованные SHA-256 и сначала падают на старом
   lock/runtime.
3. Оба инструмента переключаются одним изменением; смешанная пара версий,
   release tags или source commits запрещена.
4. Workspace index service получает новый generation namespace. Тест с
   существующим готовым builder 14 доказывает, что он игнорируется и запускается
   cold build v15, а старый каталог остаётся нетронутым.
5. `missing` запускает single-flight build. `building` и `incomplete` блокируют
   чтение. Прерванный build новой генерации восстанавливается поддерживаемым
   upstream `index update`, но ни один его промежуточный результат не
   публикуется.
6. Используемые Unica helper-контракты проверяются на `v1.33.0`; изменённая
   форма `parse_form(...).attributes[].types` и признаки усечения адаптируются
   только там, где они реально потребляются.
7. Package/runtime smoke запускает извлечённые архивные байты на macOS, Linux и
   Windows, проверяет stdio MCP через `rlm-bsl-mcp`, index CLI и отсутствие
   старого executable в новом runtime.
8. Issue #488 получает ссылки на toolchain release, consumer PR, SHA-256,
   forced cold rebuild и проверки. Критерии миграции v14 → v15 удаляются как
   противоречащие принятому generational cutover.

## Состояния и ошибки

- Отсутствие любого опубликованного target, provenance или checksum останавливает
  процесс до изменения consumer lock.
- Новый namespace не считается готовым по наличию старого индекса или по
  readiness старого MCP-процесса.
- Cold build выполняется в фоне. Запрос до его завершения получает retryable
  `index_building` в пределах действующего deadline, а не stale/partial output.
- `incomplete` относится только к оборванной сборке builder 15. Оно
  восстанавливается внутри нового namespace при сохранённом read barrier и не
  является поводом открыть builder 14.
- Cleanup временного benchmark state выполняется только после доказанного
  восстановления tracked-дерева. Старые пользовательские cache generations
  benchmark и upgrade не удаляют.
- Неудачная Unica-версия откатывается на предыдущий runtime, который продолжает
  видеть прежний namespace. Новые release tags и assets не двигаются и не
  переиспользуются.

## Проверки

### `unica-toolchain`

- `python3.12 -m unittest discover -s tests`;
- `python3.12 -m py_compile scripts/*.py toolchain/*.py toolchain/builders/*.py tests/*.py`;
- `python3.12 scripts/toolchain.py validate-source --manifest manifests/rlm-tools-bsl.json ...`;
- успешный `Build tool release` для трёх targets;
- повторное скачивание release assets, SHA-256, provenance и native smoke.

### Unica

- падающие до исправления package/provenance tests для новой пары бинарников;
- unit tests `WorkspaceIndexService`: старый v14 игнорируется, новый v15
  строится, read barrier удерживается, `incomplete` восстанавливается;
- `python3.12 -m unittest tests.ci.test_skill_provenance`;
- `python3.12 -m unittest tests.ci.test_attributions`;
- `python3.12 -m unittest tests.ci.test_product_contracts`;
- `python3.12 -m unittest tests.ci.test_package_unica_runtime`;
- `cargo test --package unica-coder --lib`;
- `cargo test --package unica-coder --test issue_89_workspace_service`;
- package smoke извлечённого runtime на трёх targets и `git diff --check`.

### Измерения

- harness abort при грязном tracked-дереве до сценария;
- детерминированный выбор того же множества файлов для source и packaged;
- отдельные `RLM_INDEX_DIR` для каждого прогона;
- JSON содержит raw samples и provenance запуска;
- после прогона `git diff --quiet` и поиск синтетического маркера подтверждают
  восстановление workspace.

## Архитектурная оценка

Изменение не расширяет публичную MCP-поверхность, но меняет контракт упаковки
двух встроенных инструментов и стратегию persistent provider state. Поэтому
`Decision: none` был бы неверен. ADR-0059 владеет именами поставляемых
executable и правилом generational cutover; ADR-0018 продолжает владеть
worktree-изоляцией, границей поставщика и логической инвалидацией. Реализация
переводит ADR-0059 из `proposed` в `accepted` и добавляет выведенные правила
реестра и их исполняемые проверки тем же consumer PR.
