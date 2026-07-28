# Реестр архитектурных инвариантов

Этот документ — машинно-проверяемый реестр правил, которые должны оставаться
верными при развитии Unica. Каждая запись формулирует одно нормативное правило,
называет решение, из которого оно следует, и конкретную проверку, которая его
удерживает.

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
| `release-gate` | шаг релизного конвейера, блокирующий публикацию | путь к скрипту или workflow |
| `manual` | ручная проверка при ревью | свободное описание |

Правила идентификаторов:

- ID соответствует `^(INV|REQ)-[A-Z][A-Z0-9]*-[0-9]{2}$`. Префикс `INV`
  принадлежит инвариантам, префикс `REQ` — требованиям к качеству.
- ID уникален во всём корпусе спецификаций и никогда не переиспользуется после
  удаления записи: удалённый номер остаётся выведенным из обращения.
- Область фиксирует владельца правила, а не файл, в котором оно проверяется.
  У каждого реестра свой набор областей, и наборы не пересекаются: инварианты
  используют `PRODUCT`, `MCP`, `SKILL`, `APP`, `CACHE`, `SOURCE`, `PKG`,
  `PLATFORM`, `CI`, `DOC`; требования к качеству — `PERF`, `TOKEN`, `SAFETY`,
  `OBS`, `MAINT`, `COMPAT`, `REL`. Новая область заводится вместе с первой
  записью, которая ей принадлежит, и добавляется в этот перечень.
- `Scope` перечисляет контуры, в которых правило обязано выполняться:
  `source` (рабочее дерево), `packaged` (сгенерированный пакет), `ci`
  (конвейер), `release` (публикация), `runtime` (исполнение).

## PRODUCT — границы продукта

### INV-PRODUCT-01 — Один каталог плагина обслуживает двух хостов

- **Rule:** Unica поставляется как один каталог плагина, который обслуживает и
  Codex, и Claude Code; `.mcp.json`, `skills/`, справочники и граница MCP
  остаются нейтральными к хосту, и только каталоги манифестов `.codex-plugin/`
  и `.claude-plugin/` зависят от хоста.
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** source, packaged

### INV-PRODUCT-02 — Публичная поверхность моделирует операции разработчика

- **Rule:** Публичные скиллы и инструменты `unica.*` моделируют операции
  разработчика 1С:Предприятия; вопросы инфраструктуры и упаковки в поверхность,
  которую видит модель, не попадают.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-03 — Встроенные движки не попадают в маршрутизацию, видимую модели

- **Rule:** Скиллы и справочники, которые видит модель, не должны предписывать
  ей вызывать встроенные низкоуровневые движки напрямую или называть их
  MCP-серверами; доменный инструмент можно упомянуть по смыслу, но никогда — как
  цель вызова.
- **Decision:** ADR-0001, ADR-0005, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-PRODUCT-04 — Сгенерированный пакет — полноценная поставка

- **Rule:** Каждый публичный контракт, который выполняется в исходном дереве,
  выполняется и в сгенерированном пакете для маркетплейса, а проверка на уровне
  пакета обязательна дополнительно к проверке на уровне исходников.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Scope:** packaged, release

### INV-PRODUCT-05 — У версий встроенных инструментов один источник

- **Rule:** `plugins/unica/third-party/tools.lock.json` — источник версий
  встроенных инструментов, а запись о происхождении встроенного инструмента
  ссылается на него через `toolLockRef` вместо того, чтобы нести собственную
  версию или базовый коммит.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `tests/ci/test_skill_provenance.py`
- **Check:** `guard-script` — `scripts/ci/check-skill-upstreams.py`
- **Scope:** source, packaged, ci

### INV-PRODUCT-06 — DCS — каноническое имя домена компоновки данных

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

## MCP — публичная MCP-поверхность

### INV-MCP-01 — `unica` — единственный MCP-сервер, видимый модели

