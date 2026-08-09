# Объектная целостность на входе `meta` — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `unica.meta.add` и `unica.meta.edit` отказывают, если итоговое состояние вызова оставляет объект семантически неприменимым, а инструмент перестаёт придумывать содержимое за вызывающего.

**Architecture:** Закрытая таблица условий живёт в домене и не знает про XML. Чтение дескриптора живёт в инфраструктуре. Единственная точка проверки — `build_typed_operation_post_image`, которую вызывают оба пути мутации и которая отрабатывает до регистрации транзакции.

**Tech Stack:** Rust 2021, `roxmltree`, `cargo test`, точный платформенный гейт `scripts/dev/verify-8-3-27-platform.py` на установке 8.3.27.2074.

## Global Constraints

- Владелец решения — ADR-0030, проектные основания — `docs/design/2026-08-08-meta-object-integrity-design.md`.
- Запись в таблицу заводится только под наблюдённое сообщение платформы и несёт его дословно. Условие, выведенное из рассуждения, в таблицу не попадает.
- Проверка выполняется на итоговом состоянии вызова, а не на каждой операции.
- Нарушение сообщается через `dryRun` и блокирует применение; отдельный инструмент валидации не заводится.
- Сначала падающий тест, потом исправление. Тест запускается и подтверждается его падение именно по причине дефекта.
- Формат проектной записки и существование объявленной записи решения проверяет `tests/ci/test_design_documents.py`.

---

### Task 1: Таблица условий в домене

**Files:**
- Create: `crates/unica-coder/src/domain/metadata/integrity.rs`
- Modify: `crates/unica-coder/src/domain/metadata/mod.rs:1-11`

**Interfaces:**
- Consumes: `MetadataKind` из `crates/unica-coder/src/domain/metadata/types.rs`, `MetaCollection` из `operations.rs`, `MetaPropertyKey` из `properties.rs`.
- Produces: `MetaObjectRequirement`, `MetaObjectIntegrityRule`, `META_OBJECT_INTEGRITY_RULES`, `meta_object_integrity_rules(kind: MetadataKind) -> impl Iterator<Item = &'static MetaObjectIntegrityRule>`.

- [ ] **Step 1: Write the failing test**

В конец `crates/unica-coder/src/domain/metadata/integrity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_quotes_the_platform_message_it_was_derived_from() {
        assert!(!META_OBJECT_INTEGRITY_RULES.is_empty());
        for rule in META_OBJECT_INTEGRITY_RULES {
            assert!(!rule.kinds.is_empty(), "rule without kinds");
            assert!(
                !rule.platform_message.trim().is_empty(),
                "rule without the platform message it was derived from"
            );
        }
    }

    #[test]
    fn first_table_covers_the_two_observed_platform_rejections() {
        let registers = meta_object_integrity_rules(MetadataKind::InformationRegister)
            .collect::<Vec<_>>();
        assert_eq!(registers.len(), 1);
        assert_eq!(
            registers[0].platform_message,
            "Register without dimensions, resources, and attributes"
        );
        assert!(matches!(
            registers[0].requirement,
            MetaObjectRequirement::AnyCollectionNonEmpty(_)
        ));

        let services = meta_object_integrity_rules(MetadataKind::WebService).collect::<Vec<_>>();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].platform_message, "Empty name space");

        assert_eq!(meta_object_integrity_rules(MetadataKind::Catalog).count(), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p unica-coder --lib integrity::tests`
Expected: FAIL — модуль `integrity` не объявлен, сборка не проходит.

- [ ] **Step 3: Write minimal implementation**

В начало `crates/unica-coder/src/domain/metadata/integrity.rs`:

