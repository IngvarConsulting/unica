# План исправления домена XDTO после ревью PR #287

> Для выполнения использовать последовательную TDD-разработку в head-ветке
> существующего PR #287. После каждой задачи сначала приложить red-тест, затем
> минимальную реализацию и доказательство green.

**Цель:** реализовать полный утверждённый контракт issue #279 без обхода
логической адресации, форматных и support-guard, сохранив точечный writer и
синхронную публичную поверхность.

**Архитектура:** единый план XDTO строится из закрытой логической цели и
семантической модели исходного документа. Preview возвращает этот план, apply
привязывает те же доказательства к `CompileTransaction` и публикует одну
точечную замену после перевалидации. JSON Schema, runtime validation, typed
result, skill, specs, provenance и tool-surface описывают тот же контракт.

**Стек:** Rust, `roxmltree`, `serde`/`serde_json`, существующие
`platform_xml_source_targets`, `CompileTransaction`, Python CI-тесты и MCP
smoke.

---

## Task 1. Свести архитектурный контракт и актуальный `main`

**Файлы:**

- удалить: `spec/decisions/0023-xdto-package-domain.md`;
- создать: `spec/decisions/0024-xdto-package-domain.md`;
- изменить: `spec/decisions/README.md`;
- изменить: `spec/architecture/invariants.md`;
- создать: `docs/design/2026-08-02-xdto-package-domain-review-fixes-design.md`;
- проверить: `tests/ci/test_architecture_registry.py`;
- проверить: `tests/ci/test_design_documents.py`.

1. Перенумеровать только PR-локальную запись после занятого в `main` ADR-0023.
2. Вернуть в ADR-0024 полный контракт issue #279, а не сокращённую версию PR.
3. Перенаправить выведенный XDTO-инвариант и индекс решений на ADR-0024.
4. Запустить:

   `python3.12 -m unittest tests.ci.test_architecture_registry tests.ci.test_design_documents`

5. Ожидаемый green: индекс синхронен, номера уникальны, design ссылается на
   существующую принятую запись.
6. Коммит: `docs(xdto): restore approved package domain contract`.

## Task 2. Закрыть логическую цель транзакционными guards

**Файлы:**

- изменить: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`;
- изменить при необходимости: `crates/unica-coder/src/infrastructure/native_operations/common.rs`;
- изменить: `crates/unica-coder/src/infrastructure/format_guard.rs`;
- тестировать: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`;
- тестировать: `crates/unica-coder/src/infrastructure/format_guard.rs`.

1. Добавить red-тесты: XDTO descriptor `HandlerResolved` реально проходит
   format guard; смена descriptor/source identity/support/preimage между plan и
   commit отклоняется; путь вне sourceSet отклоняется.
2. Заменить ручное разрешение корня на
   `resolve_platform_xml_target(SourceTarget, TargetKindPolicy::Any)` и хранить
   закрытую ручку descriptor вместе с ресурсом `Ext/Package.bin`.
3. Перевести apply с `single_file_publisher` на `CompileTransaction`:
   `replace_bytes`, перевалидация logical target, привязка format-owner и
   support evidence, containment, commit.
4. Добавить XDTO-ветвь в `handler_resolved_format_paths`; пустой список для
   XDTO считается ошибкой контракта, а не Allow.
5. Запустить:

   `cargo test -p unica-coder xdto_guard -- --test-threads=1`

   `cargo test -p unica-coder format_guard -- --test-threads=1`

6. Коммит: `fix(xdto): bind package writes to logical target guards`.

## Task 3. Ввести семантическую модель и валидатор XDTO

**Файлы:**

- создать: `crates/unica-coder/src/infrastructure/native_operations/xdto/model.rs`;
- создать: `crates/unica-coder/src/infrastructure/native_operations/xdto/validation.rs`;
- изменить: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`.

1. Добавить red-тесты для корня/namespace, порядка секций, NCName,
   уникальности типов/свойств, допустимых контейнеров, nested object `typeDef`,
   объявленных QName-префиксов, imports, локальных type/base-ссылок и запрета
   удаления используемого типа.
2. Построить компактную модель со span исходного текста, namespace context,
   imports, типами, properties и рекурсивными `typeDef`.
3. Возвращать типизированные findings со стабильными кодами и location; отдельно
   вычислять разность before/after, чтобы прежний незатронутый дефект не
   блокировал план.
4. Запустить:

   `cargo test -p unica-coder xdto_validation -- --test-threads=1`

5. Коммит: `feat(xdto): validate package grammar and type references`.

## Task 4. Сделать точечный writer грамматически корректным

**Файлы:**

- создать: `crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs`;
- изменить: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`;
- тестировать: `crates/unica-coder/src/infrastructure/native_operations/xdto/writer.rs`.

1. Добавить red-тесты: `valueType` перед `objectType`; первый property в
   self-closing `objectType`; nested `typeDef`; BOM, LF и CRLF; без лишней
   пустой строки; неизменность всех байтов вне одного диапазона; строгий
   `propertyPath`.
