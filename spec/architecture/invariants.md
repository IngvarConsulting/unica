# Реестр архитектурных инвариантов

Этот документ — реестр правил, которые должны оставаться верными при развитии
Unica. Каждая запись формулирует одно нормативное правило, называет решение, из
которого оно следует, и проверку, которая обязана упасть, если правило нарушено.
Машинно проверяется форма записи и то, что названная проверка действительно
исполняется в CI; что она проверяет именно это правило, удерживает ревью.
Граница между тем и другим описана ниже, в разделе «Как устроен реестр»; читать
реестр, не зная её, значит принимать на веру больше, чем он доказывает.

Реестр не пересказывает архитектуру и не заменяет описание системы: он фиксирует
то, что нельзя сломать молча. Если изменение нарушает инвариант, сначала нужна
новая запись решения, которая явно заменяет или уточняет действующую; после
этого правятся запись реестра и её проверка. Правка записи без записи решения —
дефект процесса, а не редакторская работа.

## Как читать реестр

- Записи сгруппированы по областям; порядок областей — от границ продукта к
  документационному слою.
- Ссылки на решения даны по ID вида `ADR-NNNN`; действующий каталог решений —
  [spec/decisions/README.md](../decisions/README.md). Нормативный текст решения
  сюда не копируется, копируется только следствие, которое проверяется.
- Если правило нормировано записью решения, а проверки в репозитории нет, класс
  проверки — `manual` с честным описанием того, что именно проверяет человек.

## Как устроен реестр

Этот раздел описывает формат обоих реестров корпуса: реестра инвариантов
(`INV-*`, этот файл) и реестра требований к качеству (`REQ-*`,
[требования к качеству](quality-requirements.md)). Документ с требованиями
ссылается сюда и не повторяет формат.

Каждая запись оформлена одинаково. Заголовок записи —
`### <ID> — <короткое имя>`, где тире это U+2014, окружённое пробелами. Сразу
за заголовком идёт пустая строка и затем четыре поля-булета. Порядок ниже —
принятое оформление; тест проверяет наличие полей, а не их последовательность,
поэтому за порядком следит ревью:

- Поле `Rule` — ровно одно нормативное утверждение на русском, проверяемое
  кодом, тестом или ревью.
- Поле `Decision` — одна запись решения, список записей через запятую либо
  литерал `n/a`.
- Поле `Check` — одна или несколько строк; в каждой сначала класс проверки в
  обратных кавычках, затем тире U+2014, затем цель: для автоматической проверки
  это путь в обратных кавычках, для класса `manual` — свободное описание.
- Поле `Scope` — контуры, в которых правило обязано выполняться.

Имена полей (`Rule`, `Decision`, `Check`, `Scope`) остаются английскими: это
ключи, которые разбирает тест, а не проза. Внутри текста правила по-английски
остаются только идентификаторы — пути и имена файлов, имена инструментов, типов
и переменных окружения, ID записей и значения полей-перечислений.

Классы проверок:

| Класс | Что это | Что стоит в `<target>` |
| --- | --- | --- |
| `ci-test` | автоматический тест, исполняемый в CI (Python unittest или Rust `#[test]`) | путь к файлу с тестом |
| `guard-script` | скрипт-страж, исполняемый набором тестов или workflow | путь к скрипту |
| `doc-assert` | тест, который проверяет содержимое документации | путь к файлу с тестом |
| `release-gate` | шаг релизного конвейера, блокирующий допуск пакета в staging или перевод стабильного каталога | путь к скрипту или workflow |
| `manual` | ручная проверка при ревью | свободное описание |

Что доказывает тест реестра. `tests/ci/test_architecture_registry.py` проверяет
форму записи, уникальность ID, существование названного решения и то, что цель
неручной проверки — артефакт, который CI действительно исполняет: файл теста,
который собирает `unittest discover` или `cargo test --workspace` и в котором
объявлен хотя бы один тест; страж под `scripts/`, вызываемый workflow или
набором тестов; шаг релизного конвейера. Поэтому счёт автоматических проверок в
реестре — это счёт проверок, которые запускаются, а не список путей, у которых
совпало имя файла.

Чего он не доказывает: что названная проверка утверждает именно это правило.
Запись, чья `Rule` — выдумка, а `Check` указывает на настоящий исполняемый тест
из другой области, тест реестра пройдёт. Эту связь удерживает ревью, и опора у
него есть: `Rule` формулируется так, чтобы ревьюер мог запустить названную
проверку и получить вердикт по этому правилу, а не по соседнему. Проверка,
которая не падает при нарушении правила, проверкой не является, и запись с такой
целью на ревью отклоняется — как и `Верификация` записи решения, которая
пересказывает документ вместо того, чтобы называть падающую проверку.

Правила идентификаторов:

- ID соответствует `^(?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+$`. Префикс `INV`
  принадлежит инвариантам, префикс `REQ` — требованиям к качеству.
- ID уникален во всём корпусе спецификаций и никогда не переиспользуется после
  удаления записи: удалённый номер остаётся выведенным из обращения.
- Область фиксирует владельца правила, а не файл, в котором оно проверяется.
  У каждого реестра свой набор областей, и наборы не пересекаются: инварианты
  используют `PRODUCT`, `MCP`, `SKILL`, `APP`, `CACHE`, `SOURCE`, `PKG`,
  `PLATFORM`, `HOST`, `CI`, `DOC`; требования к качеству — `PERF`, `TOKEN`,
  `SAFETY`, `OBS`, `MAINT`, `COMPAT`, `REL`. Новая область заводится вместе с
  первой записью, которая ей принадлежит, и добавляется в этот перечень.
- `Scope` перечисляет контуры, в которых правило обязано выполняться:
  `source` (рабочее дерево), `packaged` (сгенерированный пакет), `ci`
  (конвейер), `release` (публикация), `runtime` (исполнение).

## PRODUCT — границы продукта

### INV-PRODUCT-SINGLE-PLUGIN-TREE — Один каталог плагина обслуживает двух хостов

- **Rule:** Unica поставляется как один каталог плагина, который обслуживает и
  Codex, и Claude Code; `.mcp.json`, `skills/`, справочники и граница MCP
  остаются нейтральными к хосту, и только каталоги манифестов `.codex-plugin/`
  и `.claude-plugin/` зависят от хоста.
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** source, packaged

### INV-PRODUCT-DEVELOPER-OPERATIONS — Публичная поверхность моделирует операции разработчика

