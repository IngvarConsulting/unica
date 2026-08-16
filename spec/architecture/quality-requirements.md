# Требования к качеству

Этот документ — реестр требований к качеству. Инвариант отвечает на вопрос «что
нельзя сломать», требование — на вопрос «насколько хорошо система обязана себя
вести». Поэтому каждая запись здесь описывает сценарий: что происходит
(стимул) и какой измеримый ответ система обязана дать.

## Как читать реестр

- Формат записи, классы проверок и правила ID общие с
  [реестром инвариантов](invariants.md); там же описано, что означают поля
  `Rule`, `Decision`, `Check` и `Scope`.
- ID требований цитируются в описании PR и в ревью так же, как ID инвариантов:
  «REQ-SAFETY-SECRET-REDACTION сохранён», «REQ-PERF-WARM-REUSE меняется, см. новый бенчмарк».
- Требование не повторяет текст инварианта. Если структурное правило уже
  нормировано, требование формулирует наблюдаемое качество и ссылается на
  инвариант по ID (INV-DOC-SINGLE-RULE-OWNER).
- Если измеримый ответ ничем не проверяется автоматически, класс проверки —
  `manual`, а в цели честно написано, что именно смотрит человек. Отсутствие
  проверки — это долг, а не оформительская мелочь; такие места собраны в
  [реестре рисков](risks.md).

## PERF — задержки и бюджеты

### REQ-PERF-DEADLINE — Ни один публичный вызов не ждёт без крайнего срока

- **Rule:** Каждый запрос от публичного инструмента к внутреннему сервису
  рабочего пространства несёт явные бюджеты на подключение, на чтение и на
  операцию целиком, поэтому неотвечающий или зависший сервис превращается в
  типизированную ошибку в пределах записанного срока, а не в MCP-вызов, который
  никогда не вернётся.
- **Decision:** ADR-0006
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** runtime

### REQ-PERF-VERIFIED-HANDOFF — Релиз объявляет runtime годным только после ограниченного по времени рукопожатия

- **Rule:** `unica-bootstrap verify` объявляет установленный runtime годным
  только после того, как тот ответил на MCP-запросы `initialize` и `tools/list`
  в пределах фиксированного бюджета времени. В релизном конвейере эта проверка
  скачивает уже опубликованные артефакты GitHub Release: зависание или неполный
  ответ проваливает исходный workflow, поэтому тонкий пакет не попадает в staging
  маркетплейса и стабильный каталог не переводится, но публикация артефактов не
  отменяется. Обычный запуск `unica-bootstrap run` рукопожатие не повторяет, а
  передаёт хосту те же байты, сверив их контрольные суммы с закреплёнными
  (INV-PKG-VERIFIED-ATOMIC-INSTALL).
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/verification_contract.rs`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Check:** `ci-test` — `tests/ci/test_bootstrap_launch_path.py`
- **Check:** `manual` — числовой бюджет, который `install_and_verify_runtime`
  передаёт в `crates/unica-bootstrap/src/main.rs`, не закреплён тестом;
  перечитай его, когда меняется путь запуска runtime
- **Scope:** packaged, release

### REQ-PERF-WARM-REUSE — Тёплое состояние переиспользуется, а не собирается заново на каждый вызов

- **Rule:** Сервис рабочего пространства, поднятый для одной пары «корень
  рабочего пространства и корень исходников», переиспользуется последующими
  вызовами, пока он жив, а его бюджеты простоя и предельного возраста берутся из
  фиксированных значений по умолчанию либо из переопределяющих их переменных
  окружения. Поэтому повторные вызовы анализатора не платят за холодный старт,
  а тёплый RLM-вызов на поддерживаемой локальной файловой системе выполняет два
  событийных барьера и не делает полного обхода неизменившегося корпуса.
- **Decision:** ADR-0006, ADR-0057
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/source_revision.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/workspace_services.rs`
- **Check:** `ci-test` — `crates/unica-coder/tests/platform/issue_89_workspace_service.rs`
- **Scope:** runtime

### REQ-PERF-SOURCE-BOUNDS — Снимки и ресурсные ответы имеют измеримые пределы