```rust
use super::{MetaCollection, MetaPropertyKey, MetadataKind};

/// Условие, доказуемое по самому объекту.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaObjectRequirement {
    /// Непусто хотя бы одно из перечисленных коллекций.
    AnyCollectionNonEmpty(&'static [MetaCollection]),
    /// Корневое свойство непусто.
    PropertyNonEmpty(MetaPropertyKey),
}

/// Запись заводится только под наблюдённое сообщение платформы: условие,
/// выведенное из рассуждения, проверить нечем, и запретить законный объект оно
/// способно так же, как пропустить сломанный (ADR-0030).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MetaObjectIntegrityRule {
    pub(crate) kinds: &'static [MetadataKind],
    pub(crate) requirement: MetaObjectRequirement,
    pub(crate) platform_message: &'static str,
}

pub(crate) const META_OBJECT_INTEGRITY_RULES: &[MetaObjectIntegrityRule] = &[
    MetaObjectIntegrityRule {
        kinds: &[
            MetadataKind::InformationRegister,
            MetadataKind::AccumulationRegister,
        ],
        requirement: MetaObjectRequirement::AnyCollectionNonEmpty(&[
            MetaCollection::Dimensions,
            MetaCollection::Resources,
            MetaCollection::Attributes,
        ]),
        platform_message: "Register without dimensions, resources, and attributes",
    },
    MetaObjectIntegrityRule {
        kinds: &[MetadataKind::WebService],
        requirement: MetaObjectRequirement::PropertyNonEmpty(MetaPropertyKey::Namespace),
        platform_message: "Empty name space",
    },
];

pub(crate) fn meta_object_integrity_rules(
    kind: MetadataKind,
) -> impl Iterator<Item = &'static MetaObjectIntegrityRule> {
    META_OBJECT_INTEGRITY_RULES
        .iter()
        .filter(move |rule| rule.kinds.contains(&kind))
}
```

В `crates/unica-coder/src/domain/metadata/mod.rs` добавить `mod integrity;` после `mod diagnostics;` и `pub(crate) use integrity::*;` после `pub(crate) use diagnostics::*;`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p unica-coder --lib integrity::tests`
Expected: PASS, 2 теста.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/metadata/integrity.rs crates/unica-coder/src/domain/metadata/mod.rs
git commit -m "feat(meta): завести закрытую таблицу условий объектной целостности"
```

---

### Task 2: Проверка дескриптора и отказ на входе `add` и `edit`

**Files:**
- Create: `crates/unica-coder/src/infrastructure/native_operations/meta/integrity_check.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs:657`

**Interfaces:**
- Consumes: `meta_object_integrity_rules`, `MetaObjectRequirement` из Task 1; `meta_info_child` из `meta/xml_model.rs`; `MetaDiagnostic`, `MetaDiagnosticCode` из домена.
- Produces: `check_meta_object_integrity(kind: MetadataKind, descriptor: &[u8]) -> Result<(), MetaDiagnostic>`.

- [ ] **Step 1: Write the failing test**

В конец `crates/unica-coder/src/infrastructure/native_operations/meta/integrity_check.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_REGISTER: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><InformationRegister><Properties><Name>R</Name></Properties><ChildObjects/></InformationRegister></MetaDataObject>"#;
    const FILLED_REGISTER: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><InformationRegister><Properties><Name>R</Name></Properties><ChildObjects><Resource><Properties><Name>Price</Name></Properties></Resource></ChildObjects></InformationRegister></MetaDataObject>"#;
    const EMPTY_NAMESPACE: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>S</Name><Namespace/></Properties></WebService></MetaDataObject>"#;
    const FILLED_NAMESPACE: &[u8] = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>S</Name><Namespace>urn:corpus</Namespace></Properties></WebService></MetaDataObject>"#;

    #[test]
    fn register_without_any_child_collection_is_refused_with_the_platform_message() {
        let error = check_meta_object_integrity(MetadataKind::InformationRegister, EMPTY_REGISTER)
            .expect_err("empty register must be refused");
        assert_eq!(error.code, MetaDiagnosticCode::ValidationFailed);
        assert!(
            error.message.contains("Register without dimensions, resources, and attributes"),
            "{error:?}"
        );
        assert!(error.message.contains("dimensions"), "{error:?}");
    }

    #[test]
    fn register_with_one_resource_is_accepted() {
        check_meta_object_integrity(MetadataKind::InformationRegister, FILLED_REGISTER).unwrap();
    }

    #[test]
    fn web_service_namespace_must_be_non_empty() {
        check_meta_object_integrity(MetadataKind::WebService, EMPTY_NAMESPACE)
            .expect_err("empty namespace must be refused");
        check_meta_object_integrity(MetadataKind::WebService, FILLED_NAMESPACE).unwrap();
    }

    #[test]
    fn kinds_without_a_rule_are_never_refused() {
        check_meta_object_integrity(MetadataKind::Catalog, EMPTY_REGISTER).unwrap();
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p unica-coder --lib integrity_check::tests`
Expected: FAIL — модуль не объявлен, сборка не проходит.