- **Rule:** Публичные скиллы и инструменты `unica.*` моделируют операции
  разработчика 1С:Предприятия; вопросы инфраструктуры и упаковки в поверхность,
  которую видит модель, не попадают.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-NO-ENGINE-ROUTING — Встроенные движки не попадают в маршрутизацию, видимую модели

- **Rule:** Скиллы и справочники, которые видит модель, не должны предписывать
  ей вызывать встроенные низкоуровневые движки напрямую или называть их
  MCP-серверами; доменный инструмент можно упомянуть по смыслу, но никогда — как
  цель вызова.
- **Decision:** ADR-0001, ADR-0005, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-PACKAGE-PARITY — Сгенерированный пакет — полноценная поставка

- **Rule:** Каждый публичный контракт, который выполняется в исходном дереве,
  выполняется и в сгенерированном пакете для маркетплейса, а проверка на уровне
  пакета обязательна дополнительно к проверке на уровне исходников.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Scope:** packaged, release

### INV-PRODUCT-TOOL-VERSION-SOURCE — У версий встроенных инструментов один источник

- **Rule:** `plugins/unica/third-party/tools.lock.json` — источник версий
  встроенных инструментов, а запись о происхождении встроенного инструмента
  ссылается на него через `toolLockRef` вместо того, чтобы нести собственную
  версию или базовый коммит.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `tests/ci/test_skill_provenance.py`
- **Check:** `guard-script` — `scripts/ci/check-skill-upstreams.py`
- **Scope:** source, packaged, ci

### INV-PRODUCT-DCS-NAMING — DCS — каноническое имя домена компоновки данных

- **Rule:** Действующие английские идентификаторы домена компоновки данных
  используют `dcs`/`Dcs`/`DCS` в инструментах, скиллах, модулях Rust, метаданных
  пакета и действующей документации; удалённый транслитерированный псевдоним и
  написание аббревиатуры с переставленными буквами не должны появиться снова
  нигде, кроме явно разрешённых исключений — донорского дерева и схем
  платформы.
- **Decision:** ADR-0011
- **Check:** `ci-test` — `tests/ci/test_dcs_naming_contract.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-mcp.py`
- **Scope:** source, packaged, runtime, release

### INV-PRODUCT-NO-FORMAT-MIGRATION — Unica не мигрирует формат выгрузки

- **Rule:** Unica никогда не мигрирует и не понижает формат выгрузки как
  побочный эффект другой операции и не публикует нативной операции миграции
  формата; перенос более старого источника выполняет пользователь, явно загрузив
  его целевой платформой и повторно выгрузив, после чего повторяет вызов Unica.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime, source

## MCP — публичная MCP-поверхность

### INV-MCP-NO-ENGINE-SERVERS — `unica` — единственный MCP-сервер, видимый модели

- **Rule:** Внутренние движки (сборка и runtime, анализ BSL, индекс кода,
  стандарты, операции с XML и DSL) доступны только через внутренние адаптеры и
  никогда не регистрируются как отдельные публичные MCP-серверы.
- **Decision:** ADR-0001, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-MCP-SINGLE-ENTRY — Единственный публичный MCP-сервер

- **Rule:** `plugins/unica/.mcp.json` объявляет ровно одну запись `mcpServers`
  с именем `unica` — и в исходном дереве, и в любом сгенерированном пакете.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-MCP-SERVER-NAME — Имя сервера в протоколе

- **Rule:** `initialize` возвращает `serverInfo.name = "unica"`.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-MCP-NAMESPACE — Публичные инструменты живут в пространстве имён `unica.*`

- **Rule:** Публичный набор инструментов адресуется именами вида
  `unica.<group>.<operation>`, и упакованный runtime отдаёт под этим именем
  каждый обязательный инструмент `unica.*`, не отдавая удалённый псевдоним.
- **Decision:** ADR-0001, ADR-0011
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-mcp.py`
- **Scope:** runtime, packaged, release

### INV-MCP-DATA-DRIVEN-SCHEMA — Контракты инструментов заданы данными и свободны от адаптеров

- **Rule:** Имена и описания инструментов берутся из реестра `ToolSpec` в
  `application/mod.rs`, входные схемы — из `application/tool_contracts.rs`
  поверх `application/operation_descriptors.rs`, транспорт только собирает эти
  три источника вместе, обязательные пути публикуются в верхнем `required` под
  каноническими именами без алиасов, и ни одна публичная схема инструмента не
  показывает сырые аргументы адаптера.
- **Decision:** ADR-0001, ADR-0013, ADR-0019
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Scope:** runtime

### INV-MCP-SDK-TRANSPORT — Транспортом владеет официальный Rust SDK

- **Rule:** Публичный stdio-сервер — это реализация `rmcp::ServerHandler` в
  `interfaces/mcp.rs`, которая обслуживает `initialize`, `tools/list` и
  `tools/call` из реестра слоя application, причём и типы `rmcp`, и макросы
  инструментов из SDK не выходят за пределы этого модуля.
- **Decision:** ADR-0013, ADR-0002
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `manual` — ни один скрипт-страж не знает имени крейта, поэтому
  ревью подтверждает, что импорты `rmcp` и макросы инструментов из SDK остаются
  внутри `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-MCP-BOUNDED-ADMISSION — Приём вызовов ограничен, отмена кооперативна

- **Rule:** Одновременно допускается не более 32 обработчиков `tools/call`,
  лишние вызовы завершаются ошибкой JSON-RPC `-32603` со словом `overloaded`,
  каждый поставщик анализа кода удерживает не более 32 исполнителей, запрос,
  отменённый через `notifications/cancelled`, не получает ответа, а остановка
  транспорта отменяет ещё выполняющиеся доменные операции и исполнителей
  поставщиков за один общий ограниченный срок.
- **Decision:** ADR-0013, ADR-0017, ADR-0018
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/code_intelligence.rs`
- **Scope:** runtime

### INV-MCP-SURFACE-SYNC — Изменения публичной поверхности синхронны

- **Rule:** Добавление, удаление или переименование публичного MCP-инструмента
  меняет одним набором изменений реестр в Rust, стенд паритета, раздел `Решение`
  записи ADR-владельца, выведенное поле `Rule` записи реестра и названную в ней
  проверку; план приёмки может быть таким свидетельством проверки, но не заменой
  владельца.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Check:** `guard-script` — `scripts/ci/check-architecture-sync.py`
- **Check:** `ci-test` — `tests/ci/test_architecture_sync_guard.py`
- **Scope:** source, packaged

### INV-MCP-TYPED-RESULT — Результат инструмента типизирован, а не отрисован текстом

- **Rule:** Успешный вызов мигрировавшего публичного инструмента публикует
  результат только как `OperationResult.data`, текстового дубля результата в
  `stdout` не существует, и `stdout` остаётся лишь потоком вывода внешнего
  процесса 1С у `unica.build.*`, `unica.runtime.execute` и
  `unica.runtime.job.logs`.
- **Decision:** ADR-0020, ADR-0023
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_tool_surface_ledger.py`
- **Scope:** source, runtime