2. Добавить red-тесты семантики дублей: идентичная сущность даёт no-op и
   `duplicate_*`, конфликтующая — blocking `duplicate_*`.
3. Реализовать один patch-range на операцию. Для вставки выводить EOL и отступ
   из контейнера/соседа; для self-closing заменять только закрывающий `/>` и
   добавляемый child; для удаления включать только whitespace выбранного узла.
4. Прогнать writer-тесты и общий XDTO-фильтр:

   `cargo test -p unica-coder xdto_writer -- --test-threads=1`

   `cargo test -p unica-coder xdto -- --test-threads=1`

5. Коммит: `fix(xdto): preserve package text while applying valid edits`.

## Task 5. Синхронизировать schema, info и события

**Файлы:**

- изменить: `crates/unica-coder/src/application/tool_contracts.rs`;
- изменить: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`;
- изменить: `crates/unica-coder/src/infrastructure/native_operations/xdto.rs`;
- изменить при необходимости: `crates/unica-coder/src/application/source_navigation.rs`;
- изменить: `crates/unica-coder/src/application/mod.rs`.

1. Добавить red JSON-Schema/runtime тесты для пяти взаимоисключающих операций:
   обязательные поля, запрет несовместимых полей, неполный `add-property`,
   недопустимый path и неизвестная операция.
2. Добавить red-тесты `info`: summary/imports/counts, детальный type/nested
   `typeDef`, addressable owner + unaddressable child, limit и request-bound
   cursor с неверным/чужим cursor.
3. Ввести сериализуемые структуры info/edit/findings вместо ad-hoc `Value` и
   подключить их к typed-result boundary.
4. Использовать общий либо выделенный request-bound cursor с максимумом 50.
5. Согласовать события: изменяющий preview проектирует событие без записи,
   apply публикует его после commit, no-op не проектирует и не публикует.
6. Запустить:

   `cargo test -p unica-coder xdto_contract -- --test-threads=1`

   `cargo test -p unica-coder xdto_info -- --test-threads=1`

   `cargo test -p unica-coder xdto_events -- --test-threads=1`

7. Коммит: `feat(xdto): complete typed contract and pagination`.

## Task 6. Доказать формат, corpus и публичную упаковку

**Файлы:**

- изменить: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`;
- изменить/добавить fixtures: `crates/unica-coder/tests/fixtures/format-8-3-27/`;
- изменить: `plugins/unica/references/specs/1c-configuration-spec.md`;
- изменить: `plugins/unica/references/specs/format-index.md`;
- изменить: `plugins/unica/skills/xdto/SKILL.md`;
- изменить: `spec/provenance/skill-upstreams.json`;
- изменить: `spec/architecture/tool-surface-review.json`;
- регенерировать: `spec/architecture/tool-surface.md`;
- изменить при необходимости: `tests/ci/test_unica_skills.py`;
- изменить при необходимости: `tests/ci/test_reference_format_profile.py`;
- изменить при необходимости: `scripts/ci/smoke-unica-mcp.py`.

1. Добавить red corpus-тест с classifier namespace
   `{http://v8.1c.ru/8.1/xdto}package`; убедиться, что он не находится только в
   ignored generator.
2. Зафиксировать в спецификациях `Package.bin`, секции/order, QName/import и
   допустимые контейнеры, подтверждённые fixture/donor corpus.
3. Исправить provenance: назвать донора и adapted/material relation, не
   объявлять заимствованный workflow полностью `unicaOwnedSkills`.
4. Проверить skill-путь info → preview → apply, отсутствие validate/direct
   script и паритет исходного/упакованного MCP.
5. Добавить две XDTO-записи в review ledger и регенерировать tool-surface
   собранным бинарём по help генератора.
6. Запустить:

   `cargo test -p unica-coder --test format_8_3_27_xml_corpus -- --test-threads=1`

   `python3.12 -m unittest tests.ci.test_reference_format_profile tests.ci.test_unica_skills tests.ci.test_skill_provenance tests.ci.test_tool_surface_ledger`

   `python3.12 scripts/ci/smoke-unica-mcp.py`

7. Коммит: `test(xdto): cover package corpus and public boundary`.

## Task 7. Полная проверка и обновление PR

1. Получить актуальные `origin/main` и head fork; подтвердить отсутствие нового
   расхождения ADR-номера и неожиданных коммитов в удалённой head-ветке.
2. Запустить форматирование и полный набор в затронутой области:

   `cargo fmt --all -- --check`

   `cargo test -p unica-coder -- --test-threads=1`

   `python3.12 -m unittest discover -s tests/ci -p 'test_*.py'`

3. Запустить package/smoke проверки, названные ADR-0024 и XDTO-инвариантом.
4. Просмотреть `git diff --check`, итоговый diff против `origin/main` и все
   unresolved review threads.
5. Отдать каждый task-коммит независимому code-review агенту; исправить все
   blocking/high замечания и повторить проверки.
6. Force-push с lease в существующую head-ветку PR, не создавать дочерний PR.
7. Ответить в PR кратким mapping «замечание → коммит/тест», разрешить только
   действительно закрытые threads и дождаться GitHub checks.