- [ ] **Step 3: Write minimal implementation**

В начало `crates/unica-coder/src/infrastructure/native_operations/meta/integrity_check.rs`:

```rust
use roxmltree::{Document, Node};

use super::xml_model::meta_info_child;
use crate::domain::metadata::{
    meta_object_integrity_rules, MetaCollection, MetaDiagnostic, MetaDiagnosticCode,
    MetaObjectRequirement, MetaPropertyKey, MetadataKind,
};

/// Платформа принимает такой дескриптор как документ и отвергает как объект
/// конфигурации, поэтому условие проверяется здесь, а не схемой (ADR-0030).
pub(crate) fn check_meta_object_integrity(
    kind: MetadataKind,
    descriptor: &[u8],
) -> Result<(), MetaDiagnostic> {
    let mut rules = meta_object_integrity_rules(kind).peekable();
    if rules.peek().is_none() {
        return Ok(());
    }
    let text = std::str::from_utf8(descriptor)
        .map_err(|error| validation_failed(format!("descriptor is not UTF-8: {error}")))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| validation_failed(format!("descriptor is not valid XML: {error}")))?;
    let object = document
        .root_element()
        .children()
        .find(Node::is_element)
        .ok_or_else(|| validation_failed("descriptor has no metadata object"))?;

    for rule in rules {
        let satisfied = match rule.requirement {
            MetaObjectRequirement::AnyCollectionNonEmpty(collections) => {
                let children = meta_info_child(object, "ChildObjects");
                collections.iter().any(|collection| {
                    children.is_some_and(|children| {
                        children.children().any(|child| {
                            child.is_element()
                                && child.tag_name().name() == collection_element_name(*collection)
                        })
                    })
                })
            }
            MetaObjectRequirement::PropertyNonEmpty(property) => meta_info_child(object, "Properties")
                .and_then(|properties| meta_info_child(properties, property_element_name(property)))
                .and_then(|node| node.text())
                .is_some_and(|value| !value.trim().is_empty()),
        };
        if !satisfied {
            return Err(validation_failed(format!(
                "{} rejects this object: {}. Provide it through `operations` on the same call: {}",
                kind.as_str(),
                rule.platform_message,
                requirement_hint(rule.requirement)
            )));
        }
    }
    Ok(())
}

fn collection_element_name(collection: MetaCollection) -> &'static str {
    match collection {
        MetaCollection::Dimensions => "Dimension",
        MetaCollection::Resources => "Resource",
        MetaCollection::Attributes => "Attribute",
        MetaCollection::TabularSections => "TabularSection",
        MetaCollection::EnumValues => "EnumValue",
        MetaCollection::Columns => "Column",
        MetaCollection::Forms => "Form",
        MetaCollection::Templates => "Template",
        MetaCollection::Commands => "Command",
    }
}

fn property_element_name(property: MetaPropertyKey) -> &'static str {
    match property {
        MetaPropertyKey::Namespace => "Namespace",
        // Таблица содержит только `Namespace`; новый вид свойства заводится
        // вместе со своим именем элемента, а не молчаливым умолчанием.
        other => panic!("integrity rule references a property without an element name: {other:?}"),
    }
}

fn requirement_hint(requirement: MetaObjectRequirement) -> String {
    match requirement {
        MetaObjectRequirement::AnyCollectionNonEmpty(collections) => format!(
            "add at least one of {}",
            collections
                .iter()
                .map(|collection| collection.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        MetaObjectRequirement::PropertyNonEmpty(property) => {
            format!("set a non-empty `{}`", property_element_name(property))
        }
    }
}

fn validation_failed(message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
}
```