### INV-MCP-SOURCE-SURFACE — Ресурсная поверхность логична и ограничена

- **Rule:** Публичная группа источников содержит читающие
  `unica.source.resolve`, `unica.source.children`, `unica.source.locate`,
  `unica.source.resources` и `unica.source.read`; группа не содержит мутирующих
  инструментов, её схемы принимают логические цели и непрозрачные снимки, не
  принимают физический путь или закрытую ручку и удерживают объявленные
  пределы. Изменение BSL выполняет `unica.code.patch`.
- **Decision:** ADR-0021, ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-MCP-CODE-SEARCH-SECTIONS — Поиск сохраняет независимые секции поставщиков

- **Rule:** `unica.code.search` возвращает в фиксированном порядке секции
  `rlm`, `bsl-analyzer` и `git-grep`, не сравнивает их оценки и не скрывает
  отказ секции; результат успешен, когда хотя бы одна секция имеет состояние
  `ok` или `empty`, а отмена не возвращает частичный успех.
- **Decision:** ADR-0017
- **Check:** `ci-test` — `crates/unica-coder/src/application/code_intelligence.rs`
- **Check:** `ci-test` — `tests/ci/test_release_assessment.py`
- **Scope:** runtime, packaged

### INV-MCP-OUTLINE-DATA — Outline возвращает типизированные данные

- **Rule:** Успешный `unica.code.outline` публикует доказанную структуру модуля
  только как типизированный объект `data` общего конверта без `stdout`, вид
  метода имеет каноническое значение `procedure` или `function`, а каждый
  параметр представлен отдельными полями имени, передачи по значению и
  выражения по умолчанию вместо сырого текста объявления.
- **Decision:** ADR-0020
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/bsl_outline.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`
- **Scope:** source, runtime

## SKILL — маршрутизация скиллов

### INV-SKILL-DECLARED-ROUTING — Скиллы маршрутизируются через MCP `unica`

- **Rule:** Каждый скилл, на который распространяется правило, документирует
  свою маршрутизацию через MCP `unica` и называет инструмент `unica.*`, который
  вызывает.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-NO-ADAPTER-TARGETS — Скиллы не называют внутренние серверы-адаптеры

- **Rule:** Скиллы и справочники, которые видит модель, не должны называть
  внутренние MCP-серверы адаптеров или их идентификаторы инструментов как цели
  маршрутизации.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-NO-SCRIPT-ROUTE — Локальные для скилла скрипты операций не возвращаются

- **Rule:** Скиллы не должны поставлять или упоминать локальные для скилла файлы
  операций на Python, PowerShell или shell как путь исполнения; переход на
  нативные обработчики `unica.*` завершён, и возвращение такого пути требует
  решения, заменяющего действующее.
- **Decision:** ADR-0004, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-SKILL-SCRIPTS-AS-FIXTURES — Эталонные модели существуют только как тестовые фикстуры

- **Rule:** Адаптированные скрипты операций существуют только как принадлежащие
  Unica эталонные модели в
  `tests/fixtures/unica_mcp_script_parity/unica_reference_models`,
  отревьюированный снимок донора — только в
  `tests/fixtures/unica_mcp_script_parity/cc-1c-skills`, и ни одно из этих
  деревьев не попадает в пакет и не доступно во время исполнения.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-SKILL-DOCUMENTED-PREVIEW — Изменяющие инструкции по умолчанию ведут через предпросмотр

- **Rule:** Инструкции скиллов держат путь предпросмотра на виду на
  разрушительных и неполных маршрутах: скилл `meta-remove` документирует вызов
  с `"dryRun": true`, а каждая документированная инкрементальная, частичная или
  относящаяся к внешнему набору исходников выгрузка записана как
  вызов-предпросмотр.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-SOURCE-FALLBACK — Ресурсная запись остаётся запасным маршрутом

- **Rule:** Скилл ресурсного доступа сначала выбирает существующий предметный
  инструмент записи, использует `unica.source.resources` и `unica.source.read`
  для исследования, а изменение BSL вносит через `unica.code.patch` с
  предпросмотром до применения; ресурсная группа мутирующих инструментов не
  содержит.
- **Decision:** ADR-0022
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-EXECUTABLE-EXAMPLES — Примеры в скиллах — исполнимые вызовы MCP

- **Rule:** Каждый пример `tools/call` в скилле — настоящий параметризованный
  вызов, который успешно исполняется как сухой прогон MCP.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source, packaged

### INV-SKILL-REACHABLE-REFERENCES — Справочный документ поставки назван скиллом

- **Rule:** Каждый документ каталога `plugins/unica/references`, попадающий в
  поставку, назван хотя бы одним `SKILL.md` напрямую либо достижим от
  названного по цепочке ссылок между справочными документами; непокрытый на
  сегодня остаток перечислен поимённо списком долга в проверке, и этот список
  может только сокращаться.
- **Decision:** n/a
- **Check:** `ci-test` — `tests/ci/test_reference_reachability.py`
- **Scope:** source, packaged

## APP — границы слоёв приложения

### INV-APP-DISPATCH-OWNERSHIP — Слой application владеет диспетчеризацией и доменными событиями

- **Rule:** `UnicaApplication` владеет публичным реестром инструментов,
  диспетчеризацией вызовов и порождением доменных событий; новый обработчик
  инструмента входит в систему через диспетчеризацию application и никак иначе.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** source, runtime

### INV-APP-THIN-TRANSPORT — Транспорт только отображает протокол на вызовы application

- **Rule:** `interfaces::mcp` обслуживает `tools/list` из
  `UnicaApplication::tools()`, направляет каждый `tools/call` через
  `call_tool_cancellable` и возвращает как текст инструмента конверт результата,
  собранный слоем application, а не собственную структуру.
- **Decision:** ADR-0002, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-APP-NO-ADAPTER-BYPASS — Адаптеры идут к рабочему пространству через порты application

- **Rule:** Адаптеры инфраструктуры обращаются к состоянию рабочего
  пространства через `ApplicationPorts` и никогда не импортируют слой
  interfaces, поэтому адаптер не может отрисовать ответ MCP и по дороге наружу
  обойти отчёт о кеше, который ведёт слой application.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, runtime

### INV-APP-NO-SCRIPT-BACKEND — В runtime нет скриптового бэкенда

- **Rule:** В `unica-coder` нет отката на файлы операций во время исполнения: ни
  унаследованного обработчика скриптов, ни запуска `python`, `python3`, `bash`,
  `powershell` или `pwsh` из продуктивного кода.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, runtime

### INV-APP-DEPENDENCY-DIRECTION — Направление зависимостей между слоями закреплено проверкой

- **Rule:** `domain` не импортирует ни `application`, ни `infrastructure`, ни
  `interfaces` и не обращается к файловой системе и процессам, а `application`
  не импортирует ни `infrastructure`, ни `interfaces`.
- **Decision:** ADR-0009, ADR-0002
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-APP-NO-DIRECT-GIT — Application не запускает git напрямую

- **Rule:** Продуктивный код в `crates/unica-coder/src/application` никогда не
  создаёт дочерний процесс `git`; состояние git читается через инфраструктуру.
- **Decision:** ADR-0002, ADR-0009
- **Check:** `ci-test` — `tests/ci/test_product_contracts.py`
- **Scope:** source

### INV-APP-CODE-PROVIDER-BOUNDARY — Анализ кода не зависит от движка

- **Rule:** Слой application оркестрирует поиск и навигацию только через
  типизированные `CodeIntelligenceProvider` и `CodeIntelligenceContext`,
  разрешает корень исходников один раз и не знает команд процессов, транспортов
  и форматов частного хранилища поставщика; эти детали принадлежат
  инфраструктурной реализации.
- **Decision:** ADR-0017
- **Check:** `ci-test` — `crates/unica-coder/src/application/code_intelligence.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- **Scope:** source, runtime

