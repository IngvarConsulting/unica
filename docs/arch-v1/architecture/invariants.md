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
- **Decision:** ADR-0001
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

## MCP — публичная MCP-поверхность

### INV-MCP-ROLE-EDIT — Право роли изменяется через логическую typed-операцию

- **Rule:** `unica.role.edit` выбирает существующую роль только через
  `sourceSet + metadataPath`, принимает непустой закрытый массив операций
  `setRight` и возвращает типизированные `changed`, `effects` по
  `operationIndex`, `validation` и `diagnostics` без `stdout`, текстовой разницы
  и физических путей; схема и синтаксический разбор не принимают
  верхнеуровневые `RightsPath`, `Path`, `ObjectName`, `Name` и `Value`, а
  записывающий компонент сохраняет невыбранные права, ограничения на уровне
  записей, шаблоны и глобальные флаги в одной атомарной транзакции.
- **Decision:** ADR-0043
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/role.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

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
- **Decision:** ADR-0001
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

### INV-MCP-REACHABLE-ARGS — Инструмент публикует только достижимый аргумент

- **Rule:** Публичная схема инструмента называет только те аргументы, которые
  его обработчик читает и до которых вызов способен дойти: общий список
  нативных имён не является контрактом ни одного инструмента, аргумент,
  недостижимый из-за обязательного соседа, не публикуется, ограничение объёма
  ответа режет по сущностям контракта, а опубликованный и принимаемый наборы
  типизированных читателей закреплены таблицей.
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Scope:** runtime

### INV-MCP-SDK-TRANSPORT — Транспортом владеет официальный Rust SDK

- **Rule:** Публичный stdio-сервер — это реализация `rmcp::ServerHandler` в
  `interfaces/mcp.rs`, которая обслуживает оба жизненных цикла SDK —
  `initialize` и прямой первый запрос ревизии `2026-07-28`
  (`server/discover`, `tools/list`, `tools/call`) — из реестра слоя
  application, причём и типы `rmcp`, и макросы инструментов из SDK не выходят
  за пределы этого модуля.
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
  поставщиков за один общий ограниченный срок; одинаковые вызовы, ожидающие уже
  начатую доставку движка, не удерживают общее окно ожидания и сразу освобождают
  место в пуле допуска с состоянием работы.
- **Decision:** ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/code_intelligence.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/engine_delivery.rs`
- **Scope:** runtime

### INV-MCP-DELIVERY-STATE — Доставка движка остаётся внутренней наблюдаемой работой

- **Rule:** Инструмент, которому в опубликованной поставке не хватает движка,
  сначала присоединяется к единственной серверной доставке его артефакта; если
  владелец не завершил её внутри окна, обработчик не запускается, а общий
  конверт возвращает необязательное `work` с `status`, `statusMessage` и
  необязательным `pollIntervalMs`: `working` несёт `ok=true`, отказ доставки —
  `ok=false` и причину, а следующий предметный вызов повторно проверяет
  готовность без отдельного публичного инструмента установки.
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/domain/long_work.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/application_ports.rs`
- **Scope:** packaged, runtime

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

### INV-MCP-PREVIEW-MUTATION-ONLY — Предпросмотр принадлежит мутации

- **Rule:** `ToolExecution::Read` не публикует и не принимает `dryRun` и
  исполняется только как `InvocationMode::Read`; `ToolExecution::Mutation`
  выводит `Preview` при отсутствующем или истинном `dryRun` и `Apply` только
  при `dryRun: false`.
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source, runtime, packaged

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

### INV-SKILL-EXECUTABLE-EXAMPLES — Примеры в скиллах — исполнимые вызовы MCP

- **Rule:** Каждый пример `tools/call` в скилле — настоящий параметризованный
  вызов: мутация успешно исполняется через предпросмотр MCP, а читатель — как
  настоящее чтение MCP над детерминированной фикстурой или локальным подставным
  поставщиком без записи в рабочее пространство и зависимости от живой сети.
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

### INV-APP-CONFIG-SNAPSHOT — Конфигурация вызова изолирована рабочим пространством