- **Rule:** Ресурсный снимок живёт не более 5 минут, содержит не более 100
  ресурсов и 32 МиБ, одна страница возвращает не более 50 ресурсов, одно чтение
  — не более 64 КиБ, замена — не более 1 МиБ, а экземпляр удерживает не более
  64 снимков и 128 МиБ; отмена проверяется до разрешения цели, между
  ограниченными фазами и непосредственно перед атомарной публикацией.
- **Decision:** ADR-0022
- **Check:** `doc-assert` — `tests/ci/test_architecture_registry.py`
- **Check:** `ci-test` — `crates/unica-coder/src/domain/source_resources.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- **Scope:** runtime

## TOKEN — расход контекста модели

### REQ-TOKEN-NO-EXTRA-ROUNDTRIP — Согласование кеша не стоит лишнего вызова

- **Rule:** Вызывающей стороне никогда не нужен второй вызов инструмента, чтобы
  узнать, что обесценила мутация: тот же самый результат называет затронутые
  кеши, поэтому у модели никогда не спрашивают, какой внутренний движок каким
  кешем владеет (структурное правило: INV-CACHE-ORCHESTRATOR-OWNED).
- **Decision:** ADR-0001, ADR-0003
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime

### REQ-TOKEN-BOUNDED-LOG-TAILS — Вывод долгой операции остаётся в файлах, а не в результате

- **Rule:** Вывод задания runtime захватывается по потокам в `stdout.log` и
  `stderr.log` рядом с долговременной записью задания, а `unica.runtime.job.*`
  возвращает удержанные хвосты потоков и пути к этим файлам, а не сами файлы
  журнала, поэтому опрос долгой сборки или долгого прогона тестов не тащит весь
  журнал в результат.
- **Decision:** ADR-0001
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Scope:** runtime

## SAFETY — безопасность мутаций и данных

### REQ-SAFETY-PREVIEW-BY-DEFAULT — Мутация сначала показывает предпросмотр, потом применяется

- **Rule:** Мутирующий инструмент, вызванный без явного `dryRun: false`,
  разрешается в предпросмотр: он планирует изменение, сообщает его влияние на
  кеш и ничего не пишет; тронуть рабочее пространство может только явный
  `dryRun: false`.
- **Decision:** ADR-0003, ADR-0005
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### REQ-SAFETY-SECRET-REDACTION — Секреты не доходят до модели

- **Rule:** Строки подключения, пароли, токены и другие значения под секретными
  ключами вымарываются из результатов инструментов, из записей заданий runtime,
  из отражённых в ответе векторов аргументов и из сообщений об ошибках прежде,
  чем покинут процесс, — в том числе когда секретный ключ и его значение
  разорваны между потоковыми фрагментами.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/redaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/operational_config.rs`
- **Scope:** runtime

### REQ-SAFETY-SUPPORT-LOCK — Объекты поставщика на поддержке защищены до планирования

- **Rule:** Нативная мутирующая операция, цель которой заперта состоянием
  поддержки конфигурации, отклоняется типизированным исходом, и отказ
  происходит до планирования, поэтому предпросмотр и применённый вызов приходят
  к одному вердикту; понизить отказ до предупреждения может только состояние
  поддержки, записанное в самой конфигурации.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Scope:** runtime

### REQ-SAFETY-NO-PARTIAL-WRITE — Упавшая запись не оставляет наполовину переписанное дерево

- **Rule:** Публикация метаданных или файла либо становится видимой целиком,
  либо не происходит вовсе: прерванная, упавшая или отменённая запись оставляет
  прежнее дерево исходников нетронутым, а не переписанным наполовину.
- **Decision:** n/a
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
- **Scope:** runtime

## OBS — наблюдаемость результата

### REQ-OBS-STABLE-ENVELOPE — Один стабильный конверт результата

- **Rule:** Каждый инструмент возвращает один и тот же конверт верхнего уровня —
  `ok`, `summary`, `changes`, `warnings`, `errors`, `artifacts`, `cache`, — а
  более объёмные данные входят через необязательные типизированные поля вроде
  `diagnostics`, `data` и `job`, поэтому новый адаптер добавляет подробности, не
  переделывая контракт, который разбирают вызывающие стороны.
- **Decision:** ADR-0001, ADR-0002
- **Check:** `ci-test` — `crates/unica-coder/src/application/mod.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_smoke.py`
- **Scope:** runtime