- **Rule:** Внутренние движки (сборка и runtime, анализ BSL, индекс кода,
  стандарты, операции с XML и DSL) доступны только через внутренние адаптеры и
  никогда не регистрируются как отдельные публичные MCP-серверы.
- **Decision:** ADR-0001, ADR-0006
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-MCP-02 — Единственный публичный MCP-сервер

- **Rule:** `plugins/unica/.mcp.json` объявляет ровно одну запись `mcpServers`
  с именем `unica` — и в исходном дереве, и в любом сгенерированном пакете.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-MCP-03 — Имя сервера в протоколе

- **Rule:** `initialize` возвращает `serverInfo.name = "unica"`.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-MCP-04 — Публичные инструменты живут в пространстве имён `unica.*`

- **Rule:** Публичный набор инструментов адресуется именами вида
  `unica.<group>.<operation>`, и упакованный runtime отдаёт под этим именем
  каждый обязательный инструмент `unica.*`, не отдавая удалённый псевдоним.
- **Decision:** ADR-0001, ADR-0011
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-mcp.py`
- **Scope:** runtime, packaged, release

### INV-MCP-05 — Контракты инструментов заданы данными и свободны от адаптеров

- **Rule:** Имена и описания инструментов берутся из реестра `ToolSpec` в
  `application/mod.rs`, входные схемы — из `application/tool_contracts.rs`
  поверх `application/operation_descriptors.rs`, транспорт только собирает эти
  три источника вместе, и ни одна публичная схема инструмента не показывает
  сырые аргументы адаптера.
- **Decision:** ADR-0001, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Scope:** runtime

### INV-MCP-06 — Транспортом владеет официальный Rust SDK

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

### INV-MCP-07 — Приём вызовов ограничен, отмена кооперативна

- **Rule:** Одновременно допускается не более 32 обработчиков `tools/call`,
  лишние вызовы завершаются ошибкой JSON-RPC `-32603` со словом `overloaded`,
  запрос, отменённый через `notifications/cancelled`, не получает ответа, а
  остановка транспорта отменяет ещё выполняющиеся доменные операции за
  ограниченное время.
- **Decision:** ADR-0013, ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** runtime

### INV-MCP-08 — Изменения публичной поверхности синхронны

- **Rule:** Добавление, удаление или переименование публичного MCP-инструмента
  меняет одним набором изменений реестр в Rust, стенд паритета и архитектурный
  слой — запись решения, план приёмки или запись реестра.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Check:** `guard-script` — `scripts/ci/check-architecture-sync.py`
- **Check:** `ci-test` — `tests/ci/test_architecture_sync_guard.py`
- **Scope:** source, packaged

## SKILL — маршрутизация скиллов

### INV-SKILL-01 — Скиллы маршрутизируются через MCP `unica`

- **Rule:** Каждый скилл, на который распространяется правило, документирует
  свою маршрутизацию через MCP `unica` и называет инструмент `unica.*`, который
  вызывает.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-02 — Скиллы не называют внутренние серверы-адаптеры

- **Rule:** Скиллы и справочники, которые видит модель, не должны называть
  внутренние MCP-серверы адаптеров или их идентификаторы инструментов как цели
  маршрутизации.
- **Decision:** ADR-0001, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-03 — Локальные для скилла скрипты операций не возвращаются

- **Rule:** Скиллы не должны поставлять или упоминать локальные для скилла файлы
  операций на Python, PowerShell или shell как путь исполнения; переход на
  нативные обработчики `unica.*` завершён, и возвращение такого пути требует
  решения, заменяющего действующее.
- **Decision:** ADR-0004, ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged, runtime

### INV-SKILL-04 — Эталонные модели существуют только как тестовые фикстуры

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

### INV-SKILL-05 — Изменяющие инструкции по умолчанию ведут через предпросмотр

- **Rule:** Инструкции скиллов держат путь предпросмотра на виду на
  разрушительных и неполных маршрутах: скилл `meta-remove` документирует вызов
  с `"dryRun": true`, а каждая документированная инкрементальная, частичная или
  относящаяся к внешнему набору исходников выгрузка записана как
  вызов-предпросмотр.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-SKILL-06 — Примеры в скиллах — исполнимые вызовы MCP

- **Rule:** Каждый пример `tools/call` в скилле — настоящий параметризованный
  вызов, который успешно исполняется как сухой прогон MCP.
- **Decision:** ADR-0005
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source, packaged

## APP — границы слоёв приложения

### INV-APP-01 — Слой application владеет диспетчеризацией и доменными событиями

- **Rule:** `UnicaApplication` владеет публичным реестром инструментов,
  диспетчеризацией вызовов и порождением доменных событий; новый обработчик
  инструмента входит в систему через диспетчеризацию application и никак иначе.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** source, runtime

### INV-APP-02 — Транспорт только отображает протокол на вызовы application

- **Rule:** `interfaces::mcp` обслуживает `tools/list` из
  `UnicaApplication::tools()`, направляет каждый `tools/call` через
  `call_tool_cancellable` и возвращает как текст инструмента конверт результата,
  собранный слоем application, а не собственную структуру.
- **Decision:** ADR-0002, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/interfaces/mcp.rs`
- **Scope:** source, runtime