### INV-APP-OUTLINE-SOURCE — Структура модуля берётся из текущего файла

- **Rule:** `unica.code.outline` строит результат из BSL-файла, лежащего в
  выбранном корне исходников на момент вызова: он не читает снимок `bsl_index`,
  не проверяет готовность индекса, не запускает его скрытый сервис и не меняет
  состояние рабочего пространства, а недоказуемая структура завершает вызов
  отказом вместо частичного дерева.
- **Decision:** ADR-0020
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/bsl_outline.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`
- **Scope:** source, runtime

### INV-APP-LAZY-HIDDEN-SERVICES — Внутренние сервисы скрыты и привязаны к рабочему пространству

- **Rule:** Тёплые транспорты и сессии поставщиков живут в скрытом сервисе с
  ключом из корня рабочего пространства и корня исходников; сервис запускается
  только настоящей операцией поставщика, тогда как `initialize`, `tools/list`,
  `unica.project.status`, `unica.project.map` и предпросмотр его не запускают,
  и сервис никогда не становится публичной регистрацией MCP.
- **Decision:** ADR-0018
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## CACHE — состояние рабочего пространства и кеш

### INV-CACHE-ORCHESTRATOR-OWNED — Состоянием рабочего пространства владеет оркестратор

- **Rule:** Оркестратор `unica` владеет состоянием рабочего пространства и
  логической инвалидацией по доменным событиям, а поставщик владеет реализацией
  жизненного цикла своего индекса, процесса и сессии; модель не согласовывает
  свежесть между движками, и оркестратор не читает частное хранилище поставщика.
- **Decision:** ADR-0003, ADR-0001, ADR-0018
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Check:** `ci-test` — `tests/ci/test_product_contracts.py`
- **Scope:** runtime

### INV-CACHE-REPORTED-EFFECTS — Изменяющие операции порождают типизированные доменные события

- **Rule:** Каждая изменяющая операция порождает типизированные доменные
  события, и эти события отображаются на имена инвалидированных и обновлённых
  кешей, о которых сообщается вызывающему.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### INV-CACHE-WORKSPACE-ROOT — Корень изменчивого кеша можно переопределить

- **Rule:** Корень изменчивого кеша по умолчанию равен
  `<workspaceRoot>/.build/unica` и переопределяется переменной
  `UNICA_CACHE_DIR`, а записи о скрытых сервисах рабочего пространства пишутся
  под тем корнем, который действует.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

### INV-CACHE-WRITE-FREE-PREVIEW — Сухой прогон сообщает о последствиях, не записывая состояние

- **Rule:** Вызов в режиме сухого прогона сообщает о своём влиянии на кеш и не
  пишет ни состояние рабочего пространства, ни индекс, ни запись о сервисе.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
- **Scope:** runtime

### INV-CACHE-PERSISTED-STALENESS — Применённое изменение запоминает инвалидированный им кеш

- **Rule:** Применённое изменение отображает свои доменные события в
  `CacheImpact` и записывает эту проекцию через `WorkspaceStateRepository`,
  поэтому кеш, который оно инвалидировало, при следующем чтении по-прежнему
  числится устаревшим, а не оказывается молча пересобранным; хранилище не
  является журналом полного содержимого событий. Публикация состояния
  использует тот же механизм точного исходного образа и атомарной замены,
  поэтому конкурентный план либо сохраняет объединённый эффект после повторного
  планирования, либо явно отказывает, но не затирает чужую инвалидацию.
- **Decision:** ADR-0003, ADR-0018, ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Scope:** runtime

### INV-CACHE-WORKTREE-ISOLATION — Связанное рабочее дерево git изолировано

- **Rule:** Идентичность рабочего пространства, его эпоха, корни кеша и ключи
  внутренних сервисов, индексов и сессий выводятся так, что связанное рабочее
  дерево git изолировано и от основной рабочей копии, и от любого другого
  рабочего дерева, а код, читающий состояние git, разрешает `.git` и как
  каталог, и как файл-указатель.
- **Decision:** ADR-0003, ADR-0018
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`
- **Scope:** runtime

### INV-CACHE-RUNTIME-ROOT-ORDER — Разрешение корня кеша runtime детерминировано

- **Rule:** `unica-bootstrap` разрешает корень кеша runtime в фиксированном
  порядке — `UNICA_RUNTIME_CACHE_DIR` берётся как есть, если в нём не осталось
  неразвёрнутой подстроки `${`, затем `<CLAUDE_PLUGIN_DATA>/runtimes`, затем
  `<CODEX_HOME>/unica/runtimes`, затем `<HOME или USERPROFILE>/.codex/unica/runtimes`,
  а когда не задано ни одно из значений, завершается ошибкой — и публикует
  проверенный runtime атомарно под `<cacheRoot>/<pluginVersion>/<target>`.