- **Rule:** Для `unica.code.search`, `unica.code.definition`,
  `unica.code.outline` и `unica.code.diagnostics` с `action=analyze` приложение
  после обнаружения рабочего пространства разрешает ровно один неизменяемый
  `OperationalConfig`: все сроки данного вызова выводятся из этого снимка без
  повторного чтения файлов, следующий вызов разрешает его заново, а снимок
  одного рабочего пространства не переиспользуется другим. Снимок читает
  самостоятельные присутствующие слои той же неверсионируемой закрытой схемы
  `operational`, `network`, `providers`, что и сетевая политика; `version`
  считается неизвестным на корне, а недопустимый общий корень отказывает всем
  его потребителям. Все файловые сроки — целые секунды не меньше 1 без верхнего
  ограничения операционной политики; значения 300, 300, 2, 45 и 120 —
  умолчания, а не потолки; сроки `RLM` и
  `git-grep` не превышают общий срок
  поиска, а публичный явный
  `unica.code.diagnostics.timeoutSeconds` сохраняет отдельный диапазон
  `30..=3600`. Операционный потребитель проверяет только `[operational]`:
  ошибка `network` или
  `providers` его не останавливает, а невалидное операционное поддерево
  останавливает затронутый вызов до запуска поставщика или процесса. Проверка
  готовности и рабочие операции анализатора и `RLM` с бюджетом вызывающей
  стороны получают полный остаток срока вызова без 120-секундной верхней
  границы; внутренний протокол с `schema_version = 4` без потерь переносит эти
  бюджеты полями `timeout_seconds` и `timeout_nanos`, а запись сервиса с
  `schema_version = 3` не переиспользуется и заменяется сервисом текущей схемы.
  Отмена имеет приоритет над одновременно полученной ошибкой загрузки. Прочие
  вызовы не разрешают `OperationalConfig` и не читают `[operational]`; отдельные
  потребители сетевой политики документации и стандартов продолжают читать те
  же файлы по `INV-APP-DOCUMENTATION-NETWORK-POLICY`.
- **Decision:** ADR-0040
- **Check:** `ci-test` — `crates/unica-coder/src/application/operational_config.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/domain/operational_config.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/operational_config.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
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

## CACHE — состояние рабочего пространства и кеш

### INV-CACHE-ORCHESTRATOR-OWNED — Состоянием рабочего пространства владеет оркестратор

- **Rule:** Оркестратор `unica` владеет состоянием рабочего пространства и
  логической инвалидацией по доменным событиям, а поставщик владеет реализацией
  жизненного цикла своего индекса, процесса и сессии; модель не согласовывает
  свежесть между движками, и оркестратор не читает частное хранилище поставщика.
- **Decision:** ADR-0003, ADR-0001
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

### INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE — Постоянное состояние поставщика не индексирует само себя

- **Rule:** Постоянное состояние `RLM` выводится из нормализованных `workspaceRoot + sourceRoot`, остаётся вне индексируемого `sourceRoot`, изолирует разные рабочие пространства, `worktree` и корни исходников и передаётся одинаково индексатору и читающему процессу.
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
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
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Scope:** runtime

### INV-CACHE-WORKTREE-ISOLATION — Связанное рабочее дерево git изолировано

- **Rule:** Идентичность рабочего пространства, его эпоха, корни кеша и ключи
  внутренних сервисов, индексов и сессий выводятся так, что связанное рабочее
  дерево git изолировано и от основной рабочей копии, и от любого другого
  рабочего дерева, а код, читающий состояние git, разрешает `.git` и как
  каталог, и как файл-указатель.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/code_intelligence_symlinked_workspace.rs`
- **Scope:** runtime

### INV-CACHE-RUNTIME-ROOT-ORDER — Разрешение корня кеша runtime детерминировано

- **Rule:** `unica-bootstrap` разрешает корень кеша runtime в фиксированном
  порядке — `UNICA_RUNTIME_CACHE_DIR` берётся как есть, если в нём не осталось
  неразвёрнутой подстроки `${`, затем `<CLAUDE_PLUGIN_DATA>/runtimes`, затем
  `<CODEX_HOME>/unica/runtimes`, затем `<HOME или USERPROFILE>/.codex/unica/runtimes`,
  а когда не задано ни одно из значений, завершается ошибкой — и публикует
  каждый проверенный артефакт атомарно под
  `<cacheRoot>/<artifact>/<version>--<assetSha256>/<target>`; ни версия плагина,
  ни одна семантическая версия без SHA-256 не отождествляют разные байты.
- **Decision:** ADR-0012, ADR-0014
- **Check:** `ci-test` — `crates/unica-bootstrap/src/host/runtime_cache.rs`
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, runtime

## SOURCE — наборы исходников рабочего пространства

### INV-SOURCE-ROOT-SEPARATION — Корень исходников отделён от рабочего пространства

- **Rule:** Полная инспекция считает каждый уникально адресуемый корень набора
  исходников строгим потомком корня рабочего пространства, поэтому равенство
  после нормализации или разрешения физической идентичности, включая `path: .`,
  `./` и ссылочный псевдоним, даёт одну первичную ошибку
  `source_set.root_is_workspace`, закрывает `ready` и не порождает производные
  ошибки о служебных путях внутри того же корня.