### INV-APP-03 — Адаптеры идут к рабочему пространству через порты application

- **Rule:** Адаптеры инфраструктуры обращаются к состоянию рабочего
  пространства через `ApplicationPorts` и никогда не импортируют слой
  interfaces, поэтому адаптер не может отрисовать ответ MCP и по дороге наружу
  обойти отчёт о кеше, который ведёт слой application.
- **Decision:** ADR-0002, ADR-0003
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, runtime

### INV-APP-04 — В runtime нет скриптового бэкенда

- **Rule:** В `unica-coder` нет отката на файлы операций во время исполнения: ни
  унаследованного обработчика скриптов, ни запуска `python`, `python3`, `bash`,
  `powershell` или `pwsh` из продуктивного кода.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, runtime

### INV-APP-05 — Направление зависимостей между слоями закреплено проверкой

- **Rule:** `domain` не импортирует ни `application`, ни `infrastructure`, ни
  `interfaces` и не обращается к файловой системе и процессам, а `application`
  не импортирует ни `infrastructure`, ни `interfaces`.
- **Decision:** ADR-0009, ADR-0002
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-APP-06 — Application не запускает git напрямую

- **Rule:** Продуктивный код в `crates/unica-coder/src/application` никогда не
  создаёт дочерний процесс `git`; состояние git читается через инфраструктуру.
- **Decision:** ADR-0002, ADR-0009
- **Check:** `ci-test` — `tests/ci/test_product_contracts.py`
- **Scope:** source

### INV-APP-07 — Внутренние сервисы скрыты и привязаны к рабочему пространству

- **Rule:** Прогретое состояние анализатора и индекса живёт в скрытых сервисах,
  ключ которых складывается из корня рабочего пространства и корня исходников;
  сервисы запускаются лениво и только тогда, когда их требует не сухая операция
  анализатора или индекса; дешёвые операции только на чтение вроде
  `unica.code.grep` сервис не поднимают, и ни один сервис не становится
  публичной регистрацией MCP.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## CACHE — состояние рабочего пространства и кеш

### INV-CACHE-01 — Состоянием рабочего пространства владеет оркестратор

- **Rule:** Состояние рабочего пространства и инвалидация кеша принадлежат
  оркестратору `unica`; модель никогда не просят согласовывать свежесть кеша
  между движками.
- **Decision:** ADR-0003, ADR-0001
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### INV-CACHE-02 — Изменяющие операции порождают типизированные доменные события

- **Rule:** Каждая изменяющая операция порождает типизированные доменные
  события, и эти события отображаются на имена инвалидированных и обновлённых
  кешей, о которых сообщается вызывающему.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### INV-CACHE-03 — Корень изменчивого кеша можно переопределить