- **Decision:** ADR-0008, ADR-0012, ADR-0014
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/runtime_cache.rs`
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, runtime

## SOURCE — наборы исходников рабочего пространства

### INV-SOURCE-PER-SET-FORMAT — Формат — свойство набора исходников

- **Rule:** `unica.project.map` сообщает `sourceSets[]`, и каждая запись несёт
  собственный `sourceFormat`, потому что формат исходников — свойство
  отдельного набора, а не всего рабочего пространства.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-UNAMBIGUOUS-SET — Один набор исходников не бывает двух форматов сразу

- **Rule:** Противоречащие друг другу признаки формата внутри одного набора
  исходников делают его недопустимым или неоднозначным; набор никогда не
  сообщает смешанный формат.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Scope:** runtime

### INV-SOURCE-MULTI-FORMAT-WORKSPACE — В рабочем пространстве может действовать несколько форматов

- **Rule:** Одно рабочее пространство может содержать несколько наборов
  исходников с разными действующими форматами — например, конфигурацию в формате
  EDT рядом с внешними обработками и отчётами в формате platform XML.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-PLATFORM-XML-ONLY — Нативные операции с XML требуют формата platform XML

- **Rule:** Нативная операция над метаданными в формате platform XML сначала
  разрешает набор исходников, у которого `sourceFormat` равен `platform_xml`, и
  лишь затем трогает XML-файлы; если разрешённый набор оказался в формате EDT,
  недопустимым или неоднозначным, операция отклоняется типизированной ошибкой.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-SINGLE-RESOLVED-ROOT — Выбор корня исходников детерминирован и общий

- **Rule:** Непустой `sourceDir` разрешается относительно рабочего каталога
  запроса, иначе побеждает набор исходников с именем `main`, а за ним —
  единственный набор исходников конфигурации; разрешённый корень нормализуется,
  остаётся внутри рабочего пространства и служит тем же корнем для анализатора,
  индекса, идентичности сервиса, `unica.project.status` и `unica.project.map`.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-LOGICAL-IDENTITY — Точная цель не зависит от файловой раскладки

- **Rule:** Точная существующая цель задаётся именем `sourceSet` и
  необязательным каноническим `metadataPath`: английские и русские виды
  нормализуются в английские токены, прикладные имена сохраняются, а физический
  путь не принимается и не возвращается как идентичность цели.
- **Decision:** ADR-0021
- **Check:** `ci-test` — `crates/unica-coder/src/domain/source_target.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- **Scope:** source, runtime

### INV-SOURCE-WRITE-TARGET-KIND — Писатель принимает только терминал модуля

- **Rule:** Разрешение цели выполняется под явной политикой вида: пишущая
  операция запрашивает только терминал модуля и отклоняет адрес объекта
  метаданных стабильным `TargetKindMismatch`, закрытая ручка несёт вид, под
  которым выдана, и повторная проверка выполняется под той же политикой,
  поэтому расширение резолвера на новый вид цели не расширяет право записи.
- **Decision:** ADR-0021
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- **Scope:** source, runtime

### INV-SOURCE-SNAPSHOT-BINDING — Ресурс действует только внутри своего снимка

- **Rule:** Непрозрачные `snapshotId` и `resourceId` связаны с экземпляром
  приложения, рабочим пространством, поставщиком, набором исходников, целью,
  областью, ревизией и сроком действия; ресурс из другого, истёкшего или
  подделанного снимка не читается и не записывается.
- **Decision:** ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-SOURCE-ROLE-ALLOWLIST — Право записи выдаётся по доказанной роли

- **Rule:** Первый ресурсный writer заменяет ровно один существующий
  `bslModule` только из полного снимка с возможностью `replace`;
  дескрипторы, регистрации, формы, DCS, MXL, права, двоичные и неизвестные роли
  остаются доступными только для чтения независимо от типа содержимого.
- **Decision:** ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/domain/source_resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Scope:** runtime

### INV-SOURCE-OBSERVED-EOL — Перевод строки наблюдается в источнике, а не назначается

- **Rule:** Снимок исходного текста классифицирует переводы строк как `None`
  (ни одного), `Uniform` (единственный вид — LF, CRLF или одиночный CR) или
  `Mixed` с точным счётчиком каждого вида и отдельно запоминает завершающий
  перевод строки; политика `Preserve` берёт локальный перевод строки, при его
  отсутствии — единый профиль источника, а на смешанном профиле и на источнике
  вовсе без переводов строк отказывает; политики `Lf` и `CrLf` профиль
  игнорируют, политика `Repository` пока не разрешается никогда; источник без
  единого перевода строки writer обслуживает явной политикой `Lf`, а источник с
  одиночными CR — отказом `unica.code.patch`, поэтому глобальной нормализации
  переводов строк не происходит ни при каком исходе.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Scope:** runtime

### INV-SOURCE-ATOMIC-PUBLISH — Мутация источника публикуется атомарно после проверки

- **Rule:** Изменяющая операция сначала собирает точный образ файла после записи
  и проверяет его целиком в памяти — включая повторный разбор и применение
  собственного diff, результат которого обязан побайтно совпасть с образом, — и
  только затем публикует его через промежуточный файл и атомарную замену;
  провал проверки, занятый путь промежуточного файла и любая ошибка публикации
  оставляют исходные байты нетронутыми.
- **Decision:** ADR-0021, ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
- **Scope:** runtime

### INV-SOURCE-IDEMPOTENT-REWRITE — Повторная идентичная мутация ничего не пишет

- **Rule:** Повторный идентичный вызов изменяющей операции распознаётся до
  записи как семантически пустой: хеш до совпадает с хешем после, diff и
  диапазоны пусты, ни файл, ни состояние кеша не меняются и доменное событие не
  публикуется; первый вызов отклоняется без записи, если его образ после записи
  не позволяет доказать эту пустоту при следующем идентичном вызове. Замена,
  поглотившая собственный селектор, удовлетворяет правилу иначе: повторный вызов
  не находит цель и отказывает, ничего не записав, поэтому второго применения не
  происходит.
- **Decision:** ADR-0021, ADR-0022
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Scope:** runtime

### INV-SOURCE-WRITE-CONTAINMENT — Запись не выходит за корень рабочего пространства

- **Rule:** Путь, в который инструмент собирается писать, проходит через
  `WorkspacePathPolicy::resolve_write`: относительный путь разрешается от
  рабочего каталога запроса, `.` и `..` сворачиваются лексически, результат
  обязан остаться под корнем рабочего пространства, а ближайший существующий
  предок дополнительно канонизируется и тоже обязан остаться под ним, поэтому
  и лексический выход за корень, и выход через символическую ссылку отклоняются
  до записи первого байта.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/path_policy.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-WRITABLE-FORMAT — Записывается только действующий профиль выгрузки

