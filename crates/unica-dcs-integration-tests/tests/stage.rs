//! Запись кодека СКД в предложение workspace: постобраз схемы, метка
//! порядка байтов, язык по умолчанию и роли полей.
mod fixtures;
mod stand;

use fixtures::VERSIONS_SCHEMA as SCHEMA;
use serde_json::json;
use stand::{plan, stage_all, Stand};
use std::path::Path;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, IndexedPlanOperation};

#[test]
fn dcs_operations_transform_the_staged_schema_and_keep_its_byte_order_mark() {
    let fixture = Stand::new();
    let template_dir = fixture
        .source_dir()
        .join("Reports/Versions/Templates/Schema/Ext");
    std::fs::create_dir_all(&template_dir).unwrap();
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(SCHEMA.trim_start_matches('\u{feff}').as_bytes());
    std::fs::write(template_dir.join("Template.xml"), &bytes).unwrap();

    let admission = fixture.admission();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .dcs_mxl_planning_authority(&fixture.binding)
        .unwrap();
    let add_field = plan(
        "field.add",
        &json!({
            "at": "main:Report.Versions.Template.Schema.DataSet.НаборДанных1",
            "items": [{"dataPath": "Автор", "title": "Автор версии"}]
        }),
        0,
    )
    .unwrap();
    let clear_filter = plan(
        "filter.clear",
        &json!({"at": "main:Report.Versions.Template.Schema.Setting.Основной"}),
        1,
    )
    .unwrap();
    let structure = plan(
        "structure.set",
        &json!({
            "at": "main:Report.Versions.Template.Schema.Setting.Основной",
            "values": {"groupBy": ["ТипОбъекта"]}
        }),
        2,
    )
    .unwrap();
    let (staged, effects) = stage_all(
        staged,
        authority,
        &[
            IndexedPlanOperation::new(0, add_field),
            IndexedPlanOperation::new(1, clear_filter),
            IndexedPlanOperation::new(2, structure),
        ],
    )
    .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    // Clearing a filter the schema never had changes nothing.
    assert_eq!(effects.len(), 2);
    let changes = staged.planned_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].relative_path,
        Path::new("Reports/Versions/Templates/Schema/Ext/Template.xml")
    );
    let unica_source_contract::proposal::StagedFileState::Bytes(current) = &changes[0].current
    else {
        panic!("the schema keeps bytes");
    };
    assert!(current.starts_with(b"\xef\xbb\xbf"));
    let text = String::from_utf8(current[3..].to_vec()).unwrap();
    assert!(
        roxmltree::Document::parse(&text)
            .unwrap()
            .descendants()
            .any(|node| node.tag_name().name() == "dataPath" && node.text() == Some("Автор")),
        "{text}"
    );
    assert!(text.contains("Автор версии"), "{text}");
    assert!(text.contains("ТипОбъекта"), "{text}");
}
/// Lays the acceptance schema out as `Report.Versions.Template.Schema`
/// and plans `operations` on it.
fn plan_on_versions_schema(
    fixture: &Stand,
    operations: &[(&str, serde_json::Value)],
) -> Result<String, ApplyPlanError> {
    let template_dir = fixture
        .source_dir()
        .join("Reports/Versions/Templates/Schema/Ext");
    std::fs::create_dir_all(&template_dir).unwrap();
    std::fs::write(template_dir.join("Template.xml"), SCHEMA.as_bytes()).unwrap();
    let admission = fixture.admission();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .dcs_mxl_planning_authority(&fixture.binding)
        .unwrap();
    let operations = operations
        .iter()
        .enumerate()
        .map(|(index, (op, args))| {
            IndexedPlanOperation::new(
                index,
                plan(op, args, index).unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        })
        .collect::<Vec<_>>();
    let (staged, _) = stage_all(staged, authority, &operations)?;
    let changes = staged.planned_changes();
    let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current else {
        panic!("expected XML bytes");
    };
    Ok(std::str::from_utf8(bytes)
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string())
}
/// The translations of the element of `tag` named by its `key` child.
fn translations(xml: &str, tag: &str, key: &str, name: &str, text: &str) -> Vec<(String, String)> {
    let document = roxmltree::Document::parse(xml).unwrap();
    let owner = document
        .descendants()
        .find(|node| {
            node.tag_name().name() == tag
                && node
                    .children()
                    .any(|child| child.tag_name().name() == key && child.text() == Some(name))
        })
        .unwrap_or_else(|| panic!("{tag} {name} is missing\n{xml}"));
    let value = owner
        .children()
        .find(|child| child.tag_name().name() == text)
        .unwrap_or_else(|| panic!("{tag} {name} has no {text}\n{xml}"));
    value
        .children()
        .filter(|item| item.tag_name().name() == "item")
        .map(|item| {
            let part = |name: &str| {
                item.children()
                    .find(|child| child.tag_name().name() == name)
                    .and_then(|child| child.text())
                    .unwrap_or_default()
                    .to_string()
            };
            (part("lang"), part("content"))
        })
        .collect()
}
/// #1257: a plain-string title is written in the default language of the
/// source set; a set keeps the translations of the other languages.
#[test]
fn dcs_titles_write_the_configuration_default_language() {
    let fixture = Stand::new();
    fixture.use_english_only_language();
    let schema = "main:Report.Versions.Template.Schema";
    let data_set = format!("{schema}.DataSet.НаборДанных1");
    let xml = plan_on_versions_schema(
        &fixture,
        &[
            (
                "field.add",
                json!({"at": data_set, "items": [{"dataPath": "Автор", "title": "Version author"}]}),
            ),
            (
                "field.set",
                json!({"at": format!("{data_set}.Field.Количество"), "values": {"title": "Count"}}),
            ),
            (
                "parameter.add",
                json!({"at": schema, "items": [{"name": "Period", "title": "Period"}]}),
            ),
            (
                "calculatedField.add",
                json!({"at": schema, "items": [{"name": "Share", "expression": "1", "title": "Share"}]}),
            ),
            (
                "variant.add",
                json!({"at": schema, "items": [{"name": "Brief", "title": "Brief"}]}),
            ),
            (
                "variant.set",
                json!({"at": format!("{schema}.Setting.Основной"), "values": {"title": "Stored versions"}}),
            ),
        ],
    )
    .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    let en = |text: &str| ("en".to_string(), text.to_string());
    let ru = |text: &str| ("ru".to_string(), text.to_string());
    assert_eq!(
        translations(&xml, "field", "dataPath", "Автор", "title"),
        [en("Version author")]
    );
    assert_eq!(
        translations(&xml, "field", "dataPath", "Количество", "title"),
        [ru("Количество"), en("Count")],
        "the Russian translation is kept"
    );
    assert_eq!(
        translations(&xml, "parameter", "name", "Period", "title"),
        [en("Period")]
    );
    assert_eq!(
        translations(&xml, "calculatedField", "dataPath", "Share", "title"),
        [en("Share")]
    );
    assert_eq!(
        translations(&xml, "settingsVariant", "name", "Brief", "presentation"),
        [en("Brief")]
    );
    assert_eq!(
        translations(&xml, "settingsVariant", "name", "Основной", "presentation"),
        [
            ru("Количество и объем хранимых версий объектов"),
            en("Stored versions")
        ]
    );
    let content = "<v8:content>Количество и объем хранимых версий объектов</v8:content>";
    let start = SCHEMA[..SCHEMA.find(content).unwrap()]
        .rfind("<v8:item>")
        .unwrap();
    let end = SCHEMA[start..].find("</v8:item>").unwrap() + start + "</v8:item>".len();
    assert!(
        xml.contains(&SCHEMA[start..end]),
        "the kept translation keeps its bytes\n{xml}"
    );
}
/// The default language is read only for a plain-string text: other DCS
/// edits do not depend on it, a title does and is refused without it.
#[test]
fn dcs_edits_without_text_do_not_read_the_default_language() {
    let fixture = Stand::new();
    std::fs::remove_file(fixture.source_dir().join("Languages/Русский.xml")).unwrap();
    let data_set = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
    let xml = plan_on_versions_schema(
        &fixture,
        &[(
            "field.add",
            json!({"at": data_set, "items": [{"dataPath": "Автор", "title": {"en": "Author"}}]}),
        )],
    )
    .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    assert_eq!(
        translations(&xml, "field", "dataPath", "Автор", "title"),
        [("en".to_string(), "Author".to_string())],
        "an explicit translation needs no default language"
    );
    let error = plan_on_versions_schema(
        &fixture,
        &[(
            "field.set",
            json!({"at": format!("{data_set}.Field.Количество"), "values": {"title": "Count"}}),
        )],
    )
    .expect_err("a plain-string title needs the default language");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{error:?}");
    assert_eq!(error.path(), Some("ops[0].args.at"));
}
#[test]
fn typed_dcs_role_changes_the_existing_field_instead_of_a_skipped_renamed_target() {
    let fixture = Stand::new();
    let template_dir = fixture
        .source_dir()
        .join("Reports/Versions/Templates/Schema/Ext");
    std::fs::create_dir_all(&template_dir).unwrap();
    std::fs::write(template_dir.join("Template.xml"), SCHEMA.as_bytes()).unwrap();
    let admission = fixture.admission();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .dcs_mxl_planning_authority(&fixture.binding)
        .unwrap();
    let operation = plan(
        "fieldRole.set",
        &json!({
            "at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.РазмерДанных",
            "values":{"role":{"dimension":true}}
        }),
        0,
    )
    .unwrap();
    let (staged, effects) = stage_all(
        staged,
        authority,
        &[IndexedPlanOperation::new(0, operation)],
    )
    .unwrap();
    assert_eq!(
        effects.len(),
        1,
        "a success must change the named existing field"
    );
    let changes = staged.planned_changes();
    let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current else {
        panic!("expected XML bytes");
    };
    let xml = std::str::from_utf8(bytes)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(xml).unwrap();
    let namespace = unica_dcs_designer::schema::DCS_COMMON_NS;
    let field = document
        .descendants()
        .find(|node| {
            node.tag_name().name() == "field"
                && node.children().any(|child| {
                    child.tag_name().name() == "dataPath" && child.text() == Some("РазмерДанных")
                })
        })
        .unwrap();
    assert!(field
        .descendants()
        .any(|node| node.tag_name().namespace() == Some(namespace)
            && node.tag_name().name() == "dimension"
            && node.text() == Some("true")));
}
/// Поле с составным `dataPath` адресуется `Field.A\.B` (#1356):
/// `field.set` и `fieldRole.set` по этому адресу меняют именно его,
/// а не поле `A` и не несуществующую ветвь `B`.
#[test]
fn typed_dcs_field_set_changes_the_composite_field_by_its_escaped_address() {
    let fixture = Stand::new();
    let template_dir = fixture
        .source_dir()
        .join("Reports/Versions/Templates/Schema/Ext");
    std::fs::create_dir_all(&template_dir).unwrap();
    let composite = "<field xsi:type=\"DataSetFieldField\">\n\t\t\t<dataPath>ТипОбъекта.Представление</dataPath>\n\t\t\t<field>ТипОбъекта.Представление</field>\n\t\t</field>\n\t\t<field xsi:type=\"DataSetFieldField\">";
    std::fs::write(
        template_dir.join("Template.xml"),
        SCHEMA
            .replacen("<field xsi:type=\"DataSetFieldField\">", composite, 1)
            .as_bytes(),
    )
    .unwrap();
    let admission = fixture.admission();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .dcs_mxl_planning_authority(&fixture.binding)
        .unwrap();
    let at = r"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.ТипОбъекта\.Представление";
    let operations = [
        IndexedPlanOperation::new(
            0,
            plan(
                "field.set",
                &json!({"at": at, "values": {"title": "Тип (представление)"}}),
                0,
            )
            .unwrap(),
        ),
        IndexedPlanOperation::new(
            1,
            plan(
                "fieldRole.set",
                &json!({"at": at, "values": {"role": {"dimension": true}}}),
                1,
            )
            .unwrap(),
        ),
    ];
    let (staged, effects) = stage_all(staged, authority, &operations)
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
    assert!(!effects.is_empty());
    let changes = staged.planned_changes();
    let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current else {
        panic!("expected XML bytes");
    };
    let xml = std::str::from_utf8(bytes)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(xml).unwrap();
    let field_by_path = |path: &str| {
        document
            .descendants()
            .find(|node| {
                node.tag_name().name() == "field"
                    && node.children().any(|child| {
                        child.tag_name().name() == "dataPath" && child.text() == Some(path)
                    })
            })
            .unwrap_or_else(|| panic!("field {path}"))
    };
    let changed = field_by_path("ТипОбъекта.Представление");
    assert!(changed
        .descendants()
        .any(|node| node.text() == Some("Тип (представление)")));
    let namespace = unica_dcs_designer::schema::DCS_COMMON_NS;
    assert!(changed
        .descendants()
        .any(|node| node.tag_name().namespace() == Some(namespace)
            && node.tag_name().name() == "dimension"
            && node.text() == Some("true")));
    // Соседнее поле `ТипОбъекта` не тронуто.
    let neighbour = field_by_path("ТипОбъекта");
    assert!(!neighbour
        .descendants()
        .any(|node| node.text() == Some("Тип (представление)")
            || node.tag_name().name() == "dimension"));
}
#[test]
fn typed_dcs_role_preserves_every_supported_key_and_replaces_previous_entries() {
    for (role, expected) in [
        ("periodNumber=2", vec![("periodNumber", "2")]),
        ("periodType=Main", vec![("periodType", "Main")]),
        ("dimension=false", vec![("dimension", "false")]),
        (
            "parentDimension=Parent",
            vec![("parentDimension", "Parent")],
        ),
        ("account=true", vec![("account", "true")]),
        (
            "accountTypeExpression=Type",
            vec![("accountTypeExpression", "Type")],
        ),
        ("balance=true", vec![("balance", "true")]),
        (
            "balanceGroupName=Amount",
            vec![("balanceGroupName", "Amount")],
        ),
        (
            "balanceType=OpeningBalance",
            vec![("balanceType", "OpeningBalance")],
        ),
        (
            "accountingBalanceType=Debit",
            vec![("accountingBalanceType", "Debit")],
        ),
        ("accountField=Account", vec![("accountField", "Account")]),
        ("ignoreNullValues=true", vec![("ignoreNullValues", "true")]),
        ("required=true", vec![("required", "true")]),
        (
            "dimensionAttribute=true",
            vec![("dimensionAttribute", "true")],
        ),
        (
            "dimension=true parentDimension=Parent",
            vec![("dimension", "true"), ("parentDimension", "Parent")],
        ),
    ] {
        let fixture = Stand::new();
        let template_dir = fixture
            .source_dir()
            .join("Reports/Versions/Templates/Schema/Ext");
        std::fs::create_dir_all(&template_dir).unwrap();
        let mut original = SCHEMA.to_string();
        let document = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
        let field = document
            .descendants()
            .find(|node| {
                node.children().any(|child| {
                    child.tag_name().name() == "dataPath" && child.text() == Some("РазмерДанных")
                })
            })
            .unwrap();
        let insert = field
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "appearance")
            .unwrap()
            .range()
            .start
            + usize::from(original.starts_with('\u{feff}')) * 3;
        original.insert_str(
            insert,
            "<role><dcscom:required>false</dcscom:required></role>",
        );
        let role_args = expected
            .iter()
            .map(|(key, value)| {
                (
                    (*key).to_string(),
                    if matches!(*value, "true" | "false") {
                        json!(*value == "true")
                    } else {
                        json!(value)
                    },
                )
            })
            .collect::<serde_json::Map<_, _>>();
        std::fs::write(template_dir.join("Template.xml"), original.as_bytes()).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let operation=plan("fieldRole.set",&json!({"at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.РазмерДанных","values":{"role":role_args}}),0).unwrap();
        let (staged, effects) = stage_all(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation)],
        )
        .unwrap();
        assert_eq!(effects.len(), 1, "{role}");
        let changes = staged.planned_changes();
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current
        else {
            panic!("XML bytes")
        };
        let document = roxmltree::Document::parse(
            std::str::from_utf8(bytes)
                .unwrap()
                .trim_start_matches('\u{feff}'),
        )
        .unwrap();
        let field = document
            .descendants()
            .find(|node| {
                node.tag_name().name() == "field"
                    && node.children().any(|child| {
                        child.tag_name().name() == "dataPath"
                            && child.text() == Some("РазмерДанных")
                    })
            })
            .unwrap();
        let entries: Vec<_> = field
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().namespace()
                        == Some(unica_dcs_designer::schema::DCS_COMMON_NS)
            })
            .map(|node| (node.tag_name().name(), node.text().unwrap_or_default()))
            .collect();
        assert_eq!(entries, expected, "{role}");
    }
}
