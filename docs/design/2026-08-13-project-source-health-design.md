- Date: `2026-08-13`
- Status: `approved`
- Decision: `ADR-0056`

# Типизированная проверка готовности проекта и наборов исходников

## Результат проектирования

`unica.project.status` становится единственной явной точкой полной проверки
рабочего пространства. Вызов остаётся только читающим и возвращает два
независимых результата:

- `ready` отвечает, может ли Unica безопасно работать с объявленными наборами
  исходников;
- `repositoryReady` отвечает, удовлетворяет ли репозиторий переносимому
  командному Git-контракту.

Отсутствие Git-репозитория не делает Unica целиком неработоспособной:
`ready` может быть `true`, а `repositoryReady` будет `false`. Такой ответ не
скрывает проблему совместной работы и одновременно не выдаёт Git за
обязательное условие чтения или изменения Platform XML.

Проверка не добавляется в каждый предметный инструмент. Она выполняется только
по явному вызову `unica.project.status`, чтобы операции не получили скрытую
стоимость, зависимость от Git и новые отказы.

## Исходное состояние

На текущем `main` `unica.project.status` возвращает корни рабочего пространства
и кеша и найденные `sourceSets`. Ошибка обнаружения наборов попадает в
`warnings` конверта. Отдельный `GitTrackingAdapter` проверяет отслеживаемые
`ConfigDumpInfo.xml`, но возвращает одну строку; эту же проверку вызывают и
`unica.project.status`, и `unica.project.map`.

У такой формы четыре недостатка:

1. AI-потребитель обязан разбирать прозу и не может надёжно отличить область,
   код, доказательство и способ исправления.
2. Карта источников смешана с оценкой здоровья репозитория.
3. Нет общего результата готовности: отсутствие Git, неверный корень источника
   и неполная проверка выглядят как несвязанные предупреждения.