### REQ-OBS-DETACHED-PROGRESS — За долгой операцией можно наблюдать, пока она идёт

- **Rule:** Операция, явно запущенная через `unica.runtime.job.start`, отдаёт
  свой статус, свои ограниченные по объёму журналы и факт завершения через
  долговременные записи, поэтому её ход виден без удержания открытого
  MCP-вызова на всё время прогона; эта поверхность не является запасным путём
  для `unica.runtime.execute` (INV-MCP-RUNTIME-RECEIPT).
- **Decision:** ADR-0001, ADR-0006, ADR-0066
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- **Scope:** runtime

## MAINT — сопровождаемость

### REQ-MAINT-CONTAINED-ADAPTER-SWAP — Замена адаптера не переписывает работу с MCP

- **Rule:** Замена или переписывание внутреннего адаптера заперты внутри слоя
  `infrastructure`: имена инструментов, схемы, диспетчеризация и отображение на
  протокол остаются на своих местах, а страж слоёв проваливает изменение, если
  граница пересечена (структурное правило: INV-APP-DEPENDENCY-DIRECTION).
- **Decision:** ADR-0002, ADR-0009
- **Check:** `guard-script` — `scripts/ci/check-rust-platform-boundary.py`
- **Check:** `ci-test` — `tests/ci/test_rust_platform_boundary.py`
- **Scope:** source

### REQ-MAINT-NO-TRANSPORT-EDIT — Новый публичный инструмент — это данные, а не транспортный код

- **Rule:** Добавление публичного инструмента добавляет запись в реестр
  инструментов и схему его аргументов в слое приложения и не требует правки
  модуля MCP-транспорта, потому что имена, описания и схемы — это данные
  (структурное правило: INV-MCP-DATA-DRIVEN-SCHEMA).
- **Decision:** ADR-0001, ADR-0013
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Scope:** source

### REQ-MAINT-DONOR-PARITY — Расхождение с донором валит набор тестов, а не рабочее пространство пользователя

- **Rule:** Поведение, обязанное оставаться совместимым с адаптированной
  донорской моделью, покрыто паритетным сценарием, который исполняется как
  настоящий MCP-вызов в режиме предпросмотра, а принятый на ревью донорский
  снимок закреплён дайджестом, поэтому ушедший в сторону Rust-порт валит CI
  раньше, чем пользователь встретит различие.
- **Decision:** ADR-0004
- **Check:** `ci-test` — `tests/ci/test_unica_mcp_script_parity.py`
- **Check:** `ci-test` — `tests/ci/test_donor_parity_contract.py`
- **Scope:** ci

## COMPAT — совместимость

### REQ-COMPAT-ALL-TARGETS-GREEN — Каждая поддерживаемая цель собирается и проверяется вместе с остальными

- **Rule:** Релиз собирает `unica` и `unica-bootstrap` для каждой поддерживаемой
  целевой платформы хоста, и каждая цель проверяет собранный ею архив прежде,
  чем этот архив можно опубликовать; цель, провалившая собственную проверку,
  блокирует публикацию для всех остальных.
- **Decision:** ADR-0055, ADR-0008
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `release-gate` — `scripts/ci/verify-release-assets.py`
- **Scope:** ci, release

### REQ-COMPAT-OLDEST-CLIENT-LOAD — Упакованный плагин загружается на нижней поддерживаемой версии клиента

- **Rule:** Сгенерированный пакет маркетплейса и его каталог проверяются
  настоящим клиентом хоста, закреплённым на самой старой поддерживаемой версии,
  потому что нераспознанный ключ манифеста или каталога там — ошибка загрузки, а
  не предупреждение (структурное правило: INV-PKG-OLDEST-CLIENT-KEYS).
- **Decision:** ADR-0012
- **Check:** `release-gate` — `.github/workflows/unica-plugin-release.yml`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Check:** `manual` — настоящую проверку клиентом в CI проходит только один из
  двух хостов; манифест и каталог второго хоста просматриваются вручную, когда
  меняется любой из этих контрактов
- **Scope:** packaged, release

### REQ-COMPAT-IDENTICAL-HOST-SURFACE — Один пакет обслуживает оба хоста