- **Decision:** ADR-0060
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_health/layout.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/project_health.rs`
- **Scope:** source, runtime

### INV-SOURCE-PORTABLE-GIT — Переносимость Git доказывается содержимым репозитория

- **Rule:** `repositoryReady` вычисляется отдельно от `ready` и требует
  отслеживаемых правил исключений, ролевой классификации атрибутов и окончаний
  строк выгрузки платформы и безопасной классификации подготовленного
  `ConfigDumpInfo.xml`; локальные правила не считаются переносимыми, а отдельное
  хранилище больших файлов предлагается только как необязательная подсказка и
  не меняет ни один флаг.
- **Decision:** ADR-0060
- **Check:** `ci-test` — `crates/unica-coder/src/domain/project_health.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_health/git.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_health/resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/project_health.rs`
- **Scope:** source, runtime

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
  индекса и идентичности сервиса. Этот выбор не сужает карту проекта:
  `unica.project.map` публикует все наборы, а `unica.project.status` проверяет
  каждый уникально адресуемый набор. Группа с повторяющимся именем получает
  одну диагностику рабочего пространства с полным `count` и не создаёт
  неразличимые проверки с `sourceSet`, потому что этот ключ не различает записи
  этой группы.
- **Decision:** ADR-0006, ADR-0060
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Check:** `ci-test` — `tests/ci/test_project_health_contract.py`
- **Scope:** source, runtime

### INV-SOURCE-SUBSYSTEM-TOPOLOGY — Публичные проекции подсистем выводятся из регистрации

- **Rule:** Единый построитель под одним удерживаемым корнем, открытым без
  перехода по символическим ссылкам, читает `Configuration.xml` и только транзитивно зарегистрированные
  дескрипторы из `Configuration/ChildObjects` и `Subsystem/ChildObjects`: только они
  расходуют бюджеты и образуют зависимости формата, а незарегистрированная раскладка не
  влияет на доказательство. Каждый элемент `Content` имеет тип `MetadataAddress | UUID`,
  и `meta.info` публикует только членства текущего дескриптора, сопоставляя обе его
  идентичности. `subsystem.info` публикует только дерево с адресами `SubsystemAddress` в
  диалекте БСП, выведенными из физического пути под доказанным корнем, а
  каждый доказанный узел принадлежит ровно одной эффективной роли. Недопустимый элемент,
  ошибка, отмена, истечение срока или неполное чтение не публикуются как пустая
  доказанная проекция, а сбор зависимостей формата не зависит от снятого `Mode`.
- **Decision:** ADR-0036
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- **Scope:** source, runtime

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
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Scope:** runtime

### INV-SOURCE-ATOMIC-PUBLISH — Мутация источника публикуется атомарно после проверки

- **Rule:** Изменяющая операция сначала собирает точный образ файла после записи
  и проверяет его целиком в памяти — включая повторный разбор и применение
  собственного diff, результат которого обязан побайтно совпасть с образом, — и
  только затем публикует его через промежуточный файл и атомарную замену;
  провал проверки, занятый путь промежуточного файла и любая ошибка публикации
  оставляют исходные байты нетронутыми.
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
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

### INV-SOURCE-BOUND-PREIMAGES — Мутация привязана к байтам, из которых выведена

- **Rule:** Для изменяющего вызова публичная предпроверка повторяется внутри
  обработчика по фактическим зависимостям XML, байты, из которых выведена
  мутация, привязываются к транзакции компиляции как точные преобразы, а
  сотрудничающие пишущие операции Unica берут одни и те же кооперативные
  блокировки публикации, поэтому изменение отклоняется, если наблюдённые байты
  разошлись между планированием и публикацией.
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/format_guard.rs`
- **Scope:** runtime

## PKG — упаковка и поставка

### INV-PKG-UNTRACKED-BUILD-OUTPUT — Собранные бинарники не попадают под контроль версий

- **Rule:** Собранные бинарники и прочие генерируемые пути пакета никогда не
  отслеживаются в исходном дереве, а упаковка завершается ошибкой, если
  отслеживаемый файл оказался внутри генерируемого пути или является
  символической ссылкой.
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-PKG-THIN-PACKAGE — Публичный пакет маркетплейса тонкий