- **Rule:** Корень изменчивого кеша по умолчанию равен
  `<workspaceRoot>/.build/unica` и переопределяется переменной
  `UNICA_CACHE_DIR`, а записи о скрытых сервисах рабочего пространства пишутся
  под тем корнем, который действует.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

### INV-CACHE-04 — Сухой прогон сообщает о последствиях, не записывая состояние

- **Rule:** Вызов в режиме сухого прогона сообщает о своём влиянии на кеш и не
  пишет ни состояние рабочего пространства, ни индекс, ни запись о сервисе.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_index.rs`
- **Scope:** runtime

### INV-CACHE-05 — Применённое изменение запоминает инвалидированный им кеш

- **Rule:** Применённое изменение записывает свои доменные события в
  `WorkspaceStateRepository`, поэтому кеш, который оно инвалидировало, при
  следующем чтении по-прежнему числится устаревшим, а не оказывается молча
  пересобранным.
- **Decision:** ADR-0003, ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_state.rs`
- **Scope:** runtime

### INV-CACHE-06 — Связанное рабочее дерево git изолировано

- **Rule:** Идентичность рабочего пространства, его эпоха, корни кеша и ключи
  внутренних сервисов выводятся так, что связанное рабочее дерево git
  изолировано и от основной рабочей копии, и от любого другого рабочего дерева,
  а код, читающий состояние git, разрешает `.git` и как каталог, и как
  файл-указатель.
