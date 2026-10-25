//! Typed family dispatch and batch ownership; codec-owned plans have no publication authority.
use super::validate_platform_xml_binding;
use serde_json::Value;
#[cfg(test)]
use unica_address::address::QualifiedAddress;

use unica_metadata_designer::resource_planner::{self, MetadataResourcePlan};
use unica_operations::{OperationDescriptor, OperationFamily};
use unica_source_contract::proposal::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, IndexedPlanOperation,
    ProvisionalApplyEffect,
};
use unica_workspace::actor::ProviderRootBinding;

#[derive(Debug, Clone)]
enum FormResourcePlanKind {
    Form(unica_forms_designer::planner::FormPlan),
    Role(unica_roles_designer::planner::RolePlan),
    Metadata(MetadataResourcePlan),
    Unsupported,
}

#[derive(Debug, Clone)]
pub(crate) struct FormResourcePlanOperation {
    operation: String,
    kind: FormResourcePlanKind,
}

impl FormResourcePlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
}

/// Разбор без семейства, выбранного вызывающим: общее имя (`attribute.add`
/// формы и объекта, #1465) выбирает запись реестра по адресу вызова.
#[cfg(test)]
pub(crate) fn parse_form_resource_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_form_resource_plan_operation_in(descriptor, operation, args, op_index, binding)
}

/// Разбор операции семейством, которое выбрал разбор `apply` по записи
/// реестра для адреса вызова.
pub(crate) fn parse_form_resource_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<FormResourcePlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match descriptor.map(OperationDescriptor::family) {
        Some(OperationFamily::Role) => {
            match descriptor
                .and_then(OperationDescriptor::operation::<unica_roles::operations::RoleOperation>)
            {
                Some(operation) => {
                    FormResourcePlanKind::Role(unica_roles_designer::planner::parse_role_operation(
                        operation,
                        object,
                        op_index,
                        binding.source_set_name(),
                    )?)
                }
                None => return Err(hidden_apply_family_unimplemented(op_index)),
            }
        }
        Some(OperationFamily::Subsystem) => {
            FormResourcePlanKind::Metadata(resource_planner::parse_subsystem_operation(
                operation,
                object,
                op_index,
                binding.source_set_name(),
            )?)
        }
        Some(OperationFamily::Interface) => {
            FormResourcePlanKind::Metadata(resource_planner::parse_subsystem_operation(
                operation,
                object,
                op_index,
                binding.source_set_name(),
            )?)
        }
        Some(OperationFamily::Support) => {
            match descriptor.and_then(
                OperationDescriptor::operation::<unica_metadata::support::SupportOperation>,
            ) {
                Some(operation) => {
                    FormResourcePlanKind::Metadata(resource_planner::parse_support_operation(
                        operation,
                        object,
                        op_index,
                        binding.source_set_name(),
                    )?)
                }
                None => return Err(hidden_apply_family_unimplemented(op_index)),
            }
        }
        Some(OperationFamily::Form) => {
            match descriptor
                .and_then(OperationDescriptor::operation::<unica_forms::operations::FormOperation>)
            {
                Some(operation) => {
                    FormResourcePlanKind::Form(unica_forms_designer::planner::parse_form_operation(
                        operation,
                        object,
                        op_index,
                        binding.source_set_name(),
                    )?)
                }
                None => return Err(hidden_apply_family_unimplemented(op_index)),
            }
        }
        _ => FormResourcePlanKind::Unsupported,
    };
    Ok(FormResourcePlanOperation {
        operation: operation.to_string(),
        kind,
    })
}

pub(crate) fn plan_form_resource_batch(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operations: &[IndexedPlanOperation<FormResourcePlanOperation>],
) -> Result<Vec<ProvisionalApplyEffect>, ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "form/resource planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        match &operation.operation().kind {
            FormResourcePlanKind::Form(plan) => {
                let reader = unica_workspace::discovery::ProjectDiscoveryReader::new(
                    &authority.workspace_context().cwd,
                );
                unica_forms_designer::planner::stage_form_plan(
                    staged,
                    authority,
                    plan,
                    op_index,
                    &mut provisional,
                    &reader,
                )?;
            }
            FormResourcePlanKind::Unsupported => {
                return Err(hidden_apply_family_unimplemented(op_index));
            }

            FormResourcePlanKind::Role(plan) => {
                unica_roles_designer::planner::stage_role_plan(
                    staged,
                    authority,
                    plan,
                    op_index,
                    &mut provisional,
                )?;
            }
            FormResourcePlanKind::Metadata(plan) => {
                resource_planner::stage_metadata_resource_plan(
                    staged,
                    authority,
                    plan,
                    op_index,
                    &mut provisional,
                )?;
            }
        }
    }
    Ok(provisional)
}

#[cfg(test)]
pub(crate) fn plan_form_resource_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::FormResourceApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<FormResourcePlanOperation>],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        Vec<ProvisionalApplyEffect>,
    ),
    ApplyPlanError,
> {
    let effects = plan_form_resource_batch(&mut staged.proposal(), &authority, operations)?;
    Ok((staged, effects))
}

#[cfg(test)]
mod tests {
    use super::{parse_form_resource_plan_operation, plan_form_resource_batch_owned_for_test};

    #[test]
    fn a_dynamic_list_table_keeps_its_discriminator() {
        let item = json!({
            "name": "Список",
            "type": "Table",
            "path": "Список",
            "_dynList": true,
            "autoRefresh": true,
        });
        let (definition, _) = unica_forms_designer::planner::legacy_element_definition(
            item.as_object().unwrap(),
            0,
            "ops[0].args.items[0]",
        )
        .expect("a dynamic-list table is a valid element definition");
        assert_eq!(definition["_dynList"], true);
        assert_eq!(definition["autoRefresh"], true);
    }
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;
    use std::path::Path;
    use unica_source_contract::proposal::ApplyPlanErrorKind;
    use unica_source_contract::proposal::IndexedPlanOperation;