- **Rule:** Опубликованный пакет несёт только файлы плагина и три небольших
  бинарника bootstrap; его `.mcp.json` запускает ядро через ограниченный
  командой shell-алиас Git, который определяет корень плагина для обоих хостов и
  передаёт его в `bootstrap/launch.sh`, и пакет никогда не зависит ни от полного
  runtime, ни от матрицы команд под каждую целевую платформу.
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, release

### INV-PKG-VERIFIED-ATOMIC-INSTALL — Получение runtime проверяется контрольной суммой и атомарно

- **Rule:** Bootstrap получает закреплённый артефакт своей цели, сверяет
  SHA-256 доставляемого ассета и каждый материализованный файл — с записанными
  суммой, размером и режимом, и только после этого публикует артефакт атомарно;
  повреждённая доставка, выход за staging, ссылка, потерянный или необъявленный
  файл никогда не становятся готовым ядром или движком.
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_runtime.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** packaged, release, runtime

### INV-PKG-TOOL-CLOSURE — Многофайловый инструмент входит в runtime полностью

- **Rule:** Многофайловый сторонний инструмент доставляется только как полная
  полезная нагрузка закреплённого артефакта: сборщик сверяет внешний
  SHA-256, безопасно извлекает только обычные файлы в изолированный staging,
  проверяет внутренние идентичности исходника и цели, точки входа, точный набор
  файлов, SHA-256, размеры и режимы, а манифест поставки перечисляет каждый файл
  с относительным соседством. Небезопасная запись, ссылка, повтор пути,
  коллизия, потерянная зависимость или необъявленный файл прекращает сборку или
  доставку до публикации готового корня.
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_runtime.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Scope:** source, packaged, release, runtime

### INV-PKG-BINARY-NAME — Публичный бинарник runtime называется `unica`

- **Rule:** Встроенный публичный бинарник, собираемый из Cargo-воркспейса,
  называется `unica` и записан под этим именем в
  `plugins/unica/third-party/tools.lock.json`.
- **Decision:** ADR-0001
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
- **Decision:** ADR-0012
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
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source

### INV-PKG-NO-INTERNAL-MATERIAL — Внутренние материалы сопровождения не уезжают в поставку

- **Rule:** Пакет плагина несёт только то, что нужно потребителю в момент работы:
  записи о происхождении апстримов, датированные записи ревью и внутренняя
  документация об устройстве пакета и конвейера живут вне `plugins/unica/` и в
  собранный плагин не попадают.
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
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-EXACT-CACHE-KEYS — Попадания в кеш Cargo точны и наблюдаемы

- **Rule:** Ключ кеша Cargo содержит ОС раннера, целевую платформу Unica,
  разрешённый ключ тулчейна и хеш `Cargo.lock`, префиксные ключи восстановления
  не используются, а каждая платформенная сборка сообщает свою целевую
  платформу, исход обращения к кешу и длительность сборки.
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-NARROW-ARTIFACTS — Артефакты узкие, типизированные и недолговечные

- **Rule:** Каталоги сборки Cargo никогда не выгружаются; между задачами данные
  переходят только как метаданные поставки, полезная нагрузка bootstrap, архивы
  ядра и узкий вход движка конкретной оценки релиза со сроком хранения в
  одни сутки, тогда как тонкая полезная нагрузка для маркетплейса сохраняет
  более длительный срок хранения для размещения и продвижения; полный комплект
  инструментов границу задания не пересекает.
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-SELF-VERIFIED-ARCHIVE — Каждая платформа проверяет то, что собрала

- **Rule:** Платформенный раннер упаковывает свой архив ядра и сверяет с его
  метаданными контрольную сумму архива, состав файлов, контрольные суммы
  элементов, режимы исполнения и обнулённые отметки времени до того, как архив
  будет выгружен или отброшен; дымовая проверка MCP исполняет это извлечённое
  ядро с явно подготовленными проверенными движками, а при публикации по тегу
  проверка ядра повторяется на скачанных опубликованных байтах.
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### INV-CI-TAG-ONLY-PUBLISH — Публикация происходит только по тегу

- **Rule:** Артефакты релиза публикуются только при push тега; прогоны для
  pull request и ручные прогоны собирают пакет и прогоняют дымовые проверки без
  публикации, а размещение и продвижение каталога остаются отдельными явными
  задачами.
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-SINGLE-GATE — Каждый pull request закрывает один агрегирующий шлюз

- **Rule:** Каждый pull request решается единственным стабильным агрегирующим
  шлюзом, который вместе оценивает задачи по исходникам, по Rust, по упаковке,
  по bootstrap, по оценке релиза и по опубликованным артефактам.
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `ci-test` — `tests/ci/test_evaluate_ci_gate.py`
- **Scope:** ci