- **Rule:** Нативная операция пишет платформенный XML только в действующем
  профиле — платформа `8.3.27`, формат выгрузки `2.20`; способность платформы
  `8.3.27` импортировать формат ниже `2.20` не делает такой формат записываемым,
  поэтому набор исходников более старого формата отклоняется, а не переписывается
  под действующий профиль.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/domain/format_profile.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Scope:** runtime, source

### INV-SOURCE-OWNER-VERSION-GATE — Версию решает корень-владелец, отказ наступает до первой записи

- **Rule:** Формат выгрузки разрешается от XML-корня, владеющего версией, для
  выбранного набора исходников, а подчинённый XML без собственной версии
  наследует этого владельца; формат ниже `2.20` — включая существующий
  корень-владелец без атрибута `version`, который означает `1.0`, —
  предупреждает при чтении там, где чтение безопасно, и отказывает до первой
  записи, предлагая явно перенести источник платформой `8.3.27`; формат выше
  `2.20` предупреждает и отказывает до первой записи, никогда не предлагая
  понижение; недопустимое, нечитаемое или неоднозначное свидетельство версии, а
  равно отсутствующий или неразрешимый владелец, отказывают до первой записи и
  не подменяются значением `1.0`; существующий распознанный корень DCS или MXL
  без версии обслуживается по фиксированному профилю без выдумывания версии.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- **Scope:** runtime, source

### INV-SOURCE-EXACT-VERSION-LITERAL — Поддерживаемая версия — точный литерал, а не численное равенство

- **Rule:** Поддерживаемым считается только точный сырой лексический срез
  атрибута `version` до декодирования XML-сущностей, равный `2.20`; численно
  равные иные написания (`2.20.0`, `02.20`, `2.020`) и написания, записанные
  сущностями (`2.&#50;0`, `&#x32;.20`, `2.2&#48;`), отклоняются как недопустимое
  свидетельство версии, а сравнение числовых компонент только классифицирует
  значение как более старое или более новое и не канонизирует альтернативное
  написание поддерживаемого.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/domain/format_profile.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime, source

### INV-SOURCE-EXACT-ROOT-QNAME — Цель записи опознаётся по точному QName корня

- **Rule:** Объявленные операцией существующие цели управляемой формы, DCS и MXL
  проверяются по точному QName корневого элемента до записи, включая пути с
  расширением `.XML` и пути без расширения; корректные документы DCS и MXL без
  версии и по-настоящему отсутствующие выходные файлы остаются допустимыми,
  потому что такие корни версией выгрузки не владеют.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Scope:** runtime

### INV-SOURCE-BOUND-PREIMAGES — Мутация привязана к байтам, из которых выведена

- **Rule:** Для изменяющего вызова публичная предпроверка повторяется внутри
  обработчика по фактическим зависимостям XML, байты, из которых выведена
  мутация, привязываются к транзакции компиляции как точные преобразы, а
  сотрудничающие пишущие операции Unica берут одни и те же кооперативные
  блокировки публикации, поэтому изменение отклоняется, если наблюдённые байты
  разошлись между планированием и публикацией.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Scope:** runtime

### INV-SOURCE-ROLLBACK-VISIBLE — Неудавшийся откат виден как ошибка целостности

- **Rule:** Неудача восстановления или удаления уже опубликованного пути
  исходников — жёсткая ошибка транзакции: результат несёт диагностику
  `rollback encountered:`, называет затронутые и сохранённые пути восстановления
  и требует от вызывающего считать целостность дерева исходников непроверенной;
  уровня предупреждения `cleanup encountered:` достигает только неудалённый
  временный, карантинный или уже восстановленный остаток.
- **Decision:** ADR-0016
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Scope:** runtime

## PKG — упаковка и поставка

### INV-PKG-UNTRACKED-BUILD-OUTPUT — Собранные бинарники не попадают под контроль версий

- **Rule:** Собранные бинарники и прочие генерируемые пути пакета никогда не
  отслеживаются в исходном дереве, а упаковка завершается ошибкой, если
  отслеживаемый файл оказался внутри генерируемого пути или является
  символической ссылкой.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-PKG-THIN-PACKAGE — Публичный пакет маркетплейса тонкий

- **Rule:** Опубликованный пакет несёт только файлы плагина и три небольших
  бинарника bootstrap; его `.mcp.json` запускает runtime через ограниченный
  командой shell-алиас Git, который определяет корень плагина для обоих хостов и
  передаёт его в `bootstrap/launch.sh`, и пакет никогда не зависит ни от полного
  бинарника runtime, ни от матрицы команд под каждую целевую платформу.
- **Decision:** ADR-0008, ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, release

### INV-PKG-VERIFIED-ATOMIC-INSTALL — Получение runtime проверяется контрольной суммой и атомарно

- **Rule:** Bootstrap скачивает закреплённый runtime своего хоста, сверяет
  SHA-256 архива с метаданными релиза и каждый извлечённый файл — с записанной
  для него контрольной суммой, и только после этого публикует runtime атомарно;
  повреждённый архив и архив с выходом за пределы каталога распаковки никогда не
  становятся готовым runtime.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_runtime.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** packaged, release, runtime

### INV-PKG-BINARY-NAME — Публичный бинарник runtime называется `unica`

- **Rule:** Встроенный публичный бинарник, собираемый из Cargo-воркспейса,
  называется `unica` и записан под этим именем в
  `plugins/unica/third-party/tools.lock.json`.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Scope:** source, packaged

### INV-PKG-VERSION-LOCKSTEP — Оба манифеста хостов несут одну версию

- **Rule:** `plugins/unica/.codex-plugin/plugin.json` и
  `plugins/unica/.claude-plugin/plugin.json` оба существуют и объявляют ту же
  версию, что Cargo-воркспейс и запись `unica` в `tools.lock.json`; манифест
  Claude не объявляет ни `skills`, ни `mcpServers`, потому что и то и другое
  обнаруживается по умолчанию.