Объявить модуль в `crates/unica-coder/src/infrastructure/native_operations/meta/mod.rs`: `mod integrity_check;` рядом с остальными и `pub(super) use integrity_check::check_meta_object_integrity;`.

Если `MetaPropertyKey` не имеет `as_str`, заменить рукав `other => other.as_str()` на явный `other => unreachable!("property without an element name: {other:?}")`, потому что таблица содержит только `Namespace`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p unica-coder --lib integrity_check::tests`
Expected: PASS, 4 теста.

- [ ] **Step 5: Встроить проверку в общую точку**

В `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`, в конце `build_typed_operation_post_image`, непосредственно перед возвратом `Ok(TypedOperationPostImage { ... })`:

```rust
    // Итоговое состояние вызова, а не каждая операция: замена единственного
    // измерения через `remove` вместе с `add` остаётся законной (ADR-0030).
    check_meta_object_integrity(kind, &descriptor)?;
```

Импорт: `use super::integrity_check::check_meta_object_integrity;`. Вид берётся из уже разрешённой цели в этой функции; если он там не связан, извлечь его из `target` тем же способом, каким функция уже определяет владельца.

- [ ] **Step 6: Написать падающие тесты обоих инструментов**

В `crates/unica-coder/src/application/meta_add_surface_tests.rs`:

```rust
#[test]
fn meta_add_refuses_a_register_without_operations_and_names_what_is_missing() {
    let fixture = add_fixture();
    let result = fixture.add("InformationRegister", "Prices", None);

    assert!(!result.ok, "{result:?}");
    let message = result.errors.join(" ");
    assert!(
        message.contains("Register without dimensions, resources, and attributes"),
        "{message}"
    );
}

#[test]
fn meta_add_accepts_a_register_whose_operations_make_it_coherent() {
    let fixture = add_fixture();
    let result = fixture.add(
        "InformationRegister",
        "Prices",
        Some(json!([{
            "op": "add",
            "collection": "resources",
            "elements": [{"name": "Price", "type": {"variants": [{"kind": "number", "digits": 15, "fractionDigits": 2}]}}]
        }])),
    );

    assert!(result.ok, "{result:?}");
}
```

Имена `add_fixture` и `add` взять фактические из этого файла: он уже строит рабочее пространство и вызывает `unica.meta.add`. Если конструктор фикстуры не принимает `operations`, добавить параметр, а существующие вызовы передают `None`.

И там же, рядом с ними, три теста про итоговое состояние и `dryRun`:

```rust
#[test]
fn meta_edit_may_replace_the_only_dimension_within_one_call() {
    // Итоговое состояние вызова целостно, поэтому промежуточная пустота между
    // remove и add нарушением не считается (ADR-0030).
    let fixture = add_fixture();
    fixture
        .add(
            "InformationRegister",
            "Prices",
            Some(json!([{
                "op": "add", "collection": "dimensions",
                "elements": [{"name": "Item", "type": {"variants": [{"kind": "string", "length": 50, "allowedLength": "variable"}]}}]
            }])),
        )
        .assert_ok();

    let result = fixture.edit(
        "InformationRegister.Prices",
        json!([
            {"op": "remove", "collection": "dimensions", "elements": [{"name": "Item"}]},
            {"op": "add", "collection": "dimensions",
             "elements": [{"name": "Product", "type": {"variants": [{"kind": "string", "length": 50, "allowedLength": "variable"}]}}]}
        ]),
    );

    assert!(result.ok, "{result:?}");
}