- **Rule:** Публикуемый пакет содержит каталоги манифестов обоих хостов и оба
  каталога маркетплейса на одной версии, а также единственный нейтральный к
  хосту запускающий модуль, поэтому установка одних и тех же байтов на любом из
  хостов даёт одну и ту же публичную поверхность инструментов (структурное
  правило: INV-PRODUCT-SINGLE-PLUGIN-TREE, INV-PKG-VERSION-LOCKSTEP).
- **Decision:** ADR-0012
- **Check:** `ci-test` — `tests/ci/test_package_unica_plugin.py`
- **Check:** `ci-test` — `tests/ci/test_version_contract.py`
- **Scope:** packaged, release

### REQ-COMPAT-FORMAT-PROFILE — Профиль формата 1С, в котором пишет Unica, задан явно и проверен

- **Rule:** Нативные операции над XML берут действующую линейку платформы и
  формат выгрузки из одной общей константы-профиля, а каждое намеренное
  отклонение от официального отображения платформы записано в матрицу
  отклонений, а не обнаруживается в выгрузке пользователя.
- **Decision:** n/a
- **Check:** `ci-test` — `tests/ci/test_format_profile_contract.py`
- **Check:** `ci-test` — `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`
- **Scope:** runtime, source

## REL — надёжность поставки

### REQ-REL-BUNDLED-ENGINES — Используются поставляемые движки, а не PATH хоста

- **Rule:** Внутренний движок разрешается из упакованного пути
  `bin/<target>/<tool>`, записанного в манифесте сторонних инструментов, и
  проверяется по SHA-256 перед запуском, а упакованный bootstrap прогоняется в
  дымовом тесте с потребительским PATH, урезанным до системных каталогов целевой
  платформы — плюс Git на Windows, — и этот тест утверждает, что Node.js оттуда
  недостижим.
- **Decision:** ADR-0006, ADR-0008
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/bundled_tools.rs`
- **Check:** `ci-test` — `tests/ci/test_smoke_unica_bootstrap.py`
- **Check:** `release-gate` — `scripts/ci/smoke-unica-bootstrap.py`
- **Scope:** packaged, runtime, release

### REQ-REL-INSTALL-ONCE — Повторный запуск переиспользует уже опубликованный runtime

- **Rule:** Второй запуск для той же версии плагина и той же целевой платформы
  хоста переиспользует runtime, уже опубликованный под этой записью кеша, вместо
  того чтобы скачивать его снова, а параллельные запуски публикуют его ровно
  один раз, поэтому загрузка и публикация оплачиваются один раз на версию и
  цель.
- **Decision:** ADR-0008
- **Check:** `ci-test` — `crates/unica-bootstrap/tests/runtime_install.rs`
- **Scope:** packaged, runtime

### REQ-REL-NO-SILENT-STALL — Релиз не умеет застревать молча

- **Rule:** Публикация идёт одним линейным конвейером от подписанного
  source-тега: каталог переводится только за существующим тегом маркетплейса и
  зелёными потребительскими установками, порядок ступеней закреплён
  зависимостями джоб, а не дисциплиной человека. Точки, где конвейер может
  остановиться молча, не существует: отказ любой ступени — красный прогон,
  привязанный к тегу релиза, каталог остаётся на прежней версии, и повторный
  запуск всего конвейера идемпотентно продолжает публикацию.
- **Decision:** ADR-0008, ADR-0068
- **Check:** `ci-test` — `tests/ci/test_unica_workflow.py`
- **Check:** `doc-assert` — `tests/ci/test_product_contracts.py`
- **Scope:** release

### REQ-REL-REAL-CONFIG-GATE — Релиз-кандидат оценивается на настоящей конфигурации

- **Rule:** Каждый релиз-кандидат прогоняет сценарии публичных инструментов на
  закреплённой по версии настоящей конфигурации 1С и выдаёт машиночитаемый
  отчёт, называющий каждый сценарий, его статус и его длительность; упавший
  блокирующий сценарий проваливает оценку.
- **Decision:** ADR-0008, ADR-0055
- **Check:** `guard-script` — `scripts/ci/release-assessment.py`
- **Check:** `ci-test` — `tests/ci/test_release_assessment.py`
- **Scope:** ci, release