    const RIGHTS: &str = concat!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Rights\" version=\"2.20\">\n",
        "\t<setForNewObjects>false</setForNewObjects>\n",
        "\t<setForAttributesByDefault>true</setForAttributesByDefault>\n",
        "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
        "\t<object>\n\t\t<name>Document.First</name>\n\t\t<right>\n\t\t\t<name>Read</name>\n\t\t\t<value>true</value>\n\t\t</right>\n\t</object>\n",
        "</Rights>\n",
    );

    fn staged_text(staged: &unica_workspace::apply::ApplyStagedState, relative: &str) -> String {
        let change = staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == Path::new(relative))
            .unwrap_or_else(|| panic!("`{relative}` is staged"));
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = change.current else {
            panic!("`{relative}` keeps bytes");
        };
        String::from_utf8(bytes).unwrap()
    }

    /// Р2 (#1447): `item.add` добавляет один элемент. Вложенное дерево,
    /// companion-содержимое и события отказывают `bad_value` по точному
    /// пути до записи, а `next` ведёт к контракту `item.add`.
    #[test]
    fn item_add_refuses_nested_content_and_events_with_the_next_call() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Document.First.Form.ФормаДокумента";
        for key in unica_forms::contracts::FORM_ELEMENT_NESTED_KEYS
            .iter()
            .chain(unica_forms::contracts::FORM_ELEMENT_EVENT_KEYS)
            .chain(&["html", "textDoc"])
        {
            let mut item = json!({"name": "Parent", "type": "Group"});
            item[*key] = json!([{"name": "Child", "type": "HTMLDocumentField"}]);
            let error = parse_form_resource_plan_operation(
                "item.add",
                &json!({"at": at, "items": [item]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{key}");
            let path = format!("ops[0].args.items[0].{key}");
            assert_eq!(error.path(), Some(path.as_str()), "{key}");
            assert_eq!(
                error.next().unwrap()["args"]["at"],
                json!(format!("{at}.Can.item.add")),
                "{key}"
            );
            let follow_up = match *key {
                "html" => "HTMLDocumentField".to_string(),
                "textDoc" => "TextDocumentField".to_string(),
                _ => format!("{at}.Item.Parent"),
            };
            assert!(error.to_string().contains(&follow_up), "{key}: {error:?}");
        }
    }

    /// Перекрёстные правила, которых схема не выражает: ключи динамического
    /// списка без `_dynList` и две команды у кнопки пропали бы молча.
    #[test]
    fn item_add_refuses_keys_the_writer_would_drop_by_a_neighbour() {
        let fixture = ApplySeamFixture::new();
        for (item, path) in [
            (
                json!({"name": "T", "type": "Table", "autoRefresh": true}),
                "ops[0].args.items[0].autoRefresh",
            ),
            (
                json!({"name": "B", "type": "Button", "command": "A", "stdCommand": "Close"}),
                "ops[0].args.items[0].stdCommand",
            ),
        ] {
            let error = parse_form_resource_plan_operation(
                "item.add",
                &json!({"at": "main:Document.First.Form.ФормаДокумента", "items": [item]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some(path));
        }
    }

    /// Значения-образцы ключа по его схеме: перебираются, пока одно не
    /// изменит записанный XML.
    fn sample_values(schema: &serde_json::Value) -> Vec<serde_json::Value> {
        if let Some(values) = schema["enum"].as_array() {
            return values.clone();
        }
        if let Some(choices) = schema["anyOf"].as_array() {
            return choices.iter().flat_map(sample_values).collect();
        }
        match schema["type"].as_str() {
            Some("boolean") => vec![json!(true), json!(false)],
            Some("integer") | Some("number") => vec![json!(7)],
            Some("string") => vec![json!("Sample")],
            Some("object") => {
                let mut sample = serde_json::Map::new();
                match schema["properties"].as_object() {
                    Some(properties) => {
                        for (key, child) in properties {
                            sample.insert(key.clone(), sample_values(child)[0].clone());
                        }
                    }
                    None => {
                        sample.insert("en".into(), json!("Sample"));
                    }
                }
                vec![serde_json::Value::Object(sample)]
            }
            other => panic!("no sample for schema type {other:?}"),
        }
    }

    /// Контракт `item.add` и писатель согласны по каждому виду: каждый
    /// опубликованный ключ принимают контракт и планировщик, и хотя бы одно
    /// его значение меняет записанный XML вида (#1447). Ключ, который
    /// писатель вида не пишет, в контракт не попадает.
    #[test]
    fn every_published_element_key_reaches_the_xml_of_its_kind() {
        use unica_forms_designer::codec::{emit_form_element, FormIdAllocator};
        let fixture = ApplySeamFixture::new();
        let contract = unica_forms::operations::FormOperation::ItemAdd.contract();
        let variants = contract.schema["properties"]["items"]["items"]["anyOf"]
            .as_array()
            .unwrap()
            .clone();
        let emit = |item: &serde_json::Value| {
            let (definition, _) = unica_forms_designer::planner::legacy_element_definition(
                item.as_object().unwrap(),
                0,
                "items[0]",
            )
            .unwrap();
            let mut lines = Vec::new();
            emit_form_element(
                &mut lines,
                &definition,
                "\t",
                &mut FormIdAllocator::new(),
                Some("en"),
            )
            .unwrap_or_else(|error| panic!("{item}: {error}"));
            lines.join("\n")
        };
        for kind in unica_forms::contracts::FORM_ELEMENT_KINDS {
            let variant = variants
                .iter()
                .find(|variant| variant["properties"]["type"]["enum"][0] == json!(kind.kind))
                .unwrap_or_else(|| panic!("{} has a contract variant", kind.kind));
            let published = variant["properties"].as_object().unwrap();
            assert_eq!(
                published.len(),
                kind.keys.len() + 3,
                "{}: name, type, after and the kind keys",
                kind.kind
            );
            for key in kind.keys {
                let mut baseline = json!({"name": "Sample", "type": kind.kind});
                if kind.kind == "TextDocumentField" {
                    baseline["path"] = json!("Protocol");
                }
                if unica_forms::contracts::FORM_DYNAMIC_LIST_KEYS.contains(key) {
                    baseline["_dynList"] = json!(true);
                }
                let before = emit(&baseline);
                let written = sample_values(&published[*key]).into_iter().any(|value| {
                    let mut item = baseline.clone();
                    item[*key] = value;
                    parse_form_resource_plan_operation(
                        "item.add",
                        &json!({"at": "main:Document.First.Form.ФормаДокумента", "items": [item.clone()]}),
                        0,
                        &fixture.binding,
                    )
                    .unwrap_or_else(|error| panic!("{} {key}: {error:?}", kind.kind));
                    emit(&item) != before
                });
                assert!(written, "{}: `{key}` never reaches the XML", kind.kind);
            }
        }
    }

    /// Перечисления контракта элемента совпадают со справочником писателя.
    #[test]
    fn element_enums_match_the_writer_enum_contract() {
        use unica_forms::contracts::form_element_enum;
        use unica_forms_designer::codec::form_8_3_27_enum_values_for_test as writer;
        for (kind, key, xml) in [
            ("InputField", "titleLocation", "FormElementTitleLocation"),
            ("CheckBox", "checkBoxType", "CheckBoxType"),
            ("InputField", "horizontalAlign", "HorizontalAlign"),
            ("Group", "orientation", "FormChildrenGroup"),
            ("Group", "behavior", "UsualGroupBehavior"),
            ("Group", "representation", "UsualGroupRepresentation"),
            ("Table", "representation", "TableRepresentation"),
            ("Button", "representation", "ButtonRepresentation"),
            ("Group", "currentRowUse", "CurrentRowUse"),
            ("Pages", "pagesRepresentation", "PagesRepresentation"),
            ("Table", "commandBarLocation", "CommandBarLocation"),
            ("Table", "initialTreeView", "InitialTreeView"),
            ("Table", "choiceFoldersAndItems", "ChoiceFoldersAndItems"),
            ("Table", "updateOnDataChange", "UpdateOnDataChange"),
            (
                "Button",
                "locationInCommandBar",
                "ButtonLocationInCommandBar",
            ),
        ] {
            let mut published = form_element_enum(kind, key).unwrap().to_vec();
            let mut expected = writer(xml).unwrap().to_vec();
            published.sort_unstable();
            expected.sort_unstable();
            assert_eq!(published, expected, "{kind} {key}");
        }
    }

    /// Реквизит формы принимает только то, что пишет его писатель (#1447);
    /// проверка заполнения пишется тегом `<FillCheck>`, который знает 8.3.27.
    #[test]
    fn form_attribute_add_refuses_unwritten_keys_and_writes_fill_check() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Document.First.Form.ФормаДокумента";
        for (key, value) in [
            ("view", json!(false)),
            ("edit", json!(false)),
            ("functionalOptions", json!(["FunctionalOption.X"])),
            ("useAlways", json!(["Field"])),
            ("save", json!(true)),
            ("fillCheck", json!(true)),
            ("valueType", json!("String")),
            ("settings", json!({})),
            ("chart", json!({})),
            ("planner", json!({})),
            ("additionalColumns", json!([])),
            ("title", json!({"ru": "Заголовок"})),
            ("fillChecking", json!("DontCheck")),
        ] {
            let error = parse_form_resource_plan_operation(
                "attribute.add",
                &json!({"at": at, "items": [{"name": "A", "type": "String", key: value}]}),
                0,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{key}");
            let path = format!("ops[0].args.items[0].{key}");
            assert_eq!(error.path(), Some(path.as_str()), "{key}");
        }
        let mut lines = Vec::new();
        unica_forms_designer::codec::emit_form_edit_attribute_item(
            &mut lines,
            json!({"name": "A", "type": "xs:string", "savedData": true, "fillChecking": "ShowError"})
                .as_object()
                .unwrap(),
            "A",
            1,
            "\t\t",
            Some("ru"),
        )
        .unwrap();
        let text = lines.join("\n");
        assert!(!text.contains("FillChecking"), "{text}");
        let saved = text.find("<SavedData>true</SavedData>").unwrap();
        let fill = text.find("<FillCheck>ShowError</FillCheck>").unwrap();
        assert!(saved < fill, "{text}");
    }

    #[test]
    fn role_create_and_form_add_write_the_configuration_default_language() {
        let fixture = ApplySeamFixture::new();
        fixture.use_english_only_language();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let operations = [
            parse(
                "role.create",
                json!({"at": "main:Configuration", "values": {"name": "Manager"}}),
                0,
            ),
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                1,
            ),
        ];
        let (staged, _) = plan_form_resource_batch_owned_for_test(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        for relative in ["Roles/Manager.xml", "Documents/First/Forms/Main.xml"] {
            let text = staged_text(&staged, relative);
            assert!(text.contains("<v8:lang>en</v8:lang>"), "{relative}\n{text}");
            assert!(
                !text.contains("<v8:lang>ru</v8:lang>"),
                "{relative}\n{text}"
            );
        }
    }

    /// #1257: titles, tooltips and input hints of new form elements,
    /// attributes and commands are written in the default language of the
    /// source set; an explicit language-to-text value is written as given.
    #[test]
    fn form_item_titles_and_tooltips_write_the_configuration_default_language() {
        let fixture = ApplySeamFixture::new();
        fixture.use_english_only_language();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let form = "main:Document.First.Form.Main";
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "item.add",
                json!({"at": form, "items": [
                    {"name": "Header", "type": "Group", "title": "Header"},
                    {"name": "Total", "path": "Объект.Total", "title": "Total",
                     "tooltip": "Total hint", "inputHint": "Type the total"},
                    {"name": "Count", "path": "Объект.Total",
                     "tooltip": {"en": "Count hint", "de": "Anzahl"}},
                    {"name": "Note", "type": "Label", "title": "Note"}
                ]}),
                1,
            ),
            parse(
                "attribute.add",
                json!({"at": form, "items": [{"name": "Comment", "type": "String", "title": "Comment"}]}),
                2,
            ),
            parse(
                "command.add",
                json!({"at": form, "items": [{"name": "Refresh", "title": "Refresh"}]}),
                3,
            ),
        ];
        let (staged, _) = plan_form_resource_batch_owned_for_test(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(!text.contains("<v8:lang>ru</v8:lang>"), "{text}");
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let translations = document
            .descendants()
            .filter(|node| node.tag_name().name() == "item")
            .filter_map(|item| {
                let part = |name: &str| {
                    item.children()
                        .find(|child| child.tag_name().name() == name)
                        .and_then(|child| child.text())
                };
                Some((part("lang")?.to_string(), part("content")?.to_string()))
            })
            .collect::<Vec<_>>();
        for content in [
            "Header",
            "Total",
            "Total hint",
            "Type the total",
            "Count hint",
            "Note",
            "Comment",
            "Refresh",
        ] {
            assert!(
                translations.contains(&("en".to_string(), content.to_string())),
                "`{content}` must be written in en: {translations:?}\n{text}"
            );
        }
        assert!(
            translations.contains(&("de".to_string(), "Anzahl".to_string())),
            "an explicit translation is written as given\n{text}"
        );
    }

    /// Only a plain-string text needs the default language: in a source set
    /// whose language cannot be read, an element without text, an explicit
    /// translation and a removal pass, a plain-string title is refused.
    #[test]
    fn form_edits_without_plain_text_do_not_read_the_default_language() {
        let fixture = ApplySeamFixture::new();
        let form = "main:Document.First.Form.Main";
        let plan = |operations: &[(&str, serde_json::Value)]| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operations = operations
                .iter()
                .enumerate()
                .map(|(index, (op, args))| {
                    IndexedPlanOperation::new(
                        index,
                        parse_form_resource_plan_operation(op, args, index, &fixture.binding)
                            .unwrap_or_else(|error| panic!("{op}: {error:?}")),
                    )
                })
                .collect::<Vec<_>>();
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .map(|(staged, _)| staged)
        };
        let staged = plan(&[
            (
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
            ),
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Header", "type": "Group", "title": "Header"}]}),
            ),
        ])
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        for relative in [
            "Documents/First.xml",
            "Documents/First/Forms/Main.xml",
            "Documents/First/Forms/Main/Ext/Form.xml",
            "Documents/First/Forms/Main/Ext/Form/Module.bsl",
        ] {
            let path = fixture.source_dir().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, staged_text(&staged, relative)).unwrap();
        }
        std::fs::remove_file(fixture.source_dir().join("Languages/Русский.xml")).unwrap();

        let error = plan(&[(
            "item.add",
            json!({"at": form, "items": [{"name": "Footer", "type": "Group", "title": "Footer"}]}),
        )])
        .expect_err("a new title needs the default language");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{error:?}");

        let staged = plan(&[
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Plain", "path": "Объект.Total"}]}),
            ),
            (
                "item.add",
                json!({"at": form, "items": [{"name": "Hinted", "path": "Объект.Total",
                    "tooltip": {"en": "Hint"}}]}),
            ),
        ])
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(text.contains("name=\"Plain\""), "{text}");
        assert!(text.contains("<v8:lang>en</v8:lang>"), "{text}");
        assert!(!text.contains("<v8:lang></v8:lang>"), "{text}");

        let staged = plan(&[("item.remove", json!({"at": format!("{form}.Item.Header")}))])
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(!text.contains("name=\"Header\""), "{text}");
    }

    #[test]
    fn form_add_then_element_and_event_edits_compose_in_one_batch() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "ФормаДокумента", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "item.add",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента",
                    "items": [{"name": "ГруппаИтог", "type": "UsualGroup"}]
                }),
                1,
            ),
            // Группа, добавленная предыдущей операцией, уже адресуется.
            parse(
                "item.add",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Item.ГруппаИтог",
                    "items": [{"name": "ПолеИтог", "path": "Объект.Total", "title": "Итог"}]
                }),
                2,
            ),
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Event.OnOpen",
                    "values": {"handler": "ПриОткрытии"}
                }),
                3,
            ),
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.ФормаДокумента.Item.ГруппаИтог.Item.ПолеИтог.Event.OnChange",
                    "values": {"handler": "ПолеИтогПриИзменении"}
                }),
                4,
            ),
        ];
        let (staged, effects) =
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 5);
        let form = staged_text(&staged, "Documents/First/Forms/ФормаДокумента/Ext/Form.xml");
        let elements = unica_forms_designer::planner::form_items_of(&form).unwrap();
        let field = unica_forms_designer::planner::form_item_at(
            &elements,
            &["ГруппаИтог".to_string(), "ПолеИтог".to_string()],
        )
        .expect("the field lands inside the group named by the address");
        assert_eq!(field.events.len(), 1, "{form}");
        assert_eq!(field.events[0].name, "OnChange");
        assert_eq!(field.events[0].handler, "ПолеИтогПриИзменении");
        assert!(form.contains("<DataPath>Объект.Total</DataPath>"), "{form}");
        assert!(
            form.contains("<Event name=\"OnOpen\">ПриОткрытии</Event>"),
            "{form}"
        );
        let owner = staged_text(&staged, "Documents/First.xml");
        assert!(owner.contains("<Form>ФормаДокумента</Form>"), "{owner}");
        assert!(
            staged
                .planned_changes()
                .iter()
                .any(|change| change.relative_path
                    == Path::new("Documents/First/Forms/ФормаДокумента/Ext/Form/Module.bsl")),
            "the form module is staged"
        );
    }

    #[test]
    fn item_add_writes_a_bound_text_document_field_and_refuses_invalid_bindings() {
        let fixture = ApplySeamFixture::new();
        let form = "main:Document.First.Form.Main";
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                .map(|operation| IndexedPlanOperation::new(index, operation))
        };
        for item in [
            json!({"name":"ProtocolField","type":"TextDocumentField"}),
            json!({"name":"ProtocolField","type":"TextDocumentField","path":""}),
        ] {
            let error = parse("item.add", json!({"at": form, "items": [item]}), 0).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some("ops[0].args.items[0].path"));
        }

        let plan = |attribute_type: Option<&str>| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let mut operations = vec![parse(
                "form.add",
                json!({"at":"main:Document.First","items":[{"name":"Main","type":"ObjectForm"}]}),
                0,
            )
            .unwrap()];
            if let Some(attribute_type) = attribute_type {
                operations.push(
                    parse(
                        "attribute.add",
                        json!({"at":form,"items":[{"name":"Protocol","type":attribute_type}]}),
                        1,
                    )
                    .unwrap(),
                );
            }
            let item_index = operations.len();
            operations.push(
                parse(
                    "item.add",
                    json!({"at":form,"items":[
                        {
                            "name":"Протокол1",
                            "type":"TextDocumentField",
                            "path":"Protocol",
                            "titleLocation":"None"
                        },
                        {
                            "name":"Протокол2",
                            "type":"TextDocumentField",
                            "path":"Protocol",
                            "visible":false,
                            "enabled":false,
                            "readOnly":true,
                            "title":"Протокол",
                            "titleLocation":"Top",
                            "tooltip":"Подсказка",
                            "tooltipRepresentation":"Button",
                            "width":35,
                            "autoMaxWidth":false,
                            "height":10,
                            "autoMaxHeight":false
                        }
                    ]}),
                    item_index,
                )
                .unwrap(),
            );
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .map(|(staged, _)| staged)
        };

        let before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        for attribute_type in [None, Some("String")] {
            let error = plan(attribute_type).unwrap_err();
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{error:?}");
            assert!(
                error
                    .to_string()
                    .contains("requires a TextDocument attribute"),
                "{error:?}"
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
                before,
                "planning an invalid field must not write the source tree"
            );
        }

        let staged = plan(Some("d5p1:TextDocument"))
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let platform_export = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/text-document-field/Form.xml"
        ));
        let platform = roxmltree::Document::parse(platform_export).unwrap();
        for name in ["Протокол1", "Протокол2"] {
            let child_tags = |root: roxmltree::Node<'_, '_>| {
                root.descendants()
                    .find(|node| {
                        node.has_tag_name("TextDocumentField")
                            && node.attribute("name") == Some(name)
                    })
                    .unwrap()
                    .children()
                    .filter(|node| node.is_element())
                    .map(|node| node.tag_name().name().to_owned())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                child_tags(document.root()),
                child_tags(platform.root()),
                "{name}"
            );
        }
        assert_eq!(document.root_element().attribute("version"), Some("2.20"));
        let field = document
            .descendants()
            .find(|node| {
                node.has_tag_name("TextDocumentField")
                    && node.attribute("name") == Some("Протокол2")
            })
            .unwrap();
        assert!(field.children().all(|node| !node.has_tag_name("Height")));
        assert_eq!(
            field
                .children()
                .find(|node| node.has_tag_name("DataPath"))
                .and_then(|node| node.text()),
            Some("Protocol")
        );
        let type_node = document
            .descendants()
            .find(|node| node.text() == Some("d5p1:TextDocument"))
            .unwrap();
        assert_eq!(
            type_node.lookup_namespace_uri(Some("d5p1")),
            Some("http://v8.1c.ru/8.1/data/txtedt")
        );
        assert!(!text.contains("<HTMLDocumentField name=\"Протокол2\""));
        assert!(!text.contains("<InputField name=\"Протокол2\""));
    }

    /// Узел правки формы — ровно адрес (#1355): цепочка `Item` сверяется
    /// с формой, отсутствующий элемент даёт `not_found`, в поле не
    /// добавить, сосед `after` берётся только из адресованного контейнера,
    /// а удаление и событие касаются только названного элемента.
    #[test]
    fn a_form_edit_writes_only_the_addressed_item() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let form = "main:Document.First.Form.Main";
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                .map(|operation| IndexedPlanOperation::new(index, operation))
        };
        let base = || {
            vec![
                parse(
                    "form.add",
                    json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                    0,
                )
                .unwrap(),
                parse(
                    "item.add",
                    json!({"at": form, "items": [
                        {"name": "Header", "type": "Group"},
                        {"name": "Footer", "type": "Group"}
                    ]}),
                    1,
                )
                .unwrap(),
                parse(
                    "item.add",
                    json!({"at": format!("{form}.Item.Header"), "items": [
                        {"name": "Total", "path": "Объект.Total"},
                        {"name": "Count", "path": "Объект.Total"}
                    ]}),
                    2,
                )
                .unwrap(),
            ]
        };
        let plan = |op: &str, args: serde_json::Value| {
            let mut operations = base();
            operations.push(parse(op, args, 3)?);
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            plan_form_resource_batch_owned_for_test(
                admission.staged_state().unwrap(),
                authority,
                &operations,
            )
            .map(|(staged, _)| {
                let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
                unica_forms_designer::planner::form_items_of(&text).unwrap()
            })
        };
        let names = |elements: &[unica_forms::facts::FormInfoElement]| {
            elements
                .iter()
                .map(|element| element.name.clone())
                .collect::<Vec<_>>()
        };
        let chain = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        for (op, args, kind, path) in [
            (
                "item.add",
                json!({"at": format!("{form}.Item.Footer.Item.Total"), "items": [{"name": "X", "path": "Объект.Total"}]}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
            (
                "item.add",
                json!({"at": format!("{form}.Item.Header.Item.Total"), "items": [{"name": "X", "path": "Объект.Total"}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[3].args.at",
            ),
            (
                "item.add",
                json!({"at": format!("{form}.Item.Footer"), "items": [{"name": "X", "path": "Объект.Total", "after": "Total"}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[3].args.items[0].after",
            ),
            (
                "item.remove",
                json!({"at": format!("{form}.Item.Total")}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
            (
                "event.bind",
                json!({"at": format!("{form}.Item.Footer.Item.Total.Event.OnChange"), "values": {"handler": "H"}}),
                ApplyPlanErrorKind::NotFound,
                "ops[3].args.at",
            ),
        ] {
            let Err(error) = plan(op, args.clone()) else {
                panic!("{op} {args} is refused");
            };
            assert_eq!(error.kind(), kind, "{op} {args}: {error}");
            assert_eq!(error.path(), Some(path), "{op} {args}: {error}");
        }
        // Сосед в том же контейнере задаёт место.
        let elements = plan(
            "item.add",
            json!({"at": format!("{form}.Item.Header"), "items": [{"name": "X", "path": "Объект.Total", "after": "Total"}]}),
        )
        .unwrap();
        let header =
            unica_forms_designer::planner::form_item_at(&elements, &chain(&["Header"])).unwrap();
        assert_eq!(names(&header.children), ["Total", "X", "Count"]);
        // Удаление вложенного элемента оставляет контейнер и соседа.
        let elements = plan(
            "item.remove",
            json!({"at": format!("{form}.Item.Header.Item.Total")}),
        )
        .unwrap();
        // Автоматическая командная панель новой формы — тоже элемент `Item`.
        assert_eq!(
            names(&elements),
            ["ФормаКоманднаяПанель", "Header", "Footer"]
        );
        let header =
            unica_forms_designer::planner::form_item_at(&elements, &chain(&["Header"])).unwrap();
        assert_eq!(names(&header.children), ["Count"]);
        // Событие ложится только на названный элемент.
        let elements = plan(
            "event.bind",
            json!({"at": format!("{form}.Item.Header.Item.Count.Event.OnChange"), "values": {"handler": "CountOnChange"}}),
        )
        .unwrap();
        let count =
            unica_forms_designer::planner::form_item_at(&elements, &chain(&["Header", "Count"]))
                .unwrap();
        assert_eq!(count.events.len(), 1);
        assert_eq!(count.events[0].handler, "CountOnChange");
        let total =
            unica_forms_designer::planner::form_item_at(&elements, &chain(&["Header", "Total"]))
                .unwrap();
        assert!(total.events.is_empty());
        // Снятые аргументы разбор семейства отвергает и в обход реестра.
        for (op, args, path) in [
            (
                "item.add",
                json!({"at": form, "items": [{"name": "X", "into": "Header"}]}),
                "ops[3].args.items[0].into",
            ),
            (
                "item.remove",
                json!({"at": format!("{form}.Item.Header"), "items": [{"name": "Total"}]}),
                "ops[3].args.items",
            ),
            (
                "event.bind",
                json!({"at": format!("{form}.Event.OnOpen"), "values": {"handler": "H", "event": "OnClose"}}),
                "ops[3].args.values.event",
            ),
            (
                "event.bind",
                json!({"at": form, "values": {"handler": "H"}}),
                "ops[3].args.at",
            ),
        ] {
            let Err(error) = parse(op, args.clone(), 3) else {
                panic!("{op} {args} is refused");
            };
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {args}");
            assert_eq!(error.path(), Some(path), "{op} {args}: {error}");
        }
    }

    /// Сосед `after` может быть добавлен раньше в том же `items` (#1443).
    #[test]
    fn item_add_after_names_an_item_added_earlier_in_the_same_list() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let form = "main:Document.First.Form.Main";
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [{"name": "Main", "type": "ObjectForm"}]}),
                0,
            ),
            parse(
                "item.add",
                json!({"at": form, "items": [{"name": "Header", "type": "Group"}]}),
                1,
            ),
            parse(
                "item.add",
                json!({"at": format!("{form}.Item.Header"), "items": [
                    {"name": "A", "path": "Объект.Total"},
                    {"name": "B", "path": "Объект.Total", "after": "A"}
                ]}),
                2,
            ),
        ];
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, _) = plan_form_resource_batch_owned_for_test(
            admission.staged_state().unwrap(),
            authority,
            &operations,
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let text = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        let elements = unica_forms_designer::planner::form_items_of(&text).unwrap();
        let header =
            unica_forms_designer::planner::form_item_at(&elements, &["Header".to_string()])
                .unwrap();
        let names = header
            .children
            .iter()
            .map(|element| element.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["A", "B"], "{text}");
    }

    /// События из каталога 8.3.27, зависящие от вида владельца формы и пути
    /// данных таблицы, принимаются и при добавлении элемента, и итоговой
    /// проверкой формы (#1443).
    #[test]
    fn owner_and_data_path_specific_events_pass_planning_and_final_validation() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{op}: {error:?}")),
            )
        };
        let operations = [
            parse(
                "form.add",
                json!({"at": "main:Document.First", "items": [
                    {"name": "Main", "type": "ObjectForm"},
                    {"name": "List", "type": "ListForm"}
                ]}),
                0,
            ),
            // `ValueChoice` формы есть только в каталоге владельца-документа.
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.Main.Event.ValueChoice",
                    "values": {"handler": "ОбработкаВыбораЗначения"}
                }),
                1,
            ),
            // `URLListGetProcessing` таблицы есть только у источника
            // динамического списка, его выбирает путь данных таблицы.
            parse(
                "item.add",
                json!({
                    "at": "main:Document.First.Form.List",
                    "items": [{
                        "name": "ВтороеПредставление",
                        "type": "Table",
                        "path": "Список"
                    }]
                }),
                2,
            ),
            // Событие нового элемента привязывает `event.bind` на его адресе
            // (Р2, #1447).
            parse(
                "event.bind",
                json!({
                    "at": "main:Document.First.Form.List.Item.ВтороеПредставление.Event.URLListGetProcessing",
                    "values": {"handler": "ВтороеПредставлениеОбработкаПолученияСпискаURL"}
                }),
                3,
            ),
        ];
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, _) = plan_form_resource_batch_owned_for_test(
            admission.staged_state().unwrap(),
            authority,
            &operations,
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let main = staged_text(&staged, "Documents/First/Forms/Main/Ext/Form.xml");
        assert!(
            main.contains("<Event name=\"ValueChoice\">ОбработкаВыбораЗначения</Event>"),
            "{main}"
        );
        let list = staged_text(&staged, "Documents/First/Forms/List/Ext/Form.xml");
        assert!(
            list.contains("<Event name=\"URLListGetProcessing\">"),
            "{list}"
        );
    }

    /// Главное обещание семейства: писатель правит только `Common`, а
    /// значения видимости по ролям — соседние элементы того же блока —
    /// остаются как были. Читатель их не показывает, поэтому наивный писатель,
    /// переписывающий блок по прочитанному, снёс бы их молча.
    #[test]
    fn command_visibility_edits_common_and_leaves_role_values_alone() {
        let fixture = ApplySeamFixture::new();
        let interface_dir = fixture.source_dir().join("Subsystems/Sales/Ext");
        std::fs::create_dir_all(&interface_dir).unwrap();
        std::fs::write(
            fixture.source_dir().join("Subsystems/Sales.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Subsystem uuid=\"44444444-4444-4444-8444-444444444444\"><Properties><Name>Sales</Name></Properties><ChildObjects/></Subsystem></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(
            interface_dir.join("CommandInterface.xml"),
            concat!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
                "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\">\n",
                "\t<CommandsVisibility>\n",
                "\t\t<Command name=\"Catalog.First.Command.Open\">\n",
                "\t\t\t<Visibility>\n",
                "\t\t\t\t<xr:Common>true</xr:Common>\n",
                "\t\t\t\t<xr:Value name=\"Role.Кладовщик\">false</xr:Value>\n",
                "\t\t\t</Visibility>\n",
                "\t\t</Command>\n",
                "\t</CommandsVisibility>\n",
                "\t<GroupsOrder>\n",
                "\t\t<Group>NavigationPanelImportant</Group>\n",
                "\t\t<Group>NavigationPanelSeeAlso</Group>\n",
                "\t</GroupsOrder>\n",
                "</CommandInterface>\n"
            ),
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        let operations = [
            parse(
                "commandVisibility.set",
                json!({
                    "at": "main:Subsystem.Sales.Interface",
                    "items": [{"command": "Catalog.First.Command.Open", "visible": false}]
                }),
                0,
            ),
            parse(
                "groupOrder.set",
                json!({
                    "at": "main:Subsystem.Sales.Interface",
                    "values": {"groups": ["NavigationPanelSeeAlso", "NavigationPanelImportant"]}
                }),
                1,
            ),
        ];

        let (staged, effects) =
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));

        assert_eq!(effects.len(), 2);
        let text = staged_text(&staged, "Subsystems/Sales/Ext/CommandInterface.xml");
        assert!(text.contains("<xr:Common>false</xr:Common>"), "{text}");
        // Роль на месте — и это единственная причина, по которой правка
        // общего значения вообще безопасна.
        assert!(
            text.contains("<xr:Value name=\"Role.Кладовщик\">false</xr:Value>"),
            "{text}"
        );
        let seen = text.find("NavigationPanelSeeAlso").expect("group order");
        let important = text
            .find("<Group>NavigationPanelImportant</Group>")
            .expect("group");
        assert!(seen < important, "группы переставлены: {text}");
    }

    #[test]
    fn right_set_grants_lists_and_restricts_rights_on_the_staged_document() {
        let fixture = ApplySeamFixture::new();
        let role_dir = fixture.source_dir().join("Roles/Reader/Ext");
        std::fs::create_dir_all(&role_dir).unwrap();
        std::fs::write(
            fixture.source_dir().join("Roles/Reader.xml"),
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Role uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(role_dir.join("Rights.xml"), RIGHTS).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, at: &str, args: serde_json::Value, index: usize| {
            let mut args = args;
            args["at"] = json!(at);
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        // Запись прав создаёт `right.add` на роли, а `right.set` меняет
        // существующую запись по её адресу (#1354).
        let operations = [
            parse(
                "right.set",
                "main:Role.Reader.Right.Document_First",
                json!({"values": {"right": "Update", "value": true}}),
                0,
            ),
            parse(
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Second", "rights": {"Read": true}}]}),
                1,
            ),
            parse(
                "right.set",
                "main:Role.Reader.Right.Document_Second",
                json!({"values": {"right": "Read", "rls": "ГДЕ Автор = &Пользователь"}}),
                2,
            ),
            // Вложенный объект — одна запись с экранированными точками
            // в имени: созданную запись правит `right.set` по её адресу.
            parse(
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "DataProcessor.D.Command.C", "rights": {"View": true}}]}),
                3,
            ),
            parse(
                "right.set",
                "main:Role.Reader.Right.DataProcessor_D\\.Command\\.C",
                json!({"values": {"right": "View", "value": false}}),
                4,
            ),
        ];
        let (staged, effects) =
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 5);
        let text = staged_text(&staged, "Roles/Reader/Ext/Rights.xml");
        assert!(text.starts_with('\u{feff}'), "byte-order mark survives");
        assert!(text.contains("<name>Update</name>"), "{text}");
        assert!(text.contains("<name>Document.Second</name>"), "{text}");
        let nested = text
            .split("<object>")
            .find(|entry| entry.contains("<name>DataProcessor.D.Command.C</name>"))
            .unwrap_or_else(|| panic!("nested entry is written: {text}"));
        assert!(
            nested.contains("<name>View</name>") && nested.contains("<value>false</value>"),
            "{nested}"
        );
        assert!(
            text.contains("<condition>ГДЕ Автор = &amp;Пользователь</condition>"),
            "{text}"
        );

        // Опечатка в адресе записи не создаёт объект роли, а повторное
        // `right.add` не перекрывает существующую запись.
        for (op, at, args, kind, path) in [
            (
                "right.set",
                "main:Role.Reader.Right.Document_Frist",
                json!({"values": {"right": "Read", "value": true}}),
                ApplyPlanErrorKind::NotFound,
                "ops[0].args.at",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.First", "rights": {"Read": false}}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Second", "rights": {"Read": false}}]}),
                ApplyPlanErrorKind::BadValue,
                "ops[0].args.items[0].rights",
            ),
        ] {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let Err(error) = plan_form_resource_batch_owned_for_test(
                staged,
                authority,
                &[parse(op, at, args, 0)],
            ) else {
                panic!("{op} at {at}: the existing entry decides");
            };
            assert_eq!(error.kind(), kind, "{op}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
        // Объект записи назван адресом: прежний аргумент и адрес роли
        // разбор отвергает.
        for (op, at, args, path) in [
            (
                "right.set",
                "main:Role.Reader.Right.Document_First",
                json!({"values": {"object": "Document.First", "right": "Read", "value": true}}),
                "ops[0].args.values.object",
            ),
            (
                "right.set",
                "main:Role.Reader",
                json!({"values": {"right": "Read", "value": true}}),
                "ops[0].args.at",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.Third", "rights": {}}]}),
                "ops[0].args.items[0].rights",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Third", "rights": {"Read": true}}]}),
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "Document.A:B", "rights": {"Read": true}}]}),
                "ops[0].args.items[0].object",
            ),
            (
                "right.add",
                "main:Role.Reader",
                json!({"items": [{"object": "DataProcessor.D..C", "rights": {"Use": true}}]}),
                "ops[0].args.items[0].object",
            ),
            // Короткий псевдоним чтения — не каноническое имя записи.
            (
                "right.set",
                "main:Role.Reader.Right.Товары_Архив",
                json!({"values": {"right": "Read", "value": true}}),
                "ops[0].args.at",
            ),
        ] {
            let mut args = args;
            args["at"] = json!(at);
            let error = parse_form_resource_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("the subject comes from the address");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {path}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
    }

    /// Вложенный объект `right.add` — из закрытого набора раскладок
    /// Rights.xml. Опечатка вида и вложенный объект корня конфигурации
    /// отклоняются до записи, право вложенного объекта — из его набора.
    #[test]
    fn right_add_accepts_only_nested_objects_a_role_lists() {
        let fixture = ApplySeamFixture::new();
        for object in [
            "Catalog.Items.Command.Open",
            "Catalog.Items.Attribute.Price",
            "Catalog.Items.StandardAttribute.Code",
            "Document.Order.TabularSection.Goods",
            "Document.Order.TabularSection.Goods.Attribute.Qty",
            "Document.Order.TabularSection.Goods.StandardAttribute.LineNumber",
            "InformationRegister.Prices.Dimension.Item",
            "AccumulationRegister.Stock.Resource.Qty",
            "Task.Job.AddressingAttribute.Assignee",
            "Subsystem.Sales.Subsystem.Orders.Subsystem.Returns",
        ] {
            let args = json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": {"View": true}}]});
            parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .unwrap_or_else(|error| panic!("{object}: {error:?}"));
        }
        for object in [
            "WebService.Exchange.Operation.Send",
            "HTTPService.Api.URLTemplate.Items.Method.Get",
            "IntegrationService.Bus.IntegrationServiceChannel.Out",
        ] {
            let args = json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": {"Use": true}}]});
            parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .unwrap_or_else(|error| panic!("{object}: {error:?}"));
        }
        for (object, rights, path) in [
            (
                "Catalog.Items.Comand.Open",
                json!({"Read": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Configuration.X.Command.Y",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Constant.Rate.Command.Open",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Catalog.Items.Command.Open.Attribute.X",
                json!({"View": true}),
                "ops[0].args.items[0].object",
            ),
            (
                "Catalog.Items.Command.Open",
                json!({"Read": true}),
                "ops[0].args.items[0].rights.Read",
            ),
        ] {
            let args =
                json!({"at": "main:Role.Reader", "items": [{"object": object, "rights": rights}]});
            let error = parse_form_resource_plan_operation("right.add", &args, 0, &fixture.binding)
                .expect_err(object);
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{object}");
            assert_eq!(error.path(), Some(path), "{object}");
        }
    }

    #[test]
    fn role_target_refusals_precede_argument_contract_refusals() {
        let fixture = ApplySeamFixture::new();
        for (operation, args) in [
            ("role.create", json!({"values":{"name":42},"unknown":true})),
            ("right.add", json!({"items":"wrong","unknown":true})),
            ("right.set", json!({"values":{"right":42},"unknown":true})),
        ] {
            let error = parse_form_resource_plan_operation(operation, &args, 0, &fixture.binding)
                .expect_err("missing target must be refused before malformed arguments");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some("ops[0].args.at"), "{operation}");
        }
    }

    #[test]
    fn role_create_contract_preserves_typed_names_and_rejects_unknown_args() {
        let fixture = ApplySeamFixture::new();
        let contract = unica_roles::operations::RoleOperation::Create.contract();
        for (input, expected) in [
            ("Reader", "Reader"),
            ("МенеджерПродаж", "МенеджерПродаж"),
            ("_Reader2", "_Reader2"),
            (" Reader ", "Reader"),
            ("\tЧитатель\r\n", "Читатель"),
            ("\u{a0}Ёж\u{a0}", "Ёж"),
        ] {
            let args = json!({"at":"main:Configuration","values":{"name":input}});
            assert!(
                contract.normalize(&args, "ops[0].args").is_ok(),
                "{input:?}"
            );
            let parsed =
                parse_form_resource_plan_operation("role.create", &args, 0, &fixture.binding)
                    .unwrap_or_else(|error| panic!("{input:?}: {error:?}"));
            let super::FormResourcePlanKind::Role(
                unica_roles_designer::planner::RolePlan::CreateRole { name },
            ) = parsed.kind
            else {
                panic!("role.create must produce a typed role creation");
            };
            assert_eq!(name, expected);
        }
        for (args, expected_path) in [
            (json!({"values":{}}), "ops[0].args.values.name"),
            (json!({"values":{"name":" "}}), "ops[0].args.values.name"),
            (
                json!({"values":{"name":"Role.Reader"}}),
                "ops[0].args.values.name",
            ),
            (json!({"values":{"name":1}}), "ops[0].args.values.name"),
            (json!({"values":{"name":null}}), "ops[0].args.values.name"),
            (json!({"values":null}), "ops[0].args.values"),
            (json!({"values":[]}), "ops[0].args.values"),
            (json!({"values":"Reader"}), "ops[0].args.values"),
            (
                json!({"values":{"name":"Reader","synonym":"Ignored"}}),
                "ops[0].args.values.synonym",
            ),
            // Previously ignored top-level fields are intentionally closed by #1468.
            (
                json!({"values":{"name":"Reader"},"ignored":true}),
                "ops[0].args.ignored",
            ),
        ] {
            let mut args = args;
            args["at"] = json!("main:Configuration");
            let schema_error = contract.normalize(&args, "ops[0].args").unwrap_err();
            assert_eq!(schema_error.path, expected_path, "{args}");
            let error =
                parse_form_resource_plan_operation("role.create", &args, 0, &fixture.binding)
                    .expect_err("closed typed creation arguments");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{args}");
            assert_eq!(error.path(), Some(expected_path), "{args}");
        }
    }

    #[test]
    fn role_create_stages_descriptor_rights_and_registration_deterministically() {
        let fixture = ApplySeamFixture::new();
        let plan = |fixture: &ApplySeamFixture| {
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operation = parse_form_resource_plan_operation(
                "role.create",
                &json!({"at": "main:Configuration", "values": {"name": "Manager"}}),
                0,
                &fixture.binding,
            )
            .unwrap();
            let (staged, _) = plan_form_resource_batch_owned_for_test(
                staged,
                authority,
                &[IndexedPlanOperation::new(0, operation)],
            )
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
            let mut paths = staged
                .planned_changes()
                .into_iter()
                .map(|change| change.relative_path)
                .collect::<Vec<_>>();
            paths.sort();
            (
                paths,
                staged_text(&staged, "Roles/Manager.xml"),
                staged_text(&staged, "Configuration.xml"),
            )
        };
        let (paths, descriptor, owner) = plan(&fixture);
        assert_eq!(
            paths,
            vec![
                std::path::PathBuf::from("Configuration.xml"),
                std::path::PathBuf::from("Roles/Manager/Ext/Rights.xml"),
                std::path::PathBuf::from("Roles/Manager.xml"),
            ]
        );
        assert!(owner.contains("<Role>Manager</Role>"), "{owner}");
        assert!(descriptor.contains("<Name>Manager</Name>"), "{descriptor}");
        let (_, descriptor_again, _) = plan(&fixture);
        assert_eq!(
            descriptor, descriptor_again,
            "the preview and the publication agree"
        );
    }

    #[test]
    fn subsystem_edits_update_content_and_children_of_the_staged_descriptor() {
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let stub = unica_metadata_designer::resource_planner::test_support::subsystem_stub_xml(
            "Sales",
            "2.20",
            "55555555-5555-4555-8555-555555555555",
        );
        let stub = stub
            .replace("<Properties>", "<InternalInfo><xr:PropertyState><xr:Property>CommandInterface</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo><Properties><ObjectBelonging>Adopted</ObjectBelonging><ExtendedConfigurationObject>66666666-6666-4666-8666-666666666666</ExtendedConfigurationObject>")
            .replace("<Synonym/>", "<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Sales original</v8:content></v8:item></Synonym>")
            .replace("<Comment/>", "<Comment>Keep&#13;this</Comment>");
        std::fs::write(
            fixture.source_dir().join("Subsystems/Sales.xml"),
            format!("\u{feff}{stub}"),
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let parse = |op: &str, args: serde_json::Value, index: usize| {
            let mut args = args;
            args["at"] = json!("main:Subsystem.Sales");
            IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            )
        };
        let operations = [
            parse(
                "content.add",
                json!({"items": [{"object": "Document.First"}]}),
                0,
            ),
            parse(
                "childSubsystem.add",
                json!({"items": [{"name": "Orders"}]}),
                1,
            ),
        ];
        let (staged, effects) =
            plan_form_resource_batch_owned_for_test(staged, authority, &operations)
                .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 2);
        let text = staged_text(&staged, "Subsystems/Sales.xml");
        assert!(text.contains("Document.First"), "{text}");
        assert!(text.contains("<Subsystem>Orders</Subsystem>"), "{text}");
        // Пустой тег раскрывается так, как пишет платформа: элемент на своей
        // строке с отступом на уровень глубже, закрывающий тег — с отступом
        // контейнера (#1889).
        let indent = |tag: &str| {
            let at = stub.find(tag).unwrap();
            stub[..at].rsplit('\n').next().unwrap().to_owned()
        };
        let (content, children) = (indent("<Content/>"), indent("<ChildObjects/>"));
        let expected = format!("\u{feff}{stub}")
            .replace(
                "<Content/>",
                &format!("<Content>\n{content}\t<xr:Item xsi:type=\"xr:MDObjectRef\">Document.First</xr:Item>\n{content}</Content>"),
            )
            .replace(
                "<ChildObjects/>",
                &format!("<ChildObjects>\n{children}\t<Subsystem>Orders</Subsystem>\n{children}</ChildObjects>"),
            );
        assert_eq!(
            text, expected,
            "only the requested lists change; all other source bytes survive"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.source_dir().join("Subsystems/Sales.xml")).unwrap(),
            format!("\u{feff}{stub}"),
            "planning never writes the source"
        );
        let child = staged_text(&staged, "Subsystems/Sales/Subsystems/Orders.xml");
        assert!(child.contains("<Name>Orders</Name>"), "{child}");
    }

    #[test]
    fn subsystem_membership_batch_noop_keeps_the_exact_original_image() {
        for empty in ["<Content/>", "<Content></Content>", "<Content > </Content>"] {
            let fixture = ApplySeamFixture::new();
            std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
            let original =
                unica_metadata_designer::resource_planner::test_support::subsystem_stub_xml(
                    "Sales",
                    "2.20",
                    "55555555-5555-4555-8555-555555555555",
                )
                .replace("<Content/>", empty)
                .replace('\n', "\r\n");
            let path = fixture.source_dir().join("Subsystems/Sales.xml");
            std::fs::write(&path, original.as_bytes()).unwrap();
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let operations: Vec<_> = ["content.add", "content.remove"].into_iter().enumerate().map(|(index, op)| {
                let args = json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
                IndexedPlanOperation::new(index, parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap())
            }).collect();
            let (mut staged, _) =
                plan_form_resource_batch_owned_for_test(staged, authority, &operations).unwrap();
            assert!(staged.planned_changes().is_empty(), "{empty}");
            assert_eq!(
                staged
                    .read(std::path::Path::new("Subsystems/Sales.xml"))
                    .unwrap()
                    .unwrap(),
                original.as_bytes()
            );
            assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        }
    }

    #[test]
    fn subsystem_membership_restores_original_lists_across_family_batches() {
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let original = unica_metadata_designer::resource_planner::test_support::subsystem_stub_xml(
            "Sales",
            "2.20",
            "55555555-5555-4555-8555-555555555555",
        );
        std::fs::write(fixture.source_dir().join("Subsystems/Sales.xml"), &original).unwrap();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        let relative = std::path::Path::new("Subsystems/Sales.xml");
        for (index, op) in ["content.add", "content.remove"].into_iter().enumerate() {
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let args =
                json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
            let operation = IndexedPlanOperation::new(
                index,
                parse_form_resource_plan_operation(op, &args, index, &fixture.binding).unwrap(),
            );
            (staged, _) =
                plan_form_resource_batch_owned_for_test(staged, authority, &[operation]).unwrap();
            if index == 0 {
                // An intervening family can edit the same descriptor. Restoration
                // must be scoped to membership and keep that staged property.
                let before = staged.read(relative).unwrap().unwrap();
                let after = String::from_utf8(before.clone())
                    .unwrap()
                    .replace("<Comment/>", "<Comment>Changed</Comment>");
                staged
                    .replace(relative, &before, after.into_bytes())
                    .unwrap();
            }
        }
        assert_eq!(
            staged.read(relative).unwrap().unwrap(),
            original
                .replace("<Comment/>", "<Comment>Changed</Comment>")
                .as_bytes()
        );
    }

    #[test]
    fn subsystem_membership_after_props_set_keeps_valid_namespace_bindings() {
        use super::super::metadata::{
            parse_metadata_plan_operation, plan_metadata_batch_owned_for_test,
        };
        let fixture = ApplySeamFixture::new();
        std::fs::create_dir_all(fixture.source_dir().join("Subsystems")).unwrap();
        let source = "<m:MetaDataObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:r=\"http://v8.1c.ru/8.3/xcf/readable\" version=\"2.20\"><m:Subsystem uuid=\"55555555-5555-4555-8555-555555555555\"><m:Properties><m:Name>Sales</m:Name><m:Comment/><m:Content/></m:Properties><m:ChildObjects/></m:Subsystem></m:MetaDataObject>";
        std::fs::write(fixture.source_dir().join("Subsystems/Sales.xml"), source).unwrap();
        let admission = fixture.admission();
        let mut staged = admission.staged_state().unwrap();
        for (index, op) in ["content.add", "content.remove"].into_iter().enumerate() {
            let authority = admission
                .form_resource_planning_authority(&fixture.binding)
                .unwrap();
            let args =
                json!({"at": "main:Subsystem.Sales", "items": [{"object": "Document.First"}]});
            let operation = IndexedPlanOperation::new(
                index * 2,
                parse_form_resource_plan_operation(op, &args, index * 2, &fixture.binding).unwrap(),
            );
            (staged, _) =
                plan_form_resource_batch_owned_for_test(staged, authority, &[operation]).unwrap();
            if index == 0 {
                let authority = admission
                    .metadata_planning_authority(&fixture.binding)
                    .unwrap();
                let args = json!({"at": "main:Subsystem.Sales", "values": {"Comment": "Changed"}});
                let operation = IndexedPlanOperation::new(
                    1,
                    parse_metadata_plan_operation("props.set", &args, 1, &fixture.binding).unwrap(),
                );
                (staged, _) =
                    plan_metadata_batch_owned_for_test(staged, authority, &[operation]).unwrap();
            }
        }
        let actual = staged_text(&staged, "Subsystems/Sales.xml");
        let doc = roxmltree::Document::parse(actual.trim_start_matches('\u{feff}')).unwrap();
        assert!(doc.descendants().any(|n| n
            .has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Comment"))
            && n.text() == Some("Changed")));
        assert!(!doc
            .descendants()
            .any(|n| n.has_tag_name(("http://v8.1c.ru/8.3/xcf/readable", "Item"))));
    }

    #[test]
    fn support_contracts_and_productive_parser_agree_on_closed_arguments() {
        let fixture = ApplySeamFixture::new();
        let capability = [
            (json!({"enabled": true}), true),
            (json!({"enabled": false}), true),
            (json!({"rule": "on"}), true),
            (json!({"rule": "off"}), true),
            (json!({"rule": "EditableSupportEnabled"}), true),
            (json!({"rule": "EditableSupportDisabled"}), true),
            (json!({"rule": "enabled"}), true),
            (json!({"rule": "disabled"}), true),
            (json!({}), false),
            (json!({"enabled": "true"}), false),
            (json!({"enabled": null}), false),
            (json!({"rule": true}), false),
            (json!({"rule": "editable"}), false),
            (json!({"enabled": true, "rule": "on"}), false),
            (json!({"enabled": true, "ignored": 1}), false),
        ];
        let rule = [
            (json!({"rule": "locked"}), true),
            (json!({"rule": "editable"}), true),
            (json!({"rule": "off-support"}), true),
            (json!({"rule": "NotEditable"}), true),
            (json!({"rule": "EditableWithSupport"}), true),
            (json!({"rule": "NotSupported"}), true),
            (json!({}), false),
            (json!({"rule": "on"}), false),
            (json!({"rule": true}), false),
            (json!({"rule": null}), false),
            (json!({"rule": ""}), false),
            (json!({"rule": "editable", "ignored": 1}), false),
        ];
        for (operation, at, cases) in [
            (
                "supportCapability.set",
                "main:Configuration",
                capability.as_slice(),
            ),
            ("supportRule.set", "main:Document.First", rule.as_slice()),
            (
                "supportRule.set",
                "main:Document.First.Form.Card",
                rule.as_slice(),
            ),
        ] {
            let descriptor = crate::domain::apply::operation_registry()
                .lookup(operation)
                .unwrap();
            let contract = descriptor
                .argument_contract()
                .expect("implemented support operation must publish its argument contract");
            assert_eq!(contract.schema["additionalProperties"], false);
            assert!(contract.example_args.get("at").is_none());
            let mut unknown_argument = contract.example_args.clone();
            unknown_argument["ignored"] = json!(true);
            for args in [
                json!({"values": null}),
                json!({"values": []}),
                json!({"values": "editable"}),
                unknown_argument,
            ] {
                let contract_error = contract.normalize(&args, "args").unwrap_err();
                let has_unknown_argument = args.get("ignored").is_some();
                if has_unknown_argument {
                    assert_eq!(contract_error.path, "args.ignored");
                }
                let mut args = args;
                args["at"] = json!(at);
                let error =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                        .unwrap_err();
                assert_eq!(
                    error.kind(),
                    ApplyPlanErrorKind::BadValue,
                    "{operation}: {args}"
                );
                if has_unknown_argument {
                    assert_eq!(error.path(), Some("ops[3].args.ignored"));
                }
            }
            for (values, accepted) in cases {
                let args = json!({"values": values});
                assert_eq!(
                    contract.normalize(&args, "args").is_ok(),
                    *accepted,
                    "contract: {operation} {args}"
                );
                let mut args = args;
                args["at"] = json!(at);
                let parsed =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding);
                assert_eq!(
                    parsed.is_ok(),
                    *accepted,
                    "parser: {operation} {args}: {parsed:?}"
                );
                if let Err(error) = parsed {
                    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
                }
            }
        }
    }

    #[test]
    fn support_aliases_select_the_exact_capability_and_object_rule() {
        use super::FormResourcePlanKind;
        use unica_metadata::support_edit::{SupportCapability, SupportObjectRule};
        let fixture = ApplySeamFixture::new();
        for (values, expected) in [
            (json!({"enabled": true}), SupportCapability::On),
            (json!({"enabled": false}), SupportCapability::Off),
            (json!({"rule": "on"}), SupportCapability::On),
            (json!({"rule": "off"}), SupportCapability::Off),
            (
                json!({"rule": "EditableSupportEnabled"}),
                SupportCapability::On,
            ),
            (
                json!({"rule": "EditableSupportDisabled"}),
                SupportCapability::Off,
            ),
            (json!({"rule": "enabled"}), SupportCapability::On),
            (json!({"rule": "disabled"}), SupportCapability::Off),
        ] {
            let args = json!({"at": "main:Configuration", "values": values});
            let parsed = parse_form_resource_plan_operation(
                "supportCapability.set",
                &args,
                3,
                &fixture.binding,
            )
            .unwrap();
            match parsed.kind {
                FormResourcePlanKind::Metadata(super::MetadataResourcePlan::SupportCapability(
                    actual,
                )) => {
                    assert_eq!(actual, expected, "{args}")
                }
                other => panic!("support capability selected {other:?}: {args}"),
            }
        }
        for (rule, expected) in [
            ("locked", SupportObjectRule::Locked),
            ("editable", SupportObjectRule::Editable),
            ("off-support", SupportObjectRule::OffSupport),
            ("NotEditable", SupportObjectRule::Locked),
            ("EditableWithSupport", SupportObjectRule::Editable),
            ("NotSupported", SupportObjectRule::OffSupport),
        ] {
            for (at, expected_target) in [
                ("main:Document.First", "Document.First"),
                ("main:Document.First.Form.Card", "Document.First.Form.Card"),
            ] {
                let args = json!({"at": at, "values": {"rule": rule}});
                let parsed = parse_form_resource_plan_operation(
                    "supportRule.set",
                    &args,
                    3,
                    &fixture.binding,
                )
                .unwrap();
                match parsed.kind {
                    FormResourcePlanKind::Metadata(super::MetadataResourcePlan::SupportRule {
                        target,
                        rule: actual,
                    }) => {
                        assert_eq!(actual, expected, "{args}");
                        assert_eq!(target.as_str(), expected_target, "{args}");
                    }
                    other => panic!("support rule selected {other:?}: {args}"),
                }
            }
        }
    }

    #[test]
    fn support_target_refusals_precede_argument_contract_refusals() {
        let fixture = ApplySeamFixture::new();
        for operation in ["supportCapability.set", "supportRule.set"] {
            for target in [None, Some(json!(42)), Some(json!(null))] {
                let mut args = json!({"values": {"rule": true}, "unknown": true});
                if let Some(target) = target {
                    args["at"] = target;
                }
                let error =
                    parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                        .expect_err("an invalid target precedes malformed arguments");
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
                assert_eq!(error.path(), Some("ops[3].args.at"), "{operation}");
                assert_eq!(
                    error.to_string(),
                    "`at` must be a logical address",
                    "{operation}"
                );
            }
        }
        for (operation, args, path) in [
            (
                "supportCapability.set",
                json!({"at":"main:Document.First", "values":{"enabled":"wrong"}}),
                "ops[3].args.values",
            ),
            (
                "supportRule.set",
                json!({"at":"main:Configuration", "values":{"rule":true}}),
                "ops[3].args.values.rule",
            ),
        ] {
            let error = parse_form_resource_plan_operation(operation, &args, 3, &fixture.binding)
                .expect_err("a logical target still checks its arguments before its owner kind");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some(path), "{operation}: {error}");
        }
    }

    #[test]
    fn support_rule_form_requires_registered_identity_and_changes_only_its_record() {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let form_path = source.join("Documents/First/Forms/Card.xml");
        std::fs::create_dir_all(form_path.parent().unwrap()).unwrap();
        let form = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Card</Name></Properties></Form></MetaDataObject>"#;
        let owner_path = source.join("Documents/First.xml");
        let owner = std::fs::read_to_string(&owner_path).unwrap();
        let registered = owner.replacen("<ChildObjects>", "<ChildObjects><Form>Card</Form>", 1);
        let marker = "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,1,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111,0,0,bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb,0,0,cccccccc-cccc-4ccc-8ccc-cccccccccccc,cccccccc-cccc-4ccc-8ccc-cccccccccccc}";
        std::fs::create_dir_all(source.join("Ext")).unwrap();
        std::fs::write(source.join("Ext/ParentConfigurations.bin"), marker).unwrap();
        let operation = parse_form_resource_plan_operation(
            "supportRule.set",
            &json!({"at": "main:Document.First.Form.Card", "values": {"rule": "editable"}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        for (owner_image, form_image, succeeds) in [
            (registered.clone(), Some(form.to_string()), true),
            (owner.clone(), Some(form.to_string()), false),
            (registered.clone(), None, false),
            (
                registered.clone(),
                Some(form.replace(
                    "<MetaDataObject ",
                    "<MetaDataObject uuid=\"11111111-1111-4111-8111-111111111111\" ",
                )),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace("<Name>Card</Name>", "<Name>Other</Name>")),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "invalid")),
                false,
            ),
            (
                registered.clone(),
                Some(form.replace(" uuid=\"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb\"", "")),
                false,
            ),
        ] {
            std::fs::write(&owner_path, owner_image).unwrap();
            match form_image {
                Some(text) => std::fs::write(&form_path, text).unwrap(),
                None => std::fs::remove_file(&form_path).unwrap(),
            }
            let admission = fixture.admission();
            let result = plan_form_resource_batch_owned_for_test(
                admission.staged_state().unwrap(),
                admission
                    .form_resource_planning_authority(&fixture.binding)
                    .unwrap(),
                &[IndexedPlanOperation::new(0, operation.clone())],
            );
            assert_eq!(result.is_ok(), succeeds, "{result:?}");
            if let Ok((staged, _)) = result {
                let expected = marker.replace("0,0,bbbbbbbb", "1,0,bbbbbbbb");
                assert_eq!(
                    staged_text(&staged, "Ext/ParentConfigurations.bin")
                        .trim_start_matches('\u{feff}'),
                    expected.trim_start_matches('\u{feff}')
                );
                let batch = admission
                    .prepare_with_effects(staged, Default::default())
                    .unwrap();
                fixture.actor().preview_prepared_apply(&batch).unwrap();
                let token = fixture
                    .actor()
                    .save_prepared_apply(
                        batch,
                        unica_execution_contract::invocation::DomainResult::success(
                            "support preview",
                        ),
                    )
                    .unwrap();
                std::fs::write(
                    &form_path,
                    form.replace("<Name>Card</Name>", "<Name>Changed</Name>"),
                )
                .unwrap();
                let result = fixture
                    .actor()
                    .execute_saved_apply(
                        &token,
                        unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                            std::time::Duration::from_secs(5),
                        ),
                        &unica_work_control::cancellation::CancellationToken::new(),
                        |mut result, publication| {
                            let error = publication
                                .expect_err("changed form descriptor must refuse publication");
                            result.ok = false;
                            result.summary = error.to_string();
                            result
                        },
                    )
                    .unwrap();
                assert!(!result.ok);
                assert!(result.summary.contains("Card.xml"), "{}", result.summary);
            }
            assert_eq!(
                std::fs::read_to_string(source.join("Ext/ParentConfigurations.bin")).unwrap(),
                marker
            );
        }
    }

    #[test]
    fn support_rule_set_rewrites_the_object_record_and_refuses_without_support_state() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_form_resource_plan_operation(
            "supportRule.set",
            &json!({"at": "main:Document.First", "values": {"rule": "editable"}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let error = plan_form_resource_batch_owned_for_test(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation.clone())],
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);

        std::fs::create_dir_all(fixture.source_dir().join("Ext")).unwrap();
        std::fs::write(
            fixture.source_dir().join("Ext/ParentConfigurations.bin"),
            "\u{feff}{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa,0,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}",
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .form_resource_planning_authority(&fixture.binding)
            .unwrap();
        let (staged, effects) = plan_form_resource_batch_owned_for_test(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation)],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(effects.len(), 1);
        let text = staged_text(&staged, "Ext/ParentConfigurations.bin");
        assert!(
            text.contains("1,0,11111111-1111-4111-8111-111111111111"),
            "the object rule flips to editable: {text}"
        );
    }

    /// Имя дочерней подсистемы задаёт адрес: аргумент, который его
    /// повторял, отказывает и в разборе планировщиком (#1353). Форму создаёт
    /// `form.add` на владельце: снятую `form.create` планировщик не знает.
    #[test]
    fn form_and_child_subsystem_names_come_only_from_the_address() {
        let fixture = ApplySeamFixture::new();
        assert!(matches!(
            parse_form_resource_plan_operation(
                "form.create",
                &json!({"at": "main:Catalog.Items.Form.Choice", "values": {"type": "ChoiceForm"}}),
                0,
                &fixture.binding,
            )
            .map(|operation| operation.kind),
            Ok(super::FormResourcePlanKind::Unsupported)
        ));
        for (op, args, path) in [
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales", "items": [{"name": "Orders"}]}),
                "ops[0].args.items",
            ),
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales.Subsystem.Orders", "values": {"name": "Orders"}}),
                "ops[0].args.values",
            ),
            (
                "childSubsystem.remove",
                json!({"at": "main:Subsystem.Sales"}),
                "ops[0].args.at",
            ),
        ] {
            let error = parse_form_resource_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("a name outside the address is refused");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} {path}");
            assert_eq!(error.path(), Some(path), "{op}");
        }
        assert!(parse_form_resource_plan_operation(
            "childSubsystem.remove",
            &json!({"at": "main:Subsystem.Sales.Subsystem.Orders"}),
            0,
            &fixture.binding,
        )
        .is_ok());
    }

    /// `command.add` принимает ровно ключи, которые писатель кладёт
    /// в `<Command>`, и передаёт их писателю; остальное отказывает
    /// `bad_value` до записи, а не теряется молча (#1439).
    #[test]
    fn form_command_add_accepts_only_the_keys_its_writer_writes() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Catalog.Items.Form.Item";
        let item = json!({
            "name": "Refresh",
            "title": "Refresh",
            "tooltip": "Refresh the list",
            "shortcut": "F5",
            "picture": "StdPicture.Refresh",
            "representation": "TextPicture",
        });
        let plan = parse_form_resource_plan_operation(
            "command.add",
            &json!({"at": at, "items": [item.clone()]}),
            0,
            &fixture.binding,
        )
        .expect("every published key is accepted")
        .kind;
        let super::FormResourcePlanKind::Form(unica_forms_designer::planner::FormPlan::FormEdit {
            definitions,
            ..
        }) = plan
        else {
            panic!("command.add plans a form edit");
        };
        let mut expected = item.as_object().unwrap().clone();
        expected.insert("action".into(), json!("Refresh"));
        let [unica_forms_designer::planner::FormEditStep::Definition(definition)] =
            definitions.as_slice()
        else {
            panic!("expected one command definition");
        };
        assert_eq!(definition, &json!({"commands": [expected]}));

        for (field, value) in [
            ("use", json!(false)),
            ("functionalOptions", json!(["FunctionalOption.X"])),
            ("currentRowUse", json!("DontUse")),
            ("modifiesSavedData", json!(true)),
            ("table", json!("Список")),
            ("loadTransparent", json!(false)),
            ("representation", json!("PictureAndText")),
            ("representation", json!("text")),
            ("picture", json!({"src": "StdPicture.Refresh"})),
            ("picture", json!("abs:picture.png")),
            ("title", json!({"ru": "Обновить"})),
            ("title", json!("")),
            ("callType", json!("Before")),
            (
                "actions",
                json!([{"callType": "Before", "handler": "RefreshBefore"}]),
            ),
            ("tooltip", json!({"ru": "Обновить"})),
            ("tooltip", json!("")),
            ("shortcut", json!("")),
        ] {
            let error = parse_form_resource_plan_operation(
                "command.add",
                &json!({"at": at, "items": [{"name": "Refresh", field: value}]}),
                0,
                &fixture.binding,
            )
            .expect_err("a key the writer does not write is refused");
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{field} {value}"
            );
            let path = format!("ops[0].args.items[0].{field}");
            assert_eq!(error.path(), Some(path.as_str()), "{field} {value}");
        }
    }

    #[test]
    fn typed_form_additions_keep_target_precedence_and_entry_transformations() {
        use unica_operations::OperationFamily;
        let fixture = ApplySeamFixture::new();
        for operation in ["attribute.add", "command.add"] {
            let descriptor = crate::domain::apply::operation_registry()
                .named(operation)
                .find(|d| d.family() == OperationFamily::Form)
                .unwrap();
            for at in [json!(42), json!(null)] {
                let error = super::parse_form_resource_plan_operation_in(
                    Some(descriptor),
                    operation,
                    &json!({"at":at,"items":false,"unknown":true}),
                    3,
                    &fixture.binding,
                )
                .unwrap_err();
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
                assert_eq!(error.path(), Some("ops[3].args.at"));
                assert_eq!(error.to_string(), "`at` must be a logical address");
            }
            let error = super::parse_form_resource_plan_operation_in(
                Some(descriptor),
                operation,
                &json!({"at":"main:Document.First.Form.Main.Item.A","items":false}),
                3,
                &fixture.binding,
            )
            .unwrap_err();
            assert_eq!(error.path(), Some("ops[3].args.at"));
            assert_eq!(
                error.to_string(),
                format!("{operation} targets the form `Owner.Name.Form.F`")
            );
        }
        for (operation, item, expected) in [
            (
                "attribute.add",
                json!({"name":" Added ","type":"String"}),
                json!({"attributes":[{"name":"Added","type":"string"}]}),
            ),
            (
                "command.add",
                json!({"name":" Added "}),
                json!({"commands":[{"name":"Added","action":"Added"}]}),
            ),
            (
                "command.add",
                json!({"name":" Added ","action":" Existing "}),
                json!({"commands":[{"name":"Added","action":" Existing "}]}),
            ),
        ] {
            let parsed = parse_form_resource_plan_operation(
                operation,
                &json!({"at":"Document.First.Form.Main","items":[item]}),
                3,
                &fixture.binding,
            )
            .unwrap();
            let super::FormResourcePlanKind::Form(
                unica_forms_designer::planner::FormPlan::FormEdit {
                    owner,
                    form,
                    definitions,
                    ..
                },
            ) = parsed.kind
            else {
                panic!("{operation} must plan a form edit")
            };
            assert_eq!(owner.as_str(), "Document.First");
            assert_eq!(form, "Main");
            let [unica_forms_designer::planner::FormEditStep::Definition(definition)] =
                definitions.as_slice()
            else {
                panic!("expected one definition")
            };
            assert_eq!(definition, &expected);
        }
    }

    #[test]
    fn form_operations_refuse_a_root_target_and_name_the_argument() {
        let fixture = ApplySeamFixture::new();
        let error = parse_form_resource_plan_operation(
            "form.add",
            &json!({"at": "main:Configuration", "items": [{"name": "Форма"}]}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }
}