- **Decision:** ADR-0012
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_version_contract.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/plugin_manifest.rs`
- **Scope:** source, packaged

### INV-PKG-OLDEST-CLIENT-KEYS — Манифесты и каталоги не выходят за нижнюю границу клиента

- **Rule:** Манифесты хостов и записи каталогов используют только те ключи,
  которые принимает самый старый поддерживаемый клиент, а оба каталога хостов
  закрепляют один и тот же неизменяемый тег релиза с типом источника,
  адресующим подкаталог.
- **Decision:** ADR-0012, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** packaged, release

### INV-PKG-DEV-ONLY-PACKAGE — Локальная отладочная упаковка существует только для разработки

- **Rule:** Локальный отладочный пакет запускает бинарник `bin/<target>/unica`
  (`unica.exe` на `win-x64`) для текущего хоста напрямую, а не через полезную
  нагрузку bootstrap — по относительному пути с `cwd` в Codex и через
  `${CLAUDE_PLUGIN_ROOT}` без `cwd` в Claude Code, — собирается только под
  текущую целевую платформу и регистрирует свой каталог Codex под именем
  `unica-dev`, чтобы этот каталог нельзя было принять за опубликованный.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source

### INV-PKG-NO-INTERNAL-MATERIAL — Внутренние материалы сопровождения не уезжают в поставку

- **Rule:** Пакет плагина несёт только то, что нужно потребителю в момент работы:
  записи о происхождении апстримов, датированные записи ревью и внутренняя
  документация об устройстве пакета и конвейера живут вне `plugins/unica/` и в
  собранный плагин не попадают.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-PKG-ATTRIBUTION-COVERAGE — Атрибуция остаётся полной и доступной

- **Rule:** У каждого встроенного инструмента, адаптированного источника скилла
  и упакованного стороннего ресурса есть запись об атрибуции, а страница
  атрибуции связана ссылкой и из репозитория, и из README в пакете.
- **Decision:** n/a
- **Check:** `guard-script` — `scripts/ci/check-attributions.py`
- **Check:** `ci-test` — `tests/ci/test_attributions.py`
- **Scope:** source, packaged

## PLATFORM — платформенный фасад

### INV-PLATFORM-OS-BEHIND-FACADE — Зависящий от ОС код живёт за платформенными фасадами

- **Rule:** Зависящий от ОС продуктивный код существует только под
  `crates/unica-coder/src/infrastructure/platform/**` и
  `crates/unica-bootstrap/src/platform/**`; поведение файловой системы, путей,
  процессов и точек входа попадает в остальной код через эти фасады в виде
  платформенно-нейтральных типов.
- **Decision:** ADR-0009
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-PLATFORM-NO-PATH-EXEMPTIONS — У платформенного стража нет исключений по путям

- **Rule:** Платформенный страж допускает зависящий от ОС код только по
  структурному расположению — два префикса платформенных фасадов и вложенные
  каталоги `tests/platform/**` — и не несёт ни одного унаследованного исключения
  для конкретного пути.
- **Decision:** ADR-0009
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Check:** `manual` — тесты проверяют структурные правила на образцах путей,
  но нового исключения не видят, поэтому каждое изменение `_is_platform_facade`
  и `_is_platform_test` в `scripts/ci/check-rust-platform-boundary.py` ревью
  проверяет на буквальный унаследованный путь до слияния
- **Scope:** source

### INV-PLATFORM-COLOCATED-TESTS — Платформенные тесты лежат рядом со своими адаптерами

- **Rule:** Зависящие от платформы тесты лежат рядом со своими адаптерами или
  под `crates/<crate>/tests/platform/**`, но никогда — как платформенный
  тестовый файл верхнего уровня.
- **Decision:** ADR-0009
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, ci

### INV-PLATFORM-NO-ORPHAN-PROCESSES — Дочерние процессы удерживаются целыми деревьями

- **Rule:** Дочерние процессы анализатора, индекса и runtime удерживаются
  целыми деревьями — Job Object с завершением по закрытию на Windows и отдельная
  группа процессов на Unix, — поэтому отмена, тайм-аут, остановка или отказ
  сессии завершают всё дерево за ограниченное время ожидания.
- **Decision:** ADR-0006, ADR-0009
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform/process.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## HOST — host-фасад

### INV-HOST-NEUTRAL-ORCHESTRATOR — Оркестратор нейтрален к хосту

- **Rule:** `crates/unica-coder/src/**` не содержит ни одного host-маркера —
  ни имени хоста, ни каталога манифеста `.codex-plugin` или `.claude-plugin`, ни
  переменных окружения `CODEX_HOME`, `CLAUDE_PLUGIN_DATA` и
  `CLAUDE_PLUGIN_ROOT`, — поэтому домен, приложение, инфраструктура и
  интерфейсный слой не знают, какой хост запустил процесс.
- **Decision:** ADR-0014, ADR-0012
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-HOST-KNOWLEDGE-BEHIND-FACADE — Знание о хосте живёт за host-фасадом

- **Rule:** Host-специфичное продуктивное поведение существует только под
  `crates/unica-bootstrap/src/host/**`, а host-специфичные тесты — дополнительно
  под `crates/<crate>/tests/host/**`; в остальной код это поведение попадает
  через host-нейтральные типы фасада, и host-нейтральный override
  `UNICA_RUNTIME_CACHE_DIR` остаётся вне описаний конкретных хостов.
- **Decision:** ADR-0014
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/runtime_cache.rs`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/plugin_manifest.rs`
- **Scope:** source, runtime

### INV-HOST-UNIFORM-CALL-SITES — Добавление хоста не меняет мест вызова

- **Rule:** Хост описан дескриптором-данными, поэтому поддержка нового хоста
  добавляется дескриптором внутри `crates/unica-bootstrap/src/host/**`, а места
  вызова перебирают весь реестр дескрипторов и не ветвятся по конкретному хосту.
- **Decision:** ADR-0014
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/plugin_manifest.rs`
- **Scope:** source

## CI — сборка, артефакты и релизный конвейер

### INV-CI-MANDATORY-BUILD — Одна закреплённая сборка Cargo на платформенный раннер

- **Rule:** Каждый платформенный раннер собирает `unica` и `unica-bootstrap`
  одним обязательным вызовом `cargo build --locked` в отдельный для целевой
  платформы каталог сборки Cargo; восстановленный кеш эту команду ускоряет, но
  никогда не заменяет.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-EXACT-CACHE-KEYS — Попадания в кеш Cargo точны и наблюдаемы

- **Rule:** Ключ кеша Cargo содержит ОС раннера, целевую платформу Unica,
  разрешённый ключ тулчейна и хеш `Cargo.lock`, префиксные ключи восстановления
  не используются, а каждая платформенная сборка сообщает свою целевую
  платформу, исход обращения к кешу и длительность сборки.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-NARROW-ARTIFACTS — Артефакты узкие, типизированные и недолговечные

- **Rule:** Каталоги сборки Cargo никогда не выгружаются; между задачами данные
  переходят только как метаданные runtime, полезная нагрузка bootstrap и архивы
  runtime со сроком хранения в одни сутки, тогда как тонкая полезная нагрузка
  для маркетплейса сохраняет более длительный срок хранения для ручного
  размещения и продвижения.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-SELF-VERIFIED-ARCHIVE — Каждая платформа проверяет то, что собрала

- **Rule:** Платформенный раннер упаковывает свой архив runtime и сверяет с его
  метаданными контрольную сумму архива, состав файлов, контрольные суммы
  элементов, режимы исполнения и обнулённые отметки времени до того, как архив
  будет выгружен или отброшен; при публикации по тегу проверка повторяется на
  скачанных опубликованных байтах.
- **Decision:** ADR-0010, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### INV-CI-TAG-ONLY-PUBLISH — Публикация происходит только по тегу

- **Rule:** Артефакты релиза публикуются только при push тега; прогоны для
  pull request и ручные прогоны собирают пакет и прогоняют дымовые проверки без
  публикации, а размещение и продвижение каталога остаются отдельными явными
  задачами.
- **Decision:** ADR-0008, ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-SINGLE-GATE — Каждый pull request закрывает один агрегирующий шлюз

- **Rule:** Каждый pull request решается единственным стабильным агрегирующим
  шлюзом, который вместе оценивает задачи по исходникам, по Rust, по упаковке,
  по bootstrap, по оценке релиза и по опубликованным артефактам.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `ci-test` — `tests/ci/test_evaluate_ci_gate.py`
- **Scope:** ci

## DOC — документационный слой

### INV-DOC-REGISTRY-ENTRY-FORMAT — Записи реестра оформлены каноническим форматом

- **Rule:** Каждая запись реестра несёт заголовок `### <ID> — <короткое имя>`,
  ровно одно поле `Rule`, поле `Decision`, хотя бы одно поле `Check` и поле
  `Scope`, причём класс проверки в `Check` взят из `ci-test`, `guard-script`,
  `doc-assert`, `release-gate` или `manual`, а значения `Scope` — из `source`,
  `packaged`, `ci`, `release` или `runtime`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-NO-ID-REUSE — ID реестра уникальны и не переиспользуются

- **Rule:** Каждый ID реестра соответствует
  `^(?:INV|REQ)-[A-Z]+(?:-[A-Z]+)+$`, уникален во всём корпусе спецификаций
  и никогда не назначается другому правилу после того, как исходная запись
  удалена.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-REAL-CHECKS — Каждый инвариант называет настоящую проверку

- **Rule:** Каждая запись реестра называет хотя бы одну проверку, и каждая
  проверка класса, отличного от `manual`, указывает на артефакт, который
  исполняет CI: собираемый файл теста, объявляющий хотя бы один тест, страж под
  `scripts/`, вызываемый workflow или набором тестов, либо шаг релизного
  конвейера; существования файла по названному пути недостаточно.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-INDEX-SYNC — Индексы синхронны со своими документами

- **Rule:** Каждая принятая запись решения перечислена в
  `spec/decisions/README.md`, каждая перечисленная запись существует на диске, и
  каждый документ каталогов `spec/architecture/` и `spec/acceptance/`
  перечислен в `spec/README.md`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-ARCHIVE-NOT-NORMATIVE — Исторические документы помечены как исторические

- **Rule:** Индекс каждого архивного дерева — `docs/design` и `docs/plans` —
  несёт архивную пометку, а текст архивного документа не владеет действующим
  архитектурным правилом; закрепление файла CI-тестом делает его живым тестовым
  входом, но не нормативной архитектурной документацией.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-RELATIVE-LINKS — Относительные ссылки разрешаются от своего документа

- **Rule:** Каждая относительная markdown-ссылка в действующем слое документации
  разрешается от каталога того документа, который её несёт, поэтому читателю не
  нужен корень репозитория, чтобы по ней перейти.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-DOC-RUSSIAN-NORMATIVE — Нормативный текст пишется по-русски

- **Rule:** Нормативные формулировки поля `Rule` каждого реестра и раздела
  `Решение` каждой новой записи ADR пишутся по-русски; по-английски внутри них
  остаются только идентификаторы — пути и имена файлов, инструментов, типов,
  функций и переменных окружения, ID записей и значения перечислений, а
  исторические смешанные ADR перечислены как исключения в индексе решений.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-SINGLE-RULE-OWNER — У нормативного текста один владелец

- **Rule:** Архитектурным выбором владеет раздел `Решение` одной записи ADR, а
  выведенным проверяемым обязательством — поле `Rule` одной записи реестра;
  остальные документы ссылаются на владельца по ID, их пояснение применения не
  становится вторым владельцем, и ни один документ не воспроизводит каталог
  решений как второй индекс.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `manual` — автоматизирована только та половина правила, что
  запрещает второй индекс, поэтому архитектурное ревью проверяет ссылку на
  владельца и отклоняет пояснение, которое выдаёт себя за независимую норму
- **Scope:** source

### INV-DOC-SUPERSEDE-NOT-EDIT — Принятое решение не переписывают

- **Rule:** Запись решения становится неизменяемой историей в момент попадания в
  целевую ветку изменения — обычный поток целится в `main`, и ветка задаётся
  проверке явно: содержание и поле `Дата` такой записи не переписываются, вместо
  правки заводится новая запись, прежняя получает статус `superseded` и называет
  заменяющую разрешимой ссылкой, редакционная правка её текста отмечается полем
  `Обновлено`, а её номер повторно не выдаётся. Запись, которой в целевой ветке
  ещё нет, внутри своего pull request правится, перенумеровывается, объединяется
  и удаляется свободно, но статуса `superseded` не получает и не занимает номер,
  уже израсходованный в целевой ветке. Запись, недоступную стражу для чтения —
  бинарный рендер содержимого или путь, который он не разобрал, — считают
  непроверенной, а не чистой.
- **Decision:** n/a
- **Check:** `guard-script` — `scripts/ci/check-architecture-sync.py`
- **Check:** `ci-test` — `tests/ci/test_architecture_sync_guard.py`
- **Scope:** source

## Выведенные из обращения идентификаторы

Идентификатор, у которого удалена запись, попадает сюда и больше никогда не
выдаётся другому правилу. Иначе ссылка из старого PR или из чужого конспекта
однажды укажет на правило, которого автор ссылки не имел в виду.

Единственное допустимое содержимое раздела — строки вида «идентификатор, дата
вывода, причина». Любой идентификатор, названный здесь, считается выведенным,
поэтому примеры в прозе неуместны: их подхватит
`tests/ci/test_architecture_registry.py`.

Выведенных из обращения идентификаторов пока нет.