- **Decision:** ADR-0003
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace.rs`
- **Scope:** runtime

### INV-CACHE-07 — Разрешение корня кеша runtime детерминировано

- **Rule:** `unica-bootstrap` разрешает корень кеша runtime в фиксированном
  порядке — `UNICA_RUNTIME_CACHE_DIR` берётся как есть, если в нём не осталось
  неразвёрнутой подстроки `${`, затем `<CLAUDE_PLUGIN_DATA>/runtimes`, затем
  `<CODEX_HOME>/unica/runtimes`, затем `<HOME или USERPROFILE>/.codex/unica/runtimes`,
  а когда не задано ни одно из значений, завершается ошибкой — и публикует
  проверенный runtime атомарно под `<cacheRoot>/<pluginVersion>/<target>`.
- **Decision:** ADR-0008, ADR-0012
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `manual` — тесты покрывают только опубликованную раскладку и
  значение `UNICA_RUNTIME_CACHE_DIR` в пакете, поэтому весь порядок отката,
  отбрасывание значения с `${`, сегмент `.codex`, добавляемый при незаданном
  `CODEX_HOME`, и жёсткую ошибку при отсутствии всех переменных ревью проверяет
  по `runtime_cache_root` и `codex_home_root` в
  `crates/unica-bootstrap/src/main.rs`
- **Scope:** packaged, runtime

## SOURCE — наборы исходников рабочего пространства

### INV-SOURCE-01 — Формат — свойство набора исходников

- **Rule:** `unica.project.map` сообщает `sourceSets[]`, и каждая запись несёт
  собственный `sourceFormat`, потому что формат исходников — свойство
  отдельного набора, а не всего рабочего пространства.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-02 — Один набор исходников не бывает двух форматов сразу

- **Rule:** Противоречащие друг другу признаки формата внутри одного набора
  исходников делают его недопустимым или неоднозначным; набор никогда не
  сообщает смешанный формат.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Scope:** runtime

### INV-SOURCE-03 — В рабочем пространстве может действовать несколько форматов

- **Rule:** Одно рабочее пространство может содержать несколько наборов
  исходников с разными действующими форматами — например, конфигурацию в формате
  EDT рядом с внешними обработками и отчётами в формате platform XML.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/project_sources.rs`
- **Check:** `doc-assert` — `tests/ci/test_unica_skills.py`
- **Scope:** runtime, source

### INV-SOURCE-04 — Нативные операции с XML требуют формата platform XML

- **Rule:** Нативная операция над метаданными в формате platform XML сначала
  разрешает набор исходников, у которого `sourceFormat` равен `platform_xml`, и
  лишь затем трогает XML-файлы; если разрешённый набор оказался в формате EDT,
  недопустимым или неоднозначным, операция отклоняется типизированной ошибкой.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-05 — Выбор корня исходников детерминирован и общий

- **Rule:** Непустой `sourceDir` разрешается относительно рабочего каталога
  запроса, иначе побеждает набор исходников с именем `main`, а за ним —
  единственный набор исходников конфигурации; разрешённый корень нормализуется,
  остаётся внутри рабочего пространства и служит тем же корнем для анализатора,
  индекса, идентичности сервиса, `unica.project.status` и `unica.project.map`.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/tool_context.rs`
- **Scope:** runtime

### INV-SOURCE-06 — Перевод строки наблюдается в источнике, а не назначается

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
- **Scope:** runtime

### INV-SOURCE-07 — Запись не выходит за корень рабочего пространства

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

## PKG — упаковка и поставка

### INV-PKG-01 — Собранные бинарники не попадают под контроль версий

- **Rule:** Собранные бинарники и прочие генерируемые пути пакета никогда не
  отслеживаются в исходном дереве, а упаковка завершается ошибкой, если
  отслеживаемый файл оказался внутри генерируемого пути или является
  символической ссылкой.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source, packaged

### INV-PKG-02 — Публичный пакет маркетплейса тонкий

- **Rule:** Опубликованный пакет несёт только файлы плагина и три небольших
  бинарника bootstrap; его `.mcp.json` запускает runtime через ограниченный
  командой shell-алиас Git, который определяет корень плагина для обоих хостов и
  передаёт его в `bootstrap/launch.sh`, и пакет никогда не зависит ни от полного
  бинарника runtime, ни от матрицы команд под каждую целевую платформу.
- **Decision:** ADR-0008, ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** packaged, release

### INV-PKG-03 — Получение runtime проверяется контрольной суммой и атомарно

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

### INV-PKG-04 — Публичный бинарник runtime называется `unica`

- **Rule:** Встроенный публичный бинарник, собираемый из Cargo-воркспейса,
  называется `unica` и записан под этим именем в
  `plugins/unica/third-party/tools.lock.json`.
- **Decision:** ADR-0001, ADR-0008
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Scope:** source, packaged

### INV-PKG-05 — Оба манифеста хостов несут одну версию

- **Rule:** `plugins/unica/.codex-plugin/plugin.json` и
  `plugins/unica/.claude-plugin/plugin.json` оба существуют и объявляют ту же
  версию, что Cargo-воркспейс и запись `unica` в `tools.lock.json`; манифест
  Claude не объявляет ни `skills`, ни `mcpServers`, потому что и то и другое
  обнаруживается по умолчанию.
- **Decision:** ADR-0012
- **Check:** `guard-script` — `scripts/ci/check-version-contract.py`
- **Check:** `ci-test` — `tests/ci/test_version_contract.py`
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `crates/unica-bootstrap/src/main.rs`
- **Scope:** source, packaged

### INV-PKG-06 — Манифесты и каталоги не выходят за нижнюю границу клиента

- **Rule:** Манифесты хостов и записи каталогов используют только те ключи,
  которые принимает самый старый поддерживаемый клиент, а оба каталога хостов
  закрепляют один и тот же неизменяемый тег релиза с типом источника,
  адресующим подкаталог.
- **Decision:** ADR-0012, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** packaged, release

### INV-PKG-07 — Локальная отладочная упаковка существует только для разработки

- **Rule:** Локальный отладочный пакет запускает бинарник `bin/<target>/unica`
  (`unica.exe` на `win-x64`) для текущего хоста напрямую, а не через полезную
  нагрузку bootstrap — по относительному пути с `cwd` в Codex и через
  `${CLAUDE_PLUGIN_ROOT}` без `cwd` в Claude Code, — собирается только под
  текущую целевую платформу и регистрирует свой каталог Codex под именем
  `unica-dev`, чтобы этот каталог нельзя было принять за опубликованный.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Scope:** source

### INV-PKG-08 — Атрибуция остаётся полной и доступной

- **Rule:** У каждого встроенного инструмента, адаптированного источника скилла
  и упакованного стороннего ресурса есть запись об атрибуции, а страница
  атрибуции связана ссылкой и из репозитория, и из README в пакете.
- **Decision:** n/a
- **Check:** `guard-script` — `scripts/ci/check-attributions.py`
- **Check:** `ci-test` — `tests/ci/test_attributions.py`
- **Scope:** source, packaged

## PLATFORM — платформенный фасад

### INV-PLATFORM-01 — Зависящий от ОС код живёт за платформенными фасадами

- **Rule:** Зависящий от ОС продуктивный код существует только под
  `crates/unica-coder/src/infrastructure/platform/**` и
  `crates/unica-bootstrap/src/platform/**`; поведение файловой системы, путей,
  процессов и точек входа попадает в остальной код через эти фасады в виде
  платформенно-нейтральных типов.
- **Decision:** ADR-0009
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### INV-PLATFORM-02 — У платформенного стража нет исключений по путям

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

### INV-PLATFORM-03 — Платформенные тесты лежат рядом со своими адаптерами

- **Rule:** Зависящие от платформы тесты лежат рядом со своими адаптерами или
  под `crates/<crate>/tests/platform/**`, но никогда — как платформенный
  тестовый файл верхнего уровня.
- **Decision:** ADR-0009
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source, ci

### INV-PLATFORM-04 — Дочерние процессы удерживаются целыми деревьями

- **Rule:** Дочерние процессы анализатора, индекса и runtime удерживаются
  целыми деревьями — Job Object с завершением по закрытию на Windows и отдельная
  группа процессов на Unix, — поэтому отмена, тайм-аут, остановка или отказ
  сессии завершают всё дерево за ограниченное время ожидания.
- **Decision:** ADR-0006, ADR-0009
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform/process.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

## CI — сборка, артефакты и релизный конвейер

### INV-CI-01 — Одна закреплённая сборка Cargo на платформенный раннер

- **Rule:** Каждый платформенный раннер собирает `unica` и `unica-bootstrap`
  одним обязательным вызовом `cargo build --locked` в отдельный для целевой
  платформы каталог сборки Cargo; восстановленный кеш эту команду ускоряет, но
  никогда не заменяет.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_build_unica_tools.py`
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-02 — Попадания в кеш Cargo точны и наблюдаемы

- **Rule:** Ключ кеша Cargo содержит ОС раннера, целевую платформу Unica,
  разрешённый ключ тулчейна и хеш `Cargo.lock`, префиксные ключи восстановления
  не используются, а каждая платформенная сборка сообщает свою целевую
  платформу, исход обращения к кешу и длительность сборки.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci

### INV-CI-03 — Артефакты узкие, типизированные и недолговечные

- **Rule:** Каталоги сборки Cargo никогда не выгружаются; между задачами данные
  переходят только как метаданные runtime, полезная нагрузка bootstrap и архивы
  runtime со сроком хранения в одни сутки, тогда как тонкая полезная нагрузка
  для маркетплейса сохраняет более длительный срок хранения для ручного
  размещения и продвижения.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-04 — Каждая платформа проверяет то, что собрала

- **Rule:** Платформенный раннер упаковывает свой архив runtime и сверяет с его
  метаданными контрольную сумму архива, состав файлов, контрольные суммы
  элементов, режимы исполнения и обнулённые отметки времени до того, как архив
  будет выгружен или отброшен; при публикации по тегу проверка повторяется на
  скачанных опубликованных байтах.
- **Decision:** ADR-0010, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### INV-CI-05 — Публикация происходит только по тегу

- **Rule:** Артефакты релиза публикуются только при push тега; прогоны для
  pull request и ручные прогоны собирают пакет и прогоняют дымовые проверки без
  публикации, а размещение и продвижение каталога остаются отдельными явными
  задачами.
- **Decision:** ADR-0008, ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Scope:** ci, release

### INV-CI-06 — Каждый pull request закрывает один агрегирующий шлюз

- **Rule:** Каждый pull request решается единственным стабильным агрегирующим
  шлюзом, который вместе оценивает задачи по исходникам, по Rust, по упаковке,
  по bootstrap, по оценке релиза и по опубликованным артефактам.
- **Decision:** ADR-0010
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `ci-test` — `tests/ci/test_evaluate_ci_gate.py`
- **Scope:** ci

## DOC — документационный слой

### INV-DOC-01 — Записи реестра оформлены каноническим форматом

- **Rule:** Каждая запись реестра несёт заголовок `### <ID> — <короткое имя>`,
  ровно одно поле `Rule`, поле `Decision`, хотя бы одно поле `Check` и поле
  `Scope`, причём класс проверки в `Check` взят из `ci-test`, `guard-script`,
  `doc-assert`, `release-gate` или `manual`, а значения `Scope` — из `source`,
  `packaged`, `ci`, `release` или `runtime`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-02 — ID реестра уникальны и не переиспользуются

- **Rule:** Каждый ID реестра соответствует
  `^(INV|REQ)-[A-Z][A-Z0-9]*-[0-9]{2}$`, уникален во всём корпусе спецификаций
  и никогда не назначается другому правилу после того, как исходная запись
  удалена.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-03 — Каждый инвариант называет настоящую проверку

- **Rule:** Каждая запись реестра называет хотя бы одну проверку, и каждая
  проверка класса, отличного от `manual`, указывает на путь, существующий в
  репозитории.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-04 — Индексы синхронны со своими документами

- **Rule:** Каждая принятая запись решения перечислена в
  `spec/decisions/README.md`, каждая перечисленная запись существует на диске, и
  каждый документ каталога `spec/architecture/` перечислен в `spec/README.md`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-05 — Исторические документы помечены как исторические

- **Rule:** Индекс каждого архивного дерева — `docs/design` и `docs/plans` —
  несёт архивную пометку, которая называет его архивным материалом планирования,
  а не источником истины, и ни один нормативный документ не живёт вне `spec/`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-06 — Относительные ссылки разрешаются от своего документа

- **Rule:** Каждая относительная markdown-ссылка в действующем слое документации
  разрешается от каталога того документа, который её несёт, поэтому читателю не
  нужен корень репозитория, чтобы по ней перейти.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `ci-test` — `tests/ci/test_unica_skills.py`
- **Scope:** source, packaged

### INV-DOC-07 — Нормативный текст пишется по-русски

- **Rule:** Нормативные поля записи — `Rule`, `Decision` и `Scope` — пишутся
  по-русски, поэтому один поиск по русской формулировке находит все утверждения
  правила; по-английски в них остаются только идентификаторы: пути и имена
  файлов, имена инструментов, типов, функций и переменных окружения, ID записей
  вида `ADR-0001` и значения перечислений в полях `Decision` и `Scope`.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Scope:** source

### INV-DOC-08 — У нормативного текста один владелец

- **Rule:** Правило, которым владеет запись реестра или запись решения,
  цитируется из других документов по ID, а не пересказывается заново, и ни один
  документ архитектурного слоя не воспроизводит каталог решений как второй
  индекс.
- **Decision:** n/a
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `manual` — автоматизирована только та половина правила, что
  запрещает второй индекс, поэтому архитектурное ревью отклоняет скопированное
  нормативное предложение в пользу ссылки по ID
- **Scope:** source

### INV-DOC-09 — Принятое решение не переписывают

- **Rule:** Принятая запись решения не правится под изменившийся код: вместо
  правки заводится новая запись, прежняя получает статус `superseded` и называет
  заменяющую, поле `Date` уже принятой записи не переписывается никогда, а
  редакционное изменение её текста отмечается полем `Updated`.
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
