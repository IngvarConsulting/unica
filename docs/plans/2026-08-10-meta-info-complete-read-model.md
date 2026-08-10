# План реализации полной read-model `unica.meta.info`

> **Для Codex:** выполнять по `superpowers:executing-plans`, сохраняя цикл red → green → refactor для каждого обнаруженного дефекта.

- Date: `2026-08-10`
- Design: `docs/design/2026-08-10-meta-info-complete-read-model-design.md`
- Decision: `ADR-0047`
- Issues: `#293`, исторический сценарий `#274`

## Цель

Сделать `unica.meta.info` полной типизированной read-model для 23 поддерживаемых видов метаданных: отделить чтение свойств и типов от разрешений writer-а, вернуть подтверждённые составные структуры Constant, DefinedType, ChartOfCharacteristicTypes, ChartOfCalculationTypes, DocumentJournal, CalculationRegister, ScheduledJob, HTTPService и WebService, а затем закрепить полноту исполняемым профилем и fixture-матрицей.

## Архитектура

Общий конверт `MetaInfoData` остаётся совместимым, но пару `kind + details` сериализует закрытый `MetaInfoDetails`. Наблюдаемые типы читает отдельная `ObservedMetadataType`; явное fallible-сужение к существующей `MetadataType` не меняет публичную алгебру `meta.add/edit`. Корневые свойства читает независимый `MetaInfoPropertyProfile`, а `MetaInfoKindProfile` классифицирует применимые прямые узлы `Properties` и `ChildObjects` каждого вида. XML разбирается только в infrastructure; application получает готовую доменную read-model.

## Технологии и проверки

Rust, `serde`, `roxmltree`, существующий typed MCP envelope, Rust unit/integration tests и Python CI-contract tests. Реализация перебазирована на `main` после #427, #325 и #428: сохраняются независимая observation algebra ADR-0042, typed `predefinedItems` и read-only invocation mode.

### Задача 1. Закрепить пользовательские потери красными интеграционными тестами

**Файлы:**

- создать `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/mod.rs` только для подключения тестового модуля;
- добавить узкие XML fixtures в `tests/fixtures/platform_8_3_27/meta_info/edge/` для Constant, DefinedType, DocumentJournal, ScheduledJob, CalculationRegister, HTTPService и WebService; ChartOfCharacteristicTypes и ChartOfCalculationTypes закрепить канонической fixture-матрицей.

**Шаги:**

1. Для каждого fixture вызвать реальную проекцию descriptor image и проверить вручную выписанные JSON-литералы: `details.type`, `details.baseCalculationTypes`, `details.registeredDocuments`, `details.method`, целую schedule-тройку, HTTP templates/methods, Web operations/parameters и expanded QName.
2. Отдельным regression-тестом сохранить уже доставленный ADR-0042 контракт: `v8:UUID` наблюдается в `collections.*[].type` с `mutationCapability: editable`.
3. Запустить `cargo test -p unica-coder info_projection_tests -- --nocapture`; убедиться, что тесты компилируются после минимального объявления ожидаемой публичной формы и падают именно на отсутствующих значениях, до реализации parser-а.
4. Не ослаблять ожидания под текущий результат: ожидаемые значения выводятся из hand-checked fixtures, а не из template/emitter helper-а.

### Задача 2. Отделить наблюдаемые типы от writer-типов

**Файлы:**

- создать `crates/unica-coder/src/domain/metadata/observed_types.rs`;
- изменить `crates/unica-coder/src/domain/metadata/mod.rs`;
- изменить `crates/unica-coder/src/domain/metadata/results.rs`;
- создать `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`.

**Шаги:**

1. Сначала добавить падающие unit-тесты на `UUID`, TypeSet, квалификаторы и QName-независимый разбор ссылок.
2. Реализовать `ObservedMetadataType`/`ObservedMetadataTypeVariant` и единый read parser для `Type`.
3. Перевести `MetaElementData.type` на наблюдаемый тип, сохранив JSON существующих writer-совместимых вариантов.
4. Реализовать и протестировать `TryFrom<ObservedMetadataType> for MetadataType`: значение с `mutationCapability: editable`, включая UUID, проверяемо сужается; вручную построенное `readOnly` отклоняется. Сохранить публичные схемы мутаций ADR-0042.
5. Запустить узкие тесты типов и существующие `info_tests`.

### Задача 3. Ввести закрытый `details` и спроецировать простые kind-specific поля

**Файлы:**

- создать `crates/unica-coder/src/domain/metadata/info.rs`;
- изменить `crates/unica-coder/src/domain/metadata/results.rs`;
- изменить `crates/unica-coder/src/application/ports.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`.

**Шаги:**

1. Добавить красный сериализационный тест: каждый `MetadataKind::ALL` даёт совпадающий `kind` и обязательный `details`; несовпадающую пару нельзя сконструировать.
2. Реализовать 23 варианта `MetaInfoDetails`, включая пустые варианты без дополнительных полей.
3. Реализовать Constant/DefinedType `type`, ScheduledJob `method` и CalculationRegister `schedule`.
4. Для method и schedule обеспечить tri-state: неприменимое поле отсутствует благодаря варианту enum; применимое недоказанное значение равно `null` и сопровождается диагностикой; целое доказанное значение сериализуется объектом.
5. Запустить `cargo test -p unica-coder meta_info_details -- --nocapture` и пользовательские regressions из задачи 1.

### Задача 4. Спроецировать HTTPService и WebService без утечки wire-деталей

**Файлы:**

- изменить `crates/unica-coder/src/domain/metadata/info.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`;
- изменить edge fixtures задачи 1 только если hand-check выявит ошибку fixture, а не для подгонки ожиданий.