#[test]
fn meta_edit_refuses_removing_the_last_child_of_a_register() {
    let fixture = add_fixture();
    fixture
        .add(
            "InformationRegister",
            "Prices",
            Some(json!([{
                "op": "add", "collection": "dimensions",
                "elements": [{"name": "Item", "type": {"variants": [{"kind": "string", "length": 50, "allowedLength": "variable"}]}}]
            }])),
        )
        .assert_ok();

    let result = fixture.edit(
        "InformationRegister.Prices",
        json!([{"op": "remove", "collection": "dimensions", "elements": [{"name": "Item"}]}]),
    );

    assert!(!result.ok, "{result:?}");
    assert!(
        result.errors.join(" ").contains("Register without dimensions, resources, and attributes"),
        "{result:?}"
    );
}

#[test]
fn dry_run_reports_the_violation_without_writing() {
    let fixture = add_fixture();
    let preview = fixture.add_dry_run("InformationRegister", "Prices", None);

    assert!(!preview.ok, "{preview:?}");
    assert!(
        preview.errors.join(" ").contains("Register without dimensions, resources, and attributes"),
        "{preview:?}"
    );
    assert!(!fixture.object_exists("InformationRegister.Prices"));
}
```

Имена `edit`, `add_dry_run`, `assert_ok` и `object_exists` взять фактические из
фикстуры этого файла. Если какого-то из них нет, добавить тонкую обёртку над уже
существующим вызовом `call_tool` — новой логики в фикстуру не вносить.

- [ ] **Step 7: Run tests to verify they fail, then pass**

Run: `cargo test -p unica-coder --lib meta`
Expected: сначала FAIL на новых тестах отказа, после Step 5 — PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta/ crates/unica-coder/src/application/
git commit -m "feat(meta): отказывать при нарушении объектной целостности на входе add и edit"
```

---

### Task 3: Снять автодополнение содержимого

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog.rs:664-686`
- Test: `crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog_tests.rs`

**Interfaces:**
- Consumes: `minimal_metadata_xml_for_tests` из `template_catalog.rs`.
- Produces: ничего нового; ветка `AccountingRegister | CalculationRegister` в `minimal_metadata_xml` исчезает.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn minimal_templates_never_invent_content() {
    // Выдуманный ресурс — молчаливое решение за вызывающего, которое остаётся
    // мусором в выгрузке (ADR-0030).
    for kind in [
        MetadataKind::AccountingRegister,
        MetadataKind::CalculationRegister,
    ] {
        let (xml, _) = minimal_metadata_xml_for_tests(kind, "Evidence").unwrap();
        assert!(
            !xml.contains("<Resource"),
            "{} invented a resource: {xml}",
            kind.as_str()
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p unica-coder --lib minimal_templates_never_invent_content`
Expected: FAIL — `AccountingRegister invented a resource`.

- [ ] **Step 3: Write minimal implementation**

В `minimal_metadata_xml` заменить всю конструкцию `if matches!(kind, AccountingRegister | CalculationRegister) { … } else if kind_declares_child_objects(kind) { … }` на:

```rust
    if kind_declares_child_objects(kind) {
        lines.push("\t\t<ChildObjects/>".to_string());
    }
```

Удалить осиротевшие `MetadataAttributeTemplate` и вызов `emit_meta_register_field` в этой функции. Сама `emit_meta_register_field` остаётся: её используют операции.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p unica-coder --lib meta`
Expected: PASS. Тесты, ожидавшие выдуманный ресурс, обновить по фактическому выводу — они закрепляли снимаемое поведение.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta/
git commit -m "fix(meta): не придумывать ресурс регистра за вызывающего"
```

---

### Task 4: Корпус, гейт и пополнение таблицы

**Files:**
- Modify: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs:1300-1315` (`seed_metadata`)
- Modify: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs:2115-2129` (аргументы цели `meta-compile-*`)

**Interfaces:**
- Consumes: `unica.meta.add` с `operations` из Task 2.
- Produces: корпус, в котором регистры и веб-сервис целостны.

- [ ] **Step 1: Провести определения корпуса в вызов**

`seed_metadata` и построитель аргументов `meta-compile-*` сегодня читают из
определения только `type` и `name`, а `dimensions`, `resources`, `periodicity`,
`namespace` игнорируют — это остатки эпохи `meta.compile` с файловым JSON DSL.