4. Текущий default-кеш `<workspace>/.build/unica` и допустимый `sourceSet.path: .`
   позволяют служебным данным оказаться внутри корня исходников. PR
   [#473](https://github.com/IngvarConsulting/unica/pull/473) отдельно выносит
   кеш `bsl-analyzer` из source root, но не запрещает саму ошибочную топологию
   набора исходников.

## Вендорные рекомендации и граница их применения

Исходными рекомендациями служат страницы 1C-Company GitConverter:

- [Символы окончания строк](https://github.com/1C-Company/GitConverter/wiki/Символы-окончания-строк);
- [Дополнительная настройка репозитория Git](https://github.com/1C-Company/GitConverter/wiki/Дополнительная-настройка-репозитория-Git);
- [Git LFS](https://github.com/1C-Company/GitConverter/wiki/Git-LFS).

Рекомендации написаны для EDT и не являются готовой спецификацией Platform XML.
Их нельзя переносить буквально:

- страница об окончаниях строк одновременно даёт разные OS-настройки
  `core.autocrlf` и локальную настройку GitConverter `core.autocrlf=true`;
  поэтому конкретное значение пользовательского Git config не становится
  критерием готовности;
- рекомендация `*.bin binary` неверна для всех Platform XML ресурсов:
  `XDTOPackages/<Имя>/Ext/Package.bin` является текстовым XML согласно
  `plugins/unica/references/specs/1c-xdto-spec.md`;
- LFS описан как оптимизация производительности. Его отсутствие не означает,
  что источник небезопасен или непереносим;
- `ConfigDumpInfo.xml` бывает и служебным `<ConfigDumpInfo>`, и законным
  дескриптором метаданных, включая EPF/ERF. Решение по одному имени файла
  опасно.

Поэтому проверка использует вендорные цели — LF в индексе, явная
текстовая/двоичная классификация, исключение служебных файлов — но применяет их
к доказанным ролям ресурсов каждого формата.

## Границы решения

### Входит

- топология workspace и всех объявленных наборов исходников;
- общий переносимый Git-контракт;
- профиль Platform XML для Configuration и Extension;
- фактические служебные пути Unica и физические `.build` внутри source root;
- правила `.gitignore`, `.gitattributes`, EOL индекса и рабочего дерева;
- классификация `ConfigDumpInfo.xml` по staged blob;
- необязательная рекомендация LFS для доказанных больших бинарных ресурсов;
- безопасная инструкция AI по исправлению каждой доказанной проблемы.

### Не входит

- автоматическое изменение файлов, индекса или Git config;
- проверка ветки, remote, чистоты рабочего дерева и серверных правил;
- `core.autocrlf`, `core.safecrlf`, `core.quotepath`, `renameLimit` и GUI encoding;
- анализ истории и переписывание уже попавших в неё бинарных объектов;
- специальная политика EDT сверх общих проверок;
- запуск проверки из `project.map` или каждого предметного инструмента;
- перенос кеша `bsl-analyzer`, которым владеет PR #473.

## Архитектурная граница

Application-координатор `ProjectHealthCoordinator` получает
`WorkspaceContext` и снимок карты источников, запрашивает факты через внутренние
порты и передаёт единый снимок чистым доменным правилам. Правила не читают
файловую систему и не запускают Git; инфраструктурные сборщики не принимают
решений о severity и готовности.

Предлагаемые внутренние части:

- `SourceLayoutInspector` — пути, идентичность, существование, containment,
  symlink/reparse и физические служебные каталоги;
- `GitRepositoryInspector` — репозиторий, index, ignore provenance, attributes,
  EOL и staged blobs;
- `SourceResourcePolicyInspector` — доказанные роли ресурсов для общего и
  Platform XML профилей;
- доменные `ProjectCheck`, `ProjectDiagnostic`, `DiagnosticSeverity`,
  `DiagnosticScope` и `Remediation`;
- набор чистых правил, принимающий immutable fact snapshot.

Проверка выполняется в рамках одного вызова, под его deadline и cancellation.
Она не запускает скрытые сервисы рабочего пространства и не создаёт фоновое
состояние. Существующий классификатор `ConfigDumpInfoXmlKind` переиспользуется,
а не дублируется.

`unica.project.map` после миграции только сообщает карту источников. Строковые
Git-предупреждения из `GitTrackingAdapter` удаляются из обоих инструментов;
эквивалентная и расширенная информация остаётся типизированной только в
`unica.project.status`.

## Публичная модель результата

Успешно завершённая инспекция возвращает `ok=true`, даже если нашла ошибки
проекта. Ошибки проекта находятся в `data.diagnostics`, а не в `errors`
конверта. `ok=false` остаётся для отмены всего вызова и внутреннего отказа
координатора, при котором достоверный статус сформировать нельзя.

```json
{
  "workspaceRoot": "/workspace",
  "cacheRoot": "/workspace/.build/unica",
  "ready": false,
  "repositoryReady": false,
  "checks": [
    {
      "id": "source.layout",
      "scope": "sourceSet",
      "sourceSet": "main",
      "status": "failed",
      "reason": "source root equals workspace root"
    }
  ],
  "sourceSets": [
    {
      "name": "main",
      "kind": "configuration",
      "path": ".",
      "sourceFormat": "platform_xml",
      "formatEvidence": ["Configuration.xml"]
    }
  ],
  "diagnostics": [
    {
      "code": "source_set.root_is_workspace",
      "severity": "error",
      "scope": "sourceSet",
      "sourceSet": "main",
      "paths": ["."],
      "count": 1,
      "message": "Source root resolves to the workspace root",
      "evidence": ["normalized identity: /workspace"],
      "remediation": {
        "summary": "Separate the 1C source root from the workspace root",
        "steps": [
          "Create a dedicated source subdirectory that is a strict child of the workspace",
          "Set the source-set path in v8project.yaml to the new subdirectory instead of .",
          "Run unica.project.status again"
        ],
        "commands": []
      }
    }
  ]
}
```

`sourceSets[]` сохраняет действующую форму карты. Данные здоровья не
дублируются в каждой записи: связь задаётся `sourceSet` у проверки и
диагностики.

### Два контура готовности

- error с областью `workspace` или `sourceSet` делает `ready=false`;
- error с областью `repository` делает `repositoryReady=false`;
- одна диагностика имеет одну область и изменяет только соответствующий
  контур; общая физическая причина представляется раздельными находками, если
  для двух контуров нужны разные исправления;
- warning и info не изменяют флаги;
- неполная обязательная проверка имеет `status=notRun` и закрывает только свой
  контур: source-проверка — `ready`, Git-проверка — `repositoryReady`;
- `notApplicable` означает доказанную неприменимость и не закрывает контур.

Отсутствие Git даёт одну диагностику `git.repository_absent`,
`repositoryReady=false` и `notRun` у зависящих от Git проверок. При этом
полностью проверенные source sets могут оставить `ready=true`.

### Проверка

```json
{
  "id": "repository.ignore",
  "scope": "repository",
  "sourceSet": "main",
  "status": "passed",
  "reason": null
}
```

Допустимые `status`: `passed`, `failed`, `notRun`, `notApplicable`.
`sourceSet` отсутствует для проверки всего workspace/repository.

### Диагностика и исправление

```json
{
  "code": "git.runtime_sidecar_tracked",
  "severity": "error",
  "scope": "repository",
  "sourceSet": "main",
  "paths": ["src/ConfigDumpInfo.xml"],
  "count": 1,
  "message": "Platform-generated ConfigDumpInfo.xml is tracked",
  "evidence": ["staged root element: ConfigDumpInfo"],
  "remediation": {
    "summary": "Stop tracking the runtime sidecar and add a portable ignore rule",
    "steps": [
      "Add a matching rule to a tracked .gitignore",
      "Remove only the classified runtime sidecar from the index",
      "Run unica.project.status again"
    ],
    "commands": [
      {
        "program": "git",
        "argv": ["rm", "--cached", "--", "src/ConfigDumpInfo.xml"],
        "cwd": "/workspace"
      }
    ]
  }
}
```

Команда представлена полями `program`, `argv` и `cwd`, а не shell-строкой.
Так путь с пробелом, кавычкой, переводом строки или начальным `-` остаётся
данными и не превращается в код оболочки. `commands` может быть пустым: AI
получает команду только когда классификация и precondition доказаны.

Для неоднозначного `ConfigDumpInfo.xml` инструкция требует ручной проверки и не
содержит `git rm`. Для `path: .` она объясняет создать отдельный каталог
исходников, перенести выгрузку поддерживаемым способом и обновить
`v8project.yaml`; универсальная разрушительная команда `mv` не предлагается.

## Каталог первичных диагностик

Диагностические коды стабильны для машинного потребителя. Новая диагностика
может добавляться без новой ADR, если она сохраняет схему, области и исчисление
готовности. Изменение смысла существующего кода или флагов требует пересмотра
контракта.

### Топология workspace и source sets

| Код | Severity | Причина | Контур |
| --- | --- | --- | --- |
| `source_set.inspection_incomplete` | error | Конфигурацию наборов исходников нельзя прочитать или разобрать полностью | `ready` |
| `source_set.none_found` | error | Инспекция завершилась, но не нашла ни одного набора исходников | `ready` |
| `source_set.root_is_workspace` | error | Нормализованный source root равен workspace root, включая `.`, `./` и alias через symlink/reparse | `ready` |
| `source_set.path_missing` | error | Объявленный корень не существует | `ready` |
| `source_set.path_unsafe` | error | Корень выходит из workspace или containment нельзя доказать | `ready` |
| `source_set.name_ambiguous` | error | Имя набора не уникально | `ready` |
| `source_set.format_invalid` | error | Признаки форматов противоречат друг другу | `ready` |
| `source_set.format_unknown` | error | Формат нельзя доказать | `ready` |
| `cache.inside_source_set` | error | Действующий cache root находится внутри source root | `ready` |
| `source_set.generated_build_present` | error | В source root физически присутствует служебный `.build` | `ready` |

`source_set.root_is_workspace` проверяет идентичность пути, а не только
буквальный YAML. Автообнаруженный набор с корнем workspace нарушает то же
правило. Это устраняет причину: source root обязан быть строгим потомком
workspace, чтобы служебные файлы проекта не смешивались с выгрузкой.

### Переносимый Git-контракт

| Код | Severity | Причина | Контур |
| --- | --- | --- | --- |
| `git.repository_absent` | error | Workspace не находится в Git work tree | `repositoryReady` |
| `git.executable_unavailable` | error | Обязательные Git-факты нельзя собрать | `repositoryReady` |
| `git.inspection_timeout` | error | Git-проверка не уложилась в бюджет | `repositoryReady` |
| `git.inspection_incomplete` | error | Вывод усечён, потерян или не разобран полностью | `repositoryReady` |
| `git.ignore_rule_missing` | error | Обязательный служебный путь не покрыт tracked `.gitignore` | `repositoryReady` |
| `git.ignore_rule_local_only` | error | Путь исключён только global excludes или `.git/info/exclude` | `repositoryReady` |
| `git.generated_path_tracked` | error | Служебный путь уже находится в index | `repositoryReady` |
| `git.runtime_sidecar_tracked` | error | Staged blob доказан как runtime sidecar | `repositoryReady` |
| `git.config_dump_info_unclassified` | error | Одноимённый staged blob нельзя безопасно классифицировать | `repositoryReady` |
| `git.attributes_local_only` | error | Нужный effective attribute задан только локальным `.git/info/attributes`, global или system policy | `repositoryReady` |
| `git.text_policy_missing` | error | Доказанный текстовый ресурс не классифицирован как text | `repositoryReady` |
| `git.binary_policy_missing` | error | Доказанный бинарный ресурс не классифицирован как `-text` | `repositoryReady` |
| `git.text_resource_marked_binary` | error | Текстовый ресурс, включая XDTO `Package.bin`, покрыт binary-политикой | `repositoryReady` |
| `git.index_eol_not_lf` | error | Текстовый blob в index не нормализован в LF | `repositoryReady` |
| `git.mixed_eol` | error | Рабочий текстовый файл содержит смешанные EOL | `repositoryReady` |
| `git.working_eol_unsupported` | error | Рабочий текстовый файл использует одиночный CR вместо LF или CRLF | `repositoryReady` |
| `git.lfs_consider` | info | Доказанные бинарные ресурсы превысили порог рекомендации | не влияет |

Если одна атрибутная причина одновременно означает отсутствие корректной
политики и ошибочную binary-классификацию текстового ресурса, публикуется более
точный `git.text_resource_marked_binary`, а не два следствия.

## Обязательные ignore-пути

Правило считается переносимым, только если его источник — отслеживаемый Git
файл `.gitignore`. Global excludes и `.git/info/exclude` полезны локально, но не
переезжают с clone и поэтому дают `git.ignore_rule_local_only`.

Проверка выполняется и до появления файла: для каждого обязательного пути
строится безопасный синтетический кандидат и вызывается ignore matching с
provenance. Это позволяет сообщить отсутствие правила сразу после clone.

Общий профиль требует исключить:

- фактический `cacheRoot`, если он находится внутри Git work tree;
- `.build/` внутри каждого source root.

Профиль Platform XML для Configuration и Extension дополнительно требует
исключить в корне набора:

- `ConfigDumpInfo.xml` с runtime-ролью;
- `DumpFilesIndex.txt`.

Для ExternalDataProcessor и ExternalReport правило на
`ConfigDumpInfo.xml` заранее не требуется: это имя может принадлежать
дескриптору исходника. Существующий файл проверяется классификацией содержимого.

Факт игнорирования не заменяет проверку index. Уже отслеживаемый служебный файл
остаётся ошибкой, даже если добавлено подходящее правило.

## Attributes и окончания строк

Platform XML профиль классифицирует ресурсы по доказанной роли, а не по одному
расширению:

- переносимая attributes policy доказывается tracked `.gitattributes` из index;
  `.git/info/attributes`, global и system attributes не засчитываются;
- XML, BSL и доказанные текстовые `.bin` обязаны иметь text policy;
- доказанные двоичные ресурсы обязаны иметь `-text`;
- broad `*.bin binary` не принимается, если оно захватывает XDTO
  `Ext/Package.bin`;
- staged-представление текстового файла обязано иметь LF;
- working tree может единообразно использовать LF или CRLF;
- смешанные LF/CRLF/CR внутри одного файла являются ошибкой;
- единообразный одиночный CR тоже является ошибкой;
- `eol=lf` не обязательно: `eol=crlf` допустимо, если index остаётся LF.

Это не меняет действующий writer-контракт сохранения наблюдённого EOL. Health
check только обнаруживает состояние repository/worktree и ничего не
нормализует.

## LFS

LFS остаётся рекомендацией уровня `info`. Находка агрегируется по профилю:
количество доказанных бинарных файлов, общий размер, самый большой файл и
несколько безопасных примеров. Она не влияет ни на `ready`, ни на
`repositoryReady`.

Рекомендация строится только для точных ролей или безопасных узких шаблонов.
Команда `git lfs track "*.bin"` не предлагается, потому что ломает текстовые
`Package.bin`. Порог является версионируемой внутренней настройкой профиля и
публикуется в evidence, чтобы результат был объяснимым.

## Сбор доказательств

Git-сборщик использует только argv-вызовы без shell interpolation:

- `git rev-parse` — наличие и корень work tree;
- `git ls-files -z` — tracked/index entries и происхождение tracked
  `.gitignore`/`.gitattributes`;
- `git check-ignore -v --no-index` — совпавшее правило и файл-источник;
- `git check-attr -z --cached --stdin` — effective attributes, а второй
  `--cached`-probe в изолированном index/GIT_DIR доказывает portable staged
  policy без `$GIT_DIR/info/attributes` и global/system rules;
- `git ls-files --eol -z` — только index EOL в изолированном index с пустым
  worktree; реальный worktree читается отдельно, bounded и component-wise
  no-follow;
- `git cat-file` — staged blob для содержательной классификации.

Решение принимается по index, а не только по рабочему файлу: именно staged blob
будет опубликован следующим commit. Неслитые stages, недоступный blob,
некорректный UTF-8 пути, усечённый вывод и превышение размера не превращаются в
`passed`.

Каждый процесс имеет deadline, cancellation, предел stdout/stderr и явный
результат полноты. Для списков используются NUL-разделители. Файловые обходы
ограничены объявленными source roots, не переходят по symlink/reparse и не
сканируют `target`, `dist`, `docs-local`, `docs/design`, `docs/plans` и другие
внесистемные корпуса.

## Дедупликация причин

Ответ показывает первичную исправимую причину:

1. `source_set.root_is_workspace` является единственной первичной причиной для
   отклонённого корня: производные source-layout и source-scoped Git-проверки
   служебных путей в том же корне получают `notRun`. При этом доказанный
   соседний source set продолжает независимые Git-проверки и не блокируется
   ошибкой отклонённого корня.
2. Ошибка обнаружения source set даёт
   `source_set.inspection_incomplete` и делает зависимые проверки `notRun`
   вместо каскада вымышленных находок. Полный пустой результат даёт отдельный
   `source_set.none_found`.
3. Отсутствие Git даёт одну диагностику; ignore/attributes/EOL становятся
   `notRun`.
4. Неоднозначный `ConfigDumpInfo.xml` даёт одну ручную диагностику без команды
   удаления.
5. Ошибочная broad binary policy для XDTO даёт
   `git.text_resource_marked_binary`, а не параллельные missing/conflict.

Диагностики сортируются детерминированно по severity, scope, `sourceSet`, code и
path. Одинаковые причины агрегируются с count и ограниченным числом примеров;
полный список не теряется из внутреннего факта, но публичный ответ соблюдает
бюджет.

## Ошибки и неполные проверки

Отказ одного сборщика не разрешает ставить зависимым правилам `passed`:

- неполная source-проверка делает `ready=false`;
- неполная repository-проверка делает `repositoryReady=false`;
- доказанная неприменимость даёт `notApplicable`;
- отмена общего вызова сохраняет стандартную семантику cancelled и `ok=false`;
- внутренний дефект сериализации или координации даёт `ok=false`, потому что
  достоверного снимка нет.

Warnings/errors верхнего конверта не дублируют `data.diagnostics`. Они остаются
только для проблемы исполнения самой инспекции, которая помешала сформировать
достоверный typed result.

## Связь с PR #473

PR #473 владеет размещением кеша `bsl-analyzer` вне source roots и проверкой
пути, который он передаёт внешнему процессу. Этот проект не повторяет изменение
поставщика и не считает его уже доставленным.

После интеграции #473 health check проверяет фактический `cacheRoot` и
физическую топологию. Даже исправленный provider cache не делает допустимым
`sourceSet.path: .`: отдельный корень исходников остаётся самостоятельным
условием безопасности.

## Проверки реализации

### Домен

- исчисление двух флагов по scope/severity;
- `notRun` и `notApplicable`;
- неразобранная и доказанно пустая карта source sets;
- подавление производных диагностик для корневой причины;
- стабильная сортировка и агрегация;
- команда remediation появляется только при доказанном precondition.

### Топология

- буквальный `.`, `./`, нормализованный alias и symlink/reparse на workspace;
- отсутствующий и выходящий наружу путь;
- физический `.build` и cache внутри source root;
- корректно разделённые `src/cf`, `src/cfe`, `src/epf`, `src/erf`.

### Ignore

- tracked `.gitignore`, nested `.gitignore`, parent repository и linked
  worktree;
- отсутствие правила до появления файла;
- правило только в global excludes или `.git/info/exclude`;
- уже tracked generated path;
- Unicode, пробел, перевод строки и начальный `-` в пути.

### Attributes и EOL

- LF в index и LF/CRLF в working tree;
- CRLF или смешанный EOL в index;
- mixed EOL в working tree;
- единообразный одиночный CR в working tree;
- доказанный binary и текстовый XDTO `Package.bin`;
- конфликтующие вложенные attributes;
- отсутствие и наличие LFS policy.

### ConfigDumpInfo

- staged/runtime sidecar при отличающемся working file;
- законный metadata descriptor и EPF/ERF;
- malformed, oversized, symlink и unmerged stages;
- отсутствие команды удаления при неоднозначности.

### Публичная граница

- `project.status` возвращает typed result и `ok=true` при найденной проблеме;
- `project.map` больше не запускает Git и не несёт Git warning;
- `project.status` не запускает hidden services;
- схема, operation descriptor, tool-surface, skill и package surface
  синхронизированы;
- MCP smoke проверяет форму и детерминированность результата.

## Рассмотренные альтернативы

### Проверять перед каждой операцией

Отклонено: Git перестал бы быть независимым контуром, чтение Platform XML без
репозитория стало бы невозможно, а стоимость и новые точки отказа появились бы
у каждого инструмента.

### Новый публичный `unica.project.check`

Отклонено: `unica.project.status` уже является точкой ответа на вопрос о
готовности workspace. Второй инструмент дублировал бы назначение и расширял
публичную поверхность без отдельной потребности.

### Оставить строковые warnings

Отклонено: AI пришлось бы разбирать нестабильную прозу; нельзя выразить два
контура готовности, неполную проверку и безопасно обусловленную remediation.

### Требовать Git для `ready`

Отклонено как фактически неверное: нативные Platform XML операции могут
работать без `.git`. Это смешало бы работоспособность Unica с переносимостью
командной разработки.

### Автоматически исправлять репозиторий

Отклонено: изменение `.gitignore`, `.gitattributes` или index требует решения
владельца проекта и может удалить законный одноимённый исходник. Unica даёт AI
доказательство и безопасный план, но остаётся read-only.

## Порядок доставки

1. Добавить падающие доменные и инфраструктурные тесты для каждого первичного
   кода и семантики полноты.
2. Ввести fact snapshot, внутренние порты и чистые правила.
3. Перевести `unica.project.status` на новую модель и убрать Git warning из
   `project.map`.
4. Вместе с кодом перевести ADR-0056 в `accepted`, добавить выведенные правила
   в реестр, синхронизировать tool surface, change checklist, operation
   descriptors, skill и package contracts.
5. Запустить unit, platform-specific, MCP smoke и архитектурные стражи.

До выполнения этих шагов ADR остаётся `proposed`: проект согласован, но
поведение ещё не считается доставленным.