**Шаги:**

1. Красными тестами закрепить `HTTPService.details.urlTemplates[].methods[]` с `name`, `template`, `httpMethod`, `handler`.
2. Красными тестами закрепить `WebService.details.xdtoPackages`, operations, return type, nillable/transactioned/procedure, parameters, transfer direction и parameter XDTO type.
3. Реализовать QName expansion через namespace URI + local name; XML prefix не сериализовать.
4. При неполной записи возвращать массив `null`, сохранять общую часть ответа и добавлять `validation_failed` с логическим `metadataPath` и публичным field path; не публиковать raw XML/физический путь.
5. Запустить все тесты `info_projection_tests`.

### Задача 5. Отвязать общие read-properties от writer allowlist

**Файлы:**

- создать `crates/unica-coder/src/domain/metadata/info_properties.rs`;
- изменить `crates/unica-coder/src/domain/metadata/mod.rs`;
- изменить `crates/unica-coder/src/domain/metadata/results.rs` при необходимости расширения read-only wire-типа;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`.

**Шаги:**

1. Добавить красный тест, в котором доказанное read-only корневое свойство отсутствует в `METADATA_PROPERTY_SPECS`, но обязано появиться в `data.properties`.
2. Реализовать закрытый `MetaInfoPropertyProfile` с read-side типом значения и маршрутом для каждого общего свойства; не использовать `METADATA_PROPERTY_SPECS` как источник разрешения чтения.
3. Сохранить прежние ключи и JSON-формы writer-подмножества отдельным regression-тестом.
4. Проверить, что список writer-свойств и публичные mutation schemas не изменились.

### Задача 6. Сделать полноту 23 видов исполняемым контрактом

**Файлы:**

- создать `tests/fixtures/platform_8_3_27/meta_info/manifest.json`;
- связать каждый `MetadataKind::ALL` с каноническим шаблоном и соответствующим exact-platform checkpoint; не копировать 23 результата того же emitter-а как ложное независимое свидетельство;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection.rs`;
- изменить `crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs`.

**Шаги:**

1. Manifest явно связывает 23 вида с каноническим descriptor case, именованным checkpoint и provenance подтверждённого platform corpus; 11 edge fixtures разделены на восемь feature-примеров и три route-coverage примера.
2. Реализовать закрытый `MetaInfoKindProfile`: каждый прямой узел `Properties`/`ChildObjects` маршрутизирован в identity, common property, relation, collection, `details` или закрытое форматное/дублирующее исключение.
3. Добавить guard: множество видов равно `MetadataKind::ALL`; каждый `platformCase` связан с точным executable case и своим kind; каждый semantic node классифицирован; каждый declared route наблюдается в canonical/edge fixture либо независимой manifest-матрице свойств/коллекций; `kind/details` совпадают.
4. Перед green намеренно добавить неизвестное свойство и дочерний объект в копию fixture и убедиться, что guard падает как `provider_unavailable`, а не молча пропускает узел.
5. Исправлять каждый найденный guard-ом пробел сначала отдельным failing regression, затем маршрутом/полем.

### Задача 7. Синхронизировать публичный контракт и архитектурное решение

**Файлы:**

- изменить `docs/design/2026-08-10-meta-info-complete-read-model-design.md` (`approved`, убрать найденный дубль ScheduledJob в fixture-матрице);
- добавить `spec/decisions/0047-polnaya-read-model-meta-info.md` (`accepted`),
  не переписывая уже попавшее в `main` предложение ADR-0041;
- изменить `spec/decisions/README.md`;
- изменить `spec/architecture/invariants.md` — добавить `INV-MCP-META-INFO-COVERAGE` с executable checks;
- изменить `spec/architecture/tool-surface.md`;
- изменить `spec/architecture/tool-surface-review.json`;
- изменить релевантные строки `spec/acceptance/format-profile-8-3-27.md`, `spec/acceptance/logical-source-addressing-and-resource-access.md`, `spec/acceptance/unica-mcp-validation.md`;
- изменить `plugins/unica/skills/meta-info/SKILL.md`;
- изменить/добавить поведенческие проверки в `tests/ci/test_meta_surface_contract.py` и при необходимости `tests/ci/test_unica_skills.py`.

**Шаги:**

1. Обновить документы по фактически реализованной форме, не обещая свойства вне fixture-матрицы.
2. В skill описать mandatory `details`, tri-state, observed type и примеры HTTP/Web/ScheduledJob без raw XML.
3. CI-тестами проверять потребительский контракт/исполняемый manifest, а не grep отдельной фразы документа.
4. Запустить Python contract suites, `check-architecture-sync.py --base origin/main` и `git diff --check`.

### Задача 8. Полная проверка, ревью и публикация отдельного PR

**Шаги:**

1. Запустить форматирование: `cargo fmt --all -- --check` (после `cargo fmt --all`, если требуется).
2. Запустить `cargo clippy --workspace --all-targets -- -D warnings`.
3. Запустить `cargo test --workspace -- --test-threads=1`.
4. Запустить релевантные Python suites и `python3.12 scripts/ci/check-architecture-sync.py --base origin/main`.
5. Проверить `git diff --check`, отсутствие raw XML/физических путей в JSON regressions и неизменность публичной mutation surface.
6. Выполнить self-review по diff и независимую проверку по `superpowers:requesting-code-review`; все найденные дефекты исправлять через red → green.
7. Намеренно закоммитить изменения, push ветки `codex/meta-info-complete-read-model`, открыть отдельный draft PR в `main` с `Closes #293`; #274 не закрывать повторно, а сослаться как на подтверждённый исторический сценарий.