Добавить рядом с `meta_definition` функцию, отдающую `operations` по ветке, и
вставлять её результат в аргументы обоих путей:

```rust
fn meta_operations(branch: &str) -> Option<Value> {
    let string50 = json!({"variants": [{"kind": "string", "length": 50, "allowedLength": "variable"}]});
    let number = |digits: u32, fraction: u32| {
        json!({"variants": [{"kind": "number", "digits": digits, "fractionDigits": fraction}]})
    };
    match branch {
        "InformationRegister" => Some(json!([
            {"op": "add", "collection": "dimensions",
             "elements": [{"name": "Item", "type": string50}]},
            {"op": "add", "collection": "resources",
             "elements": [{"name": "Price", "type": number(15, 2)}]}
        ])),
        "AccumulationRegister" => Some(json!([
            {"op": "add", "collection": "dimensions",
             "elements": [{"name": "Warehouse", "type": string50}]},
            {"op": "add", "collection": "resources",
             "elements": [{"name": "Quantity", "type": number(15, 3)}]}
        ])),
        "WebService" => Some(json!([
            {"op": "setProperties", "values": {"Namespace": "urn:corpus"}}
        ])),
        _ => None,
    }
}
```

В `seed_metadata` и в построителе аргументов `meta-compile-*` после вставки
`kind` и `name` добавить:

```rust
    if let Some(operations) = meta_operations(kind) {
        args.insert("operations".to_string(), operations);
    }
```

где `kind` — вид объекта (`case.branch` в построителе аргументов цели, поле
`type` определения в `seed_metadata`).

- [ ] **Step 2: Прогнать узкие тесты корпуса**

Run: `cargo test -p unica-coder --test format_8_3_27_xml_corpus`
Expected: PASS, все не-`ignored` тесты файла.

- [ ] **Step 3: Породить корпус**

```bash
UNICA_XML_CORPUS_DIR=<новый пустой каталог вне репозитория и вне $HOME> \
  cargo test -p unica-coder --test format_8_3_27_xml_corpus \
  generate_platform_xml_corpus -- --exact --ignored --nocapture --test-threads=1
```

- [ ] **Step 4: Прогнать точный гейт**

```bash
python scripts/dev/verify-8-3-27-platform.py \
  --ibcmd /opt/1cv8/8.3.27.2074/ibcmd \
  --corpus <каталог>/corpus-manifest.json --report <отчёт>.json
```

Гейт требует обвязки XDTO-домена из PR #383 и закреплённого дайджеста,
совпадающего с новым корпусом. Пока #383 не влит, а дайджест не пересчитан,
подставлять их временно и откатывать файл после прогона.

Expected: `meta-compile-information-register`, `meta-compile-accumulation-register`,
`meta-compile-web-service` и `cfe-patch-method-information-register-record-set-module`
переходят в `pass`.

- [ ] **Step 5: Пополнить таблицу по фактическому отказу**

Регистры бухгалтерии и расчёта проходят гейт сегодня за счёт снятого в Task 3
автодополнения. Если после снятия они отказывают, взять сообщение платформы из
отчёта дословно и добавить их в `META_OBJECT_INTEGRITY_RULES` вместе с тестом,
цитирующим это сообщение, и с `operations` в соответствующих случаях корпуса.
Если платформа их принимает — записи не заводить: условия у них нет.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/tests/format_8_3_27_xml_corpus.rs crates/unica-coder/src/domain/metadata/integrity.rs
git commit -m "test(corpus): выражать состав объектов через operations"
```

---

## Что этот план не закрывает

`meta-compile-business-process` и `meta-compile-document-journal` остаются
отклонёнными: `Task` и `RegisteredDocuments` не выражаются ни свойством, ни
одним из четырёх отношений закрытого набора. Условия межобъектные, поэтому вход
`meta` их не требует, но и будущая cf-проверка их не примет. Расширение закрытых
доменов — отдельное решение.

`xdto-add-nested-property` (`Configuration` без `InternalInfo`) к этой работе не
относится.
