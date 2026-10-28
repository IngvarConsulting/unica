//! Form operation parsing and planning over an existing source proposal.
use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_forms::facts::FormInfoElement;
use unica_forms::operations::FormOperation;
use unica_forms::FormItemRole;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::proposal::input::{bad, required_values};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, ApplySourceAuthority, ProvisionalApplyEffect,
};
use unica_source_designer::registration::{
    owner_descriptor_relative, read_required, utf8_body, with_bom,
};
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
#[derive(Debug, Clone)]
pub enum FormEditStep {
    Definition(Value),
    ColumnType {
        attribute: String,
        column: String,
        type_name: String,
    },
    Move {
        request: crate::form_move::FormItemMove,
        location: String,
    },
}
/// Узел формы, который задаёт адрес правки (#1355): цепочка элементов от
/// формы, пустая у самой формы. Узел проверяется по прочитанной форме до
/// записи: отсутствующий даёт `not_found`, а не правку другого узла.
#[derive(Debug, Clone)]
pub enum FormEditSubject {
    /// A data column checked by the column type writer before publication.
    AttributeColumn,
    /// Форма или элемент, на который пишется событие или реквизит.
    Node(Vec<String>),
    /// Форма или элемент-контейнер, куда добавляются элементы.
    Container(Vec<String>),
    /// Удаляемый элемент.
    Removed(Vec<String>),
}
impl FormEditSubject {
    fn items(&self) -> &[String] {
        match self {
            Self::AttributeColumn => &[],
            Self::Node(items) | Self::Container(items) | Self::Removed(items) => items,
        }
    }
}
/// Узел формы по адресу `Owner.Name[.Form.F[.Item.A[.Item.B…]][.Event.E]]`.
pub struct FormTarget {
    owner: MetadataAddress,
    form: Option<String>,
    /// Цепочка элементов от формы: `Item.A.Item.B` даёт `[A, B]`.
    items: Vec<String>,
    /// Событие формы или последнего элемента цепочки.
    event: Option<String>,
    column: Option<(String, String)>,
}
/// Элемент формы по цепочке имён от корня, как её адресует `view`.
pub fn form_item_at<'a>(
    elements: &'a [FormInfoElement],
    chain: &[String],
) -> Option<&'a FormInfoElement> {
    let (first, rest) = chain.split_first()?;
    let item = elements.iter().find(|element| &element.name == first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        form_item_at(&item.children, rest)
    }
}
/// Путь элемента с именем `name` от корня формы: один, ни одного или
/// несколько элементов с этим именем.
pub fn form_item_path(
    elements: &[FormInfoElement],
    name: &str,
) -> unica_operations::recovery::NamedLookup {
    fn collect(
        elements: &[FormInfoElement],
        name: &str,
        prefix: &mut Vec<String>,
        found: &mut Vec<Vec<String>>,
    ) {
        for element in elements {
            prefix.push(element.name.clone());
            if element.name == name {
                found.push(prefix.clone());
            }
            collect(&element.children, name, prefix, found);
            prefix.pop();
        }
    }
    let mut found = Vec::new();
    collect(elements, name, &mut Vec::new(), &mut found);
    match found.len() {
        0 => unica_operations::recovery::NamedLookup::Missing,
        1 => unica_operations::recovery::NamedLookup::Found(found.remove(0)),
        _ => unica_operations::recovery::NamedLookup::Ambiguous(found),
    }
}
/// Дерево элементов формы, как его читает `view`.
pub fn form_items_of(text: &str) -> Result<Vec<FormInfoElement>, String> {
    crate::codec::parse_form_event_evidence_xml(text).map(|evidence| evidence.elements)
}
/// Узел правки есть в форме и подходит операции; сосед `after` нового
/// элемента лежит в том же контейнере.
pub fn check_form_edit_subject(
    xml_text: &str,
    subject: &FormEditSubject,
    definitions: &[FormEditStep],
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    if matches!(subject, FormEditSubject::AttributeColumn) {
        return Ok(());
    }
    let at_path = format!("ops[{op_index}].args.at");
    let elements = form_items_of(xml_text).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error).at_path(at_path.clone())
    })?;
    let chain = subject.items();
    let siblings = match chain {
        [] => &elements[..],
        chain => {
            let item = form_item_at(&elements, chain).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    format!(
                        "form item `Item.{}` was not found; read the form items to address an existing one",
                        chain.join(".Item.")
                    ),
                )
                .at_path(at_path.clone())
            })?;
            let role = FormItemRole::from_tag(&item.tag);
            match (subject, role) {
                (FormEditSubject::Container(_), FormItemRole::Leaf) => {
                    return Err(bad(
                        op_index,
                        "at",
                        format!(
                            "form item `{}` {} does not hold elements; item.add targets the form or a container item",
                            item.name, item.tag
                        ),
                    ))
                }
                (FormEditSubject::Removed(_), FormItemRole::AutoCommandBar) => {
                    return Err(bad(
                        op_index,
                        "at",
                        format!(
                            "`{}` is the automatic command bar of its owner and is removed only with it",
                            item.name
                        ),
                    ))
                }
                _ => {}
            }
            &item.children[..]
        }
    };
    if matches!(subject, FormEditSubject::Container(_)) {
        // Элементы, добавленные раньше в том же `items`, к моменту
        // следующего уже стоят в контейнере и тоже служат соседями.
        let mut added: Vec<&str> = Vec::new();
        for (index, step) in definitions.iter().enumerate() {
            let FormEditStep::Definition(definition) = step else {
                continue;
            };
            if let Some(after) = definition.get("after").and_then(Value::as_str) {
                if !siblings.iter().any(|sibling| sibling.name == after) && !added.contains(&after)
                {
                    return Err(bad(
                        op_index,
                        &format!("items[{index}].after"),
                        format!(
                            "`after` names a sibling inside the addressed container; `{after}` is not there"
                        ),
                    ));
                }
            }
            added.extend(
                definition
                    .get("elements")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|element| element.get("name").and_then(Value::as_str)),
            );
        }
    }
    Ok(())
}
pub fn form_target(
    target: &QualifiedAddress,
    op_index: usize,
) -> Result<FormTarget, ApplyPlanError> {
    let segments = target.segments();
    let [owner, rest @ ..] = segments else {
        return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE));
    };
    if owner.kind() == NodeKind::Configuration || owner.kind() == NodeKind::Form {
        return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad(op_index, "at", "the form owner must be named"))?;
    let address = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.{owner_name}", owner.kind().as_str()),
    )
    .map_err(|error| bad(op_index, "at", error.to_string()))?;
    let named = |segment: &unica_address::address::AddressSegment, what: &str| {
        segment
            .name()
            .map(str::to_string)
            .ok_or_else(|| bad(op_index, "at", format!("the {what} must be named")))
    };
    let (form, items, event, column) = match rest {
        [] => (None, Vec::new(), None, None),
        [form, attribute, column]
            if form.kind() == NodeKind::Form
                && attribute.kind() == NodeKind::Attribute
                && column.kind() == NodeKind::Column =>
        {
            (
                Some(named(form, "form")?),
                Vec::new(),
                None,
                Some((named(attribute, "attribute")?, named(column, "column")?)),
            )
        }
        [form, tail @ ..] if form.kind() == NodeKind::Form => {
            let (items, event) = match tail {
                [items @ .., last] if last.kind() == NodeKind::Event => {
                    (items, Some(named(last, "form event")?))
                }
                items => (items, None),
            };
            let items = items
                .iter()
                .map(|item| {
                    if item.kind() == NodeKind::Item {
                        named(item, "form item")
                    } else {
                        Err(bad(op_index, "at", FORM_ADDRESS_SHAPE))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            (Some(named(form, "form")?), items, event, None)
        }
        _ => return Err(bad(op_index, "at", FORM_ADDRESS_SHAPE)),
    };
    Ok(FormTarget {
        owner: address,
        form,
        items,
        event,
        column,
    })
}
pub fn form_purpose(
    kind: Option<&str>,
    op_index: usize,
    location: &str,
) -> Result<String, ApplyPlanError> {
    Ok(match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
        None | Some("ObjectForm") | Some("Object") | Some("object") => "Object".to_string(),
        Some("ListForm") | Some("List") | Some("list") => "List".to_string(),
        Some("ChoiceForm") | Some("Choice") | Some("choice") => "Choice".to_string(),
        Some("RecordForm") | Some("RecordSetForm") | Some("Record") | Some("record") => {
            "Record".to_string()
        }
        Some("SettingsForm") | Some("Settings") | Some("FolderForm") | Some("Generic") => {
            "Object".to_string()
        }
        Some(other) => {
            return Err(bad(
                op_index,
                &format!("{location}.type"),
                format!(
                    "unknown form type `{other}`; use ObjectForm, ListForm, ChoiceForm, RecordSetForm or SettingsForm"
                ),
            ))
        }
    })
}
pub fn default_form_property(key: &str) -> Option<&'static str> {
    unica_forms::contracts::FORM_SLOTS
        .iter()
        .find(|(slot, _)| *slot == key)
        .map(|(_, tag)| *tag)
}
/// Тип реквизита формы с квалификаторами, которые пишет выгрузка 8.3.27.
/// Имена контракта `String`, `Number`, `Date`, `Boolean` (и русские) раньше
/// уходили голыми `xs:*`, и платформа дописывала умолчания (#1996). Теперь
/// строка — длина 0 переменной длины, дата — «дата и время», как понимает
/// платформа, а число — 10 знаков, как ставит конфигуратор и как у колонки
/// таблицы значений. Словарь кодека (`string`, `decimal`, `date`) проходит
/// как есть. Составной тип отображается по частям.
pub fn form_attribute_type(type_name: &str) -> String {
    type_name
        .split(['|', '+'])
        .map(|part| match part.trim() {
            "Boolean" | "Булево" => "boolean",
            "String" | "Строка" => "string",
            "Number" | "Число" => "decimal",
            "Date" | "Дата" => "dateTime",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
/// The legacy element definition for one typed v0.13 item: the discriminator
/// key named after the element type, every other property passed through.
/// The item has passed the closed `item.add` contract: its keys are the
/// ones the writer of its kind writes.
pub fn legacy_element_definition(
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(Value, Option<String>), ApplyPlanError> {
    let name =
        unica_source_contract::proposal::input::required_string(item, "name", op_index, location)?
            .to_string();
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("InputField");
    let mut definition = Map::new();
    definition.insert("name".to_string(), Value::String(name.clone()));
    let orientation = || {
        Value::String(
            item.get("orientation")
                .and_then(Value::as_str)
                .unwrap_or("vertical")
                .to_string(),
        )
    };
    match kind {
        "InputField" | "Input" | "Field" => {}
        "HTMLDocumentField" => {
            definition.insert("html".to_string(), Value::String(name.clone()));
        }
        "TextDocumentField" => {
            definition.insert("textDoc".to_string(), Value::String(name.clone()));
        }
        "Group" | "UsualGroup" => {
            definition.insert("group".to_string(), orientation());
        }
        "Table" => {
            definition.insert("table".to_string(), Value::String(name.clone()));
        }
        "Button" => {
            definition.insert("button".to_string(), Value::String(name.clone()));
        }
        "Label" | "LabelDecoration" => {
            definition.insert("label".to_string(), Value::String(name.clone()));
        }
        "LabelField" => {
            definition.insert("labelField".to_string(), Value::String(name.clone()));
        }
        "CheckBox" | "CheckBoxField" => {
            definition.insert("check".to_string(), Value::String(name.clone()));
        }
        "Pages" => {
            definition.insert("pages".to_string(), Value::String(name.clone()));
        }
        "Page" => {
            definition.insert("page".to_string(), Value::String(name.clone()));
            // Ориентацию страницы пишет её `group`; прежде `orientation`
            // у страницы молча пропадал (#1447).
            if item.contains_key("orientation") {
                definition.insert("group".to_string(), orientation());
            }
        }
        "CommandBar" => {
            definition.insert("cmdBar".to_string(), Value::String(name.clone()));
        }
        other => {
            return Err(bad(
                op_index,
                &format!("{location}.type"),
                format!(
                    "unknown form element type `{other}`; use InputField, Group, Table, Button, Label, LabelField, CheckBox, Pages, Page, CommandBar, HTMLDocumentField or TextDocumentField"
                ),
            ))
        }
    }
    let mut after = None;
    for (key, value) in item {
        match key.as_str() {
            "name" | "type" | "orientation" => {}
            "after" => after = value.as_str().map(str::to_string),
            _ => {
                definition.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((Value::Object(definition), after))
}
/// Ключи, которые `item.add` снял по Р2 (#1447), и внутренние различители
/// вида: отказ с продолжением до проверки контракта, чтобы агент получил
/// следующий вызов, а не общий «неизвестный аргумент».
pub fn refuse_nested_element_content(
    at: &str,
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(), ApplyPlanError> {
    use unica_forms::contracts::{FORM_ELEMENT_EVENT_KEYS, FORM_ELEMENT_NESTED_KEYS};
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<Element>");
    let container = format!("{at}.Item.{}", unica_address::address::escape_name(name));
    let contract = serde_json::json!({
        "tool": "unica.view",
        "args": {"at": format!("{at}.Can.item.add")},
        "reason": "контракт item.add: один элемент на запись, ключи по виду",
    });
    if item.contains_key("html") {
        return Err(bad(
            op_index,
            &format!("{location}.html"),
            "html is an internal discriminator; use name and type: HTMLDocumentField in item.add",
        )
        .with_next(contract));
    }
    if item.contains_key("textDoc") {
        return Err(bad(
            op_index,
            &format!("{location}.textDoc"),
            "textDoc is an internal discriminator; use name and type: TextDocumentField in item.add",
        )
        .with_next(contract));
    }
    if let Some(key) = FORM_ELEMENT_NESTED_KEYS
        .iter()
        .find(|key| item.contains_key(**key))
    {
        return Err(bad(
            op_index,
            &format!("{location}.{key}"),
            format!(
                "item.add adds one element without nested content; drop `{key}` and add each nested element with a separate item.add at `{container}` once this element exists"
            ),
        )
        .with_next(contract));
    }
    if let Some(key) = FORM_ELEMENT_EVENT_KEYS
        .iter()
        .find(|key| item.contains_key(**key))
    {
        return Err(bad(
            op_index,
            &format!("{location}.{key}"),
            format!(
                "item.add does not bind events; drop `{key}` and bind each handler with event.bind at `{container}.Event.<Event>` once this element exists, naming the handler explicitly"
            ),
        )
        .with_next(contract));
    }
    Ok(())
}
/// Перекрёстные правила ключей элемента, которых схема не выражает.
pub fn check_element_cross_fields(
    item: &Map<String, Value>,
    op_index: usize,
    location: &str,
) -> Result<(), ApplyPlanError> {
    use unica_forms::contracts::FORM_DYNAMIC_LIST_KEYS;
    if item.get("_dynList").and_then(Value::as_bool) != Some(true) {
        if let Some(key) = FORM_DYNAMIC_LIST_KEYS
            .iter()
            .find(|key| item.contains_key(**key))
        {
            return Err(bad(
                op_index,
                &format!("{location}.{key}"),
                format!("`{key}` is a dynamic-list table property: it is written only with `_dynList: true`"),
            ));
        }
    }
    let commands = ["command", "commandName", "stdCommand"]
        .into_iter()
        .filter(|key| item.contains_key(*key))
        .collect::<Vec<_>>();
    if let [_, second, ..] = commands.as_slice() {
        return Err(bad(
            op_index,
            &format!("{location}.{second}"),
            format!(
                "a button runs one command: give one of `command`, `commandName`, `stdCommand`, not {}",
                commands
                    .iter()
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        ));
    }
    Ok(())
}
pub fn parse_form_root_addition(
    target: FormTarget,
    args: &Map<String, Value>,
    op_index: usize,
    key: &str,
    transform: impl Fn(&Map<String, Value>, &mut Map<String, Value>, &str),
) -> Result<FormPlan, ApplyPlanError> {
    let form = target.form.ok_or_else(|| {
        bad(
            op_index,
            "at",
            "this operation addresses one form: `Owner.Name.Form.F`",
        )
    })?;
    let items = args
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
    let mut entries = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let location = format!("items[{index}]");
        let item = item
            .as_object()
            .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
        let name = unica_source_contract::proposal::input::required_string(
            item, "name", op_index, &location,
        )?
        .to_string();
        let mut entry = item.clone();
        entry.insert("name".to_string(), Value::String(name.clone()));
        transform(item, &mut entry, &name);
        entries.push(Value::Object(entry));
    }
    if entries.is_empty() {
        return Err(bad(
            op_index,
            &format!("ops[{op_index}].args.items"),
            "`items` must list at least one entry",
        ));
    }
    let mut wrapper = Map::new();
    wrapper.insert(key.to_string(), Value::Array(entries));
    Ok(FormPlan::FormEdit {
        owner: target.owner,
        form,
        subject: FormEditSubject::Node(Vec::new()),
        definitions: vec![FormEditStep::Definition(Value::Object(wrapper))],
    })
}
pub fn parse_form_operation(
    operation: FormOperation,
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<FormPlan, ApplyPlanError> {
    let operation_name = operation.name();
    let address =
        unica_source_contract::proposal::input::qualified_target(args, op_index, source_set_name)?;
    let target = form_target(&address, op_index)?;
    if target.column.is_some() && !unica_forms::applicability::is_form_column(address.segments()) {
        return Err(bad(
            op_index,
            "at",
            "a data column requires an owner with managed forms",
        ));
    }
    // Узел, на котором исполняется операция, — ровно её `at`: элемент и
    // событие задаются адресом, а операция другого узла отказывает, а не
    // пишет молча в форму, элемент или владельца (#1351, #1355).
    let placed = (target.column.is_none() || operation == FormOperation::PropsSet)
        && match operation {
            FormOperation::Add | FormOperation::Set => target.form.is_none(),
            FormOperation::ItemAdd => target.form.is_some() && target.event.is_none(),
            FormOperation::ItemRemove | FormOperation::ItemMove => {
                !target.items.is_empty() && target.event.is_none()
            }
            FormOperation::EventBind => target.event.is_some(),
            FormOperation::Remove
            | FormOperation::AttributeAdd
            | FormOperation::CommandAdd
            | FormOperation::PropsSet => {
                target.form.is_some() && target.items.is_empty() && target.event.is_none()
            }
        };
    if !placed {
        let shape = match operation {
            FormOperation::Add | FormOperation::Set => "the form owner `Owner.Name`, not a form",
            FormOperation::ItemAdd => {
                "the form `Owner.Name.Form.F` or its item `….Item.A[.Item.B…]` that receives the elements"
            }
            FormOperation::ItemRemove | FormOperation::ItemMove => {
                "the existing item `Owner.Name.Form.F.Item.A[.Item.B…]`"
            }
            FormOperation::EventBind => "the event `Owner.Name.Form.F[.Item.A…].Event.E`",
            FormOperation::Remove
            | FormOperation::AttributeAdd
            | FormOperation::CommandAdd
            | FormOperation::PropsSet => "the form `Owner.Name.Form.F`",
        };
        return Err(bad(
            op_index,
            "at",
            format!("{operation_name} targets {shape}"),
        ));
    }
    // Свойство-ссылку корня формы операция знает, но не пишет: отказ
    // называет причину, а не «неизвестный аргумент».
    if operation == FormOperation::PropsSet && target.column.is_none() {
        if let Some(values) = args.get("values").and_then(Value::as_object) {
            for name in values.keys() {
                if unica_forms::root_properties::root_property(name).is_some_and(|(_, property)| {
                    property.value == unica_forms::root_properties::RootValue::Reference
                }) {
                    return Err(bad(
                        op_index,
                        &format!("values.{name}"),
                        format!("`{name}` names a form attribute, item or object; props.set does not write it"),
                    ));
                }
            }
        }
    }
    if operation == FormOperation::ItemAdd {
        if let Some(items) = args.get("items").and_then(Value::as_array) {
            for (index, item) in items.iter().enumerate() {
                if let Some(item) = item.as_object() {
                    refuse_nested_element_content(
                        &address.to_string(),
                        item,
                        op_index,
                        &format!("items[{index}]"),
                    )?;
                }
            }
        }
    }
    // Опубликованный контракт и разбор планировщиком принимают одну форму
    // аргументов: ключ, которого писатель не пишет, отказывает до записи,
    // а не теряется молча (#1447).
    let contract = operation.contract_for_address(&address);
    {
        let mut checked = args.clone();
        // `at` операции уже разобран в адрес; контракт сверяет остальное.
        checked.insert("at".to_string(), Value::String(address.to_string()));
        contract
            .normalize(&Value::Object(checked), &format!("ops[{op_index}].args"))
            .map_err(|error| {
                let contract_at = format!("{address}.Can.{operation_name}");
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    format!(
                        "{}: {}; `{operation_name}` writes only the arguments listed at `{contract_at}`",
                        error.path, error.message
                    ),
                )
                .at_path(error.path)
                .with_next(serde_json::json!({
                    "tool": "unica.view",
                    "args": {"at": contract_at},
                    "reason": "аргументы, которые операция записывает на этом узле",
                }))
            })?;
    }
    let items_path = format!("ops[{op_index}].args.items");
    let form_of = |target: &FormTarget| -> Result<String, ApplyPlanError> {
        target.form.clone().ok_or_else(|| {
            bad(
                op_index,
                "at",
                "this operation addresses one form: `Owner.Name.Form.F`",
            )
        })
    };
    match operation {
        FormOperation::AttributeAdd => parse_form_root_addition(
            target,
            args,
            op_index,
            "attributes",
            |item, entry, _name| {
                if let Some(type_name) = item.get("type").and_then(Value::as_str) {
                    entry.insert(
                        "type".to_string(),
                        Value::String(form_attribute_type(type_name)),
                    );
                }
            },
        ),
        FormOperation::PropsSet => {
            let form = form_of(&target)?;
            if let Some((attribute, column)) = target.column {
                let type_name = args["values"]["type"]
                    .as_str()
                    .expect("column contract validated the type");
                let type_name = form_attribute_type(type_name);
                crate::column_type::validate_type(&type_name)
                    .map_err(|error| column_type_error(error, op_index))?;
                return Ok(FormPlan::FormEdit {
                    owner: target.owner,
                    form,
                    subject: FormEditSubject::AttributeColumn,
                    definitions: vec![FormEditStep::ColumnType {
                        attribute,
                        column,
                        type_name,
                    }],
                });
            }
            let values = args
                .get("values")
                .cloned()
                .ok_or_else(|| bad(op_index, "values", "`values` must be an object"))?;
            let mut wrapper = Map::new();
            wrapper.insert("rootProperties".to_string(), values);
            Ok(FormPlan::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(Vec::new()),
                definitions: vec![FormEditStep::Definition(Value::Object(wrapper))],
            })
        }
        FormOperation::CommandAdd => {
            parse_form_root_addition(target, args, op_index, "commands", |_item, entry, name| {
                if !entry.contains_key("action") {
                    entry.insert("action".to_string(), Value::String(name.to_string()));
                }
            })
        }
        FormOperation::Add => {
            if target.form.is_some() {
                return Err(bad(
                    op_index,
                    "at",
                    "form.add targets the owner; name the forms in `items`",
                ));
            }
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            let mut forms = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                let name = unica_source_contract::proposal::input::required_string(
                    item, "name", op_index, &location,
                )?
                .to_string();
                let purpose = form_purpose(
                    item.get("type").and_then(Value::as_str),
                    op_index,
                    &location,
                )?;
                forms.push((name, purpose));
            }
            if forms.is_empty() {
                return Err(bad(
                    op_index,
                    "items",
                    "`items` must list at least one form",
                ));
            }
            Ok(FormPlan::FormAdd {
                owner: target.owner,
                forms,
            })
        }
        FormOperation::Set => {
            let values = args
                .get("values")
                .and_then(Value::as_object)
                .ok_or_else(|| bad(op_index, "values", "`values` must be an object"))?;
            let mut defaults = Vec::new();
            let owner_kind = target.owner.segments().next();
            let kind_slots = owner_kind.and_then(unica_forms::contracts::form_slots_of);
            for (key, value) in values {
                let property = default_form_property(key)
                    .filter(|_| kind_slots.is_none_or(|slots| slots.contains(&key.as_str())))
                    .ok_or_else(|| {
                        let slots = kind_slots.map_or_else(
                            || {
                                unica_forms::contracts::FORM_SLOTS
                                    .iter()
                                    .map(|(slot, _)| *slot)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
                            |slots| slots.join(", "),
                        );
                        bad(
                            op_index,
                            &format!("values.{key}"),
                            format!("unknown form slot `{key}` for this owner; use {slots}"),
                        )
                    })?;
                // An empty name clears the slot, as an export of an owner
                // without such a form writes `<Slot/>` (#2144).
                let slot_value = || {
                    bad(
                        op_index,
                        &format!("values.{key}"),
                        "the form slot takes a form name, or an empty string to clear it",
                    )
                };
                let raw = value.as_str().ok_or_else(slot_value)?;
                let form_name = if raw.is_empty() {
                    String::new()
                } else {
                    let trimmed = raw.trim();
                    let name = trimmed.rsplit(".Form.").next().unwrap_or(trimmed);
                    if name.is_empty() {
                        return Err(slot_value());
                    }
                    name.to_string()
                };
                defaults.push((property.to_string(), form_name));
            }
            if defaults.is_empty() {
                return Err(bad(
                    op_index,
                    "values",
                    "form.set needs at least one form slot",
                ));
            }
            Ok(FormPlan::FormSet {
                owner: target.owner,
                defaults,
            })
        }
        FormOperation::Remove => {
            let form = form_of(&target)?;
            Ok(FormPlan::FormRemove {
                owner: target.owner,
                form,
            })
        }
        FormOperation::ItemAdd => {
            let form = form_of(&target)?;
            let items = args
                .get("items")
                .and_then(Value::as_array)
                .ok_or_else(|| bad(op_index, "items", "`items` must be an array"))?;
            // Контейнер — последний элемент адреса; у самой формы — корень.
            // Сосед `after` лежит в том же контейнере и задаёт место сам.
            let container = target.items.last().cloned();
            let mut definitions = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let location = format!("items[{index}]");
                let item = item
                    .as_object()
                    .ok_or_else(|| bad(op_index, &location, "each item must be an object"))?;
                check_element_cross_fields(item, op_index, &location)?;
                let (definition, after) = legacy_element_definition(item, op_index, &location)?;
                let mut wrapper = Map::new();
                wrapper.insert("elements".to_string(), Value::Array(vec![definition]));
                match (after, &container) {
                    (Some(after), _) => {
                        wrapper.insert("after".to_string(), Value::String(after));
                    }
                    (None, Some(container)) => {
                        wrapper.insert("into".to_string(), Value::String(container.clone()));
                    }
                    (None, None) => {}
                }
                definitions.push(FormEditStep::Definition(Value::Object(wrapper)));
            }
            if definitions.is_empty() {
                return Err(bad(
                    op_index,
                    &items_path,
                    "`items` must list at least one element",
                ));
            }
            Ok(FormPlan::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Container(target.items),
                definitions,
            })
        }
        FormOperation::ItemMove => {
            let form = form_of(&target)?;
            let normalized = unica_forms::contracts::form_move()
                .normalize(
                    &Value::Object(args.clone()),
                    &format!("ops[{op_index}].args"),
                )
                .map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message)
                        .at_path(error.path)
                })?;
            let values = &normalized["values"];
            let into = match values.get("into") {
                Some(Value::Null) => crate::form_move::FormMoveDestination::Root,
                Some(Value::String(name)) => {
                    crate::form_move::FormMoveDestination::Named(name.clone())
                }
                _ => unreachable!("the item.move contract normalizes an explicit into"),
            };
            let request = crate::form_move::FormItemMove {
                name: target.items.last().expect("move addresses an item").clone(),
                source_path: Some(target.items.clone()),
                into,
                after: values
                    .get("after")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            Ok(FormPlan::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(target.items),
                definitions: vec![FormEditStep::Move {
                    request,
                    location: format!("ops[{op_index}].args.values"),
                }],
            })
        }
        FormOperation::ItemRemove => {
            let form = form_of(&target)?;
            // Элемент назван адресом: список в аргументах называл бы другие
            // узлы, и молча его не отбрасываем.
            if let Some(key) = ["items", "values"]
                .into_iter()
                .find(|key| args.contains_key(*key))
            {
                return Err(bad(
                    op_index,
                    key,
                    "item.remove removes the item named by its address; drop the argument",
                ));
            }
            let name = target
                .items
                .last()
                .cloned()
                .expect("item.remove target carries an item");
            Ok(FormPlan::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Removed(target.items),
                definitions: vec![FormEditStep::Definition(
                    serde_json::json!({"removeElements": [{"name": name}]}),
                )],
            })
        }
        FormOperation::EventBind => {
            let form = form_of(&target)?;
            // Событие и его элемент задаются адресом: `values` несут только
            // обработчик и `callType`.
            let values = required_values(args, op_index, &["handler", "callType"])?;
            let handler = unica_source_contract::proposal::input::required_string(
                values, "handler", op_index, "values",
            )?;
            let event = target
                .event
                .as_deref()
                .expect("event.bind target carries an event");
            // Имя события в адресе — имя реестра, как его публикует `view`;
            // псевдонимов у адреса нет.
            let mut entry = Map::new();
            entry.insert("name".to_string(), Value::String(event.to_string()));
            entry.insert("handler".to_string(), Value::String(handler.to_string()));
            if let Some(call_type) = values.get("callType").and_then(Value::as_str) {
                entry.insert("callType".to_string(), Value::String(call_type.to_string()));
            }
            let definition = match target.items.last() {
                Some(element) => {
                    entry.insert("element".to_string(), Value::String(element.clone()));
                    serde_json::json!({"elementEvents": [Value::Object(entry)]})
                }
                None => serde_json::json!({"formEvents": [Value::Object(entry)]}),
            };
            Ok(FormPlan::FormEdit {
                owner: target.owner,
                form,
                subject: FormEditSubject::Node(target.items),
                definitions: vec![FormEditStep::Definition(definition)],
            })
        }
    }
}
/// Clears every default-form slot of the owner that points at `form`.
pub fn clear_default_form_slots(owner_text: &str, reference: &str) -> String {
    let mut text = owner_text.to_string();
    for (_, property) in unica_forms::contracts::FORM_SLOTS {
        let filled = format!("<{property}>{reference}</{property}>");
        if text.contains(&filled) {
            text = text.replace(&filled, &format!("<{property}/>"));
        }
    }
    text
}

/// The owner text with the slot `property` written empty, `<Property/>`, as
/// the export writes an owner without such a form.
fn clear_form_default_property(owner_text: &str, property: &str) -> String {
    let start_tag = format!("<{property}>");
    let end_tag = format!("</{property}>");
    let Some(start) = owner_text.find(&start_tag) else {
        return owner_text.to_string();
    };
    let Some(relative_end) = owner_text[start..].find(&end_tag) else {
        return owner_text.to_string();
    };
    let end = start + relative_end + end_tag.len();
    format!(
        "{}<{property}/>{}",
        &owner_text[..start],
        &owner_text[end..]
    )
}
pub fn form_owner_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Result<Option<roxmltree::Node<'a, 'input>>, String> {
    let mut children = parent
        .children()
        .filter(|child| child.has_tag_name((FORM_OWNER_MD_NS, name)));
    let child = children.next();
    if children.next().is_some() {
        return Err(format!(
            "the form owner has ambiguous direct {name} elements"
        ));
    }
    Ok(child)
}
pub fn form_owner_qname<'t>(text: &'t str, node: roxmltree::Node<'_, '_>) -> &'t str {
    text[node.range()]
        .trim_start_matches('<')
        .split([' ', '\t', '\r', '\n', '/', '>'])
        .next()
        .unwrap_or("")
}
pub fn form_owner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|node| node.text())
        .collect()
}
/// Byte ranges come from the parsed preimage, but the emitted image consists
/// of its original slices. In particular, two separate edits must not turn
/// the mixed-EOL bytes between them into a serialization envelope.
pub fn form_add_owner_image(
    text: &str,
    owner: &MetadataAddress,
    detected_owner: (&str, &str),
    form_name: &str,
    default_property: &str,
    at_path: &str,
    name_path: &str,
) -> Result<String, ApplyPlanError> {
    use unica_metadata::metadata_name_eq;
    let (object_type, object_name) = detected_owner;
    let invalid =
        |message| ApplyPlanError::new(ApplyPlanErrorKind::InvalidState, message).at_path(at_path);
    let document = roxmltree::Document::parse(text)
        .map_err(|error| invalid(format!("XML parse error in form owner: {error}")))?;
    let root = document.root_element();
    if !root.has_tag_name((FORM_OWNER_MD_NS, "MetaDataObject")) {
        return Err(invalid(
            "the form owner must be an MDClasses MetaDataObject".into(),
        ));
    }
    let object = form_owner_child(root, object_type)
        .map_err(invalid)?
        .ok_or_else(|| invalid(format!("the form owner is missing direct {object_type}")))?;
    let properties = form_owner_child(object, "Properties")
        .map_err(invalid)?
        .ok_or_else(|| invalid("the form owner is missing direct Properties".into()))?;
    let name = form_owner_child(properties, "Name")
        .map_err(invalid)?
        .map(form_owner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("the form owner is missing its direct Name".into()))?;
    if !metadata_name_eq(&format!("{object_type}.{name}"), owner.as_str())
        || !metadata_name_eq(&name, object_name)
    {
        return Err(invalid(format!(
            "the form owner identity does not match `{}`",
            owner.as_str()
        )));
    }
    // A missing or ambiguous registration container cannot yield a successful
    // publication of orphan form files. No replacement container is invented.
    let children = form_owner_child(object, "ChildObjects")
        .map_err(invalid)?
        .ok_or_else(|| invalid("the form owner is missing direct ChildObjects".into()))?;
    for registration in children
        .children()
        .filter(|node| node.has_tag_name((FORM_OWNER_MD_NS, "Form")))
    {
        let existing = if registration.children().any(|node| node.is_element()) {
            form_owner_child(registration, "Properties")
                .map_err(invalid)?
                .map(|properties| form_owner_child(properties, "Name"))
                .transpose()
                .map_err(invalid)?
                .flatten()
                .map(form_owner_text)
        } else {
            Some(form_owner_text(registration))
        };
        let Some(existing) = existing.filter(|name| !name.trim().is_empty()) else {
            return Err(invalid(
                "the form owner has an unidentified Form registration".into(),
            ));
        };
        if metadata_name_eq(existing.trim(), form_name) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!("form `{form_name}` already exists on `{}`", owner.as_str()),
            )
            .at_path(name_path));
        }
    }

    let qname = form_owner_qname(text, children);
    let form_qname = qname
        .rsplit_once(':')
        .map_or_else(|| "Form".to_owned(), |(prefix, _)| format!("{prefix}:Form"));
    let entry = format!(
        "<{form_qname}>{}</{form_qname}>",
        unica_xml_format::text::escape_xml_text(form_name)
    );
    let child_range = children.range();
    let child_text = &text[child_range.clone()];
    let mut edits = Vec::new();
    if child_text.ends_with("/>") {
        // 8.3.27 expands an empty container on its own line: the entry one
        // level deeper, the closing tag at the container's indentation, the
        // observed line ending. Compact XML keeps the inline expansion.
        let line_start = text[..child_range.start]
            .rfind(['\r', '\n'])
            .map_or(0, |index| index + 1);
        let indentation = &text[line_start..child_range.start];
        let rest = &text[child_range.end..];
        let rest = &rest[..rest.find(['\r', '\n']).unwrap_or(rest.len())];
        let replacement = if line_start > 0
            && indentation.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
            && rest.trim().is_empty()
        {
            let ending = if text[..line_start].ends_with("\r\n") {
                "\r\n"
            } else if text[..line_start].ends_with('\r') {
                "\r"
            } else {
                "\n"
            };
            format!(">{ending}{indentation}\t{entry}{ending}{indentation}</{qname}>")
        } else {
            format!(">{entry}</{qname}>")
        };
        edits.push((child_range.end - 2..child_range.end, replacement));
    } else {
        let order = unica_source_designer::registration::platform_child_object_order(object_type);
        let form_slot = order.and_then(|order| order.iter().position(|kind| *kind == "Form"));
        let successor = children
            .children()
            .filter(|node| {
                node.is_element() && node.tag_name().namespace() == Some(FORM_OWNER_MD_NS)
            })
            .find(|node| match (order, form_slot) {
                (Some(order), Some(form_slot)) => order
                    .iter()
                    .position(|kind| *kind == node.tag_name().name())
                    .is_some_and(|slot| slot > form_slot),
                _ => node.tag_name().name() == "Template",
            });
        let anchor = successor.map_or_else(
            || {
                child_range.start
                    + child_text
                        .rfind("</")
                        .expect("parsed expanded ChildObjects has a closing tag")
            },
            |node| node.range().start,
        );
        let line_start = text[..anchor]
            .rfind(['\r', '\n'])
            .map_or(0, |index| index + 1);
        let indentation = &text[line_start..anchor];
        if line_start > 0 && indentation.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            let ending = if text[..line_start].ends_with("\r\n") {
                "\r\n"
            } else if text[..line_start].ends_with('\r') {
                "\r"
            } else {
                "\n"
            };
            let extra_indent = if successor.is_some() { "" } else { "\t" };
            edits.push((
                line_start..line_start,
                format!("{indentation}{extra_indent}{entry}{ending}"),
            ));
        } else {
            // Compact XML has no local line boundary. An inline fragment
            // preserves both its original layout and the shared kind order.
            edits.push((anchor..anchor, entry));
        }
    }

    if let Some(default) = form_owner_child(properties, default_property).map_err(invalid)? {
        let range = default.range();
        let raw = &text[range.clone()];
        let value = unica_xml_format::text::escape_xml_text(&format!(
            "{object_type}.{name}.Form.{form_name}"
        ));
        if raw.ends_with("/>") {
            edits.push((
                range.end - 2..range.end,
                format!(">{value}</{}>", form_owner_qname(text, default)),
            ));
        } else {
            // XML permits `>` inside quoted attributes. Locate the content
            // boundary without mistaking such a byte for the tag delimiter.
            let mut quote = None;
            let open_end = raw
                .bytes()
                .position(|byte| match (quote, byte) {
                    (Some(active), byte) if active == byte => {
                        quote = None;
                        false
                    }
                    (Some(_), _) => false,
                    (None, b'\'' | b'"') => {
                        quote = Some(byte);
                        false
                    }
                    (None, b'>') => true,
                    _ => false,
                })
                .expect("parsed default element has an opening tag end")
                + 1;
            let close_start = raw
                .rfind("</")
                .expect("parsed expanded default has a closing tag");
            if raw[open_end..close_start]
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            {
                edits.push((range.start + open_end..range.start + close_start, value));
            }
        }
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
    let mut updated = text.to_owned();
    for (range, replacement) in edits {
        updated.replace_range(range, &replacement);
    }
    Ok(updated)
}
pub fn stage_form_add(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    forms: &[(String, String)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    use crate::codec::{
        form_add_content_xml, form_add_metadata_xml, form_add_module_bsl, form_default_property,
    };
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let (object_type, object_name) =
        crate::codec::detect_form_add_object(&owner_source).map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
        })?;
    let mut owner_text = owner_source.clone();
    let format_version = authority.expected_format();
    let forms_dir = owner_relative.with_extension("").join("Forms");
    let mut touched = Vec::new();
    for (index, (form_name, purpose)) in forms.iter().enumerate() {
        let name_path = format!("ops[{op_index}].args.items[{index}].name");
        if !unica_metadata::identifier::is_1c_identifier(form_name) {
            return Err(bad(
                op_index,
                &format!("items[{index}].name"),
                format!("`{form_name}` is not a valid 1C identifier"),
            ));
        }
        crate::codec::validate_form_purpose(&object_type, purpose)
            .map_err(|message| bad(op_index, &format!("items[{index}].type"), message))?;
        owner_text = form_add_owner_image(
            &owner_text,
            owner,
            (&object_type, &object_name),
            form_name,
            form_default_property(&object_type, purpose),
            &at_path,
            &name_path,
        )?;
        let uuid = unica_source_designer::registration::seeded_uuid(&format!(
            "{}\0{}.Form.{form_name}",
            authority.source_set_name(),
            owner.as_str()
        ));
        let lang = unica_source_designer::language::staged_text_language(
            staged,
            authority.source_kind(),
            op_index,
        )?;
        let descriptor = form_add_metadata_xml(
            form_name,
            form_name,
            &object_type,
            format_version,
            &uuid,
            &lang,
        );
        let content = form_add_content_xml(&object_type, &object_name, purpose, format_version)
            .map_err(|message| bad(op_index, &format!("items[{index}].type"), message))?;
        let descriptor_relative = forms_dir.join(format!("{form_name}.xml"));
        let content_relative = forms_dir.join(form_name).join("Ext/Form.xml");
        let module_relative = forms_dir.join(form_name).join("Ext/Form/Module.bsl");
        unica_source_designer::registration::stage_new_file(
            staged,
            &descriptor_relative,
            descriptor.as_bytes(),
            &name_path,
        )?;
        unica_source_designer::registration::stage_new_file(
            staged,
            &content_relative,
            content.as_bytes(),
            &name_path,
        )?;
        unica_source_designer::registration::stage_new_file(
            staged,
            &module_relative,
            form_add_module_bsl().as_bytes(),
            &name_path,
        )?;
        touched.push(descriptor_relative);
        touched.push(content_relative);
        touched.push(module_relative);
    }
    let mut owner_postimage = Vec::with_capacity(UTF8_BOM.len() + owner_text.len());
    if owner_preimage.starts_with(UTF8_BOM) {
        owner_postimage.extend_from_slice(UTF8_BOM);
    }
    owner_postimage.extend_from_slice(owner_text.as_bytes());
    if owner_postimage != owner_preimage {
        staged
            .replace(&owner_relative, &owner_preimage, owner_postimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path))?;
        touched.push(owner_relative);
    }
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(DomainEventKind::FormChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}
pub fn stage_form_set(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    defaults: &[(String, String)],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let mut owner_text = owner_source.clone();
    for (property, form_name) in defaults {
        if form_name.is_empty() {
            if !owner_text.contains(&format!("<{property}>"))
                && !owner_text.contains(&format!("<{property}/>"))
            {
                return Err(bad(
                    op_index,
                    "values",
                    format!("`{}` has no `{property}` slot", owner.as_str()),
                ));
            }
            owner_text = clear_form_default_property(&owner_text, property);
            continue;
        }
        if !owner_text.contains(&format!("<Form>{form_name}</Form>")) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                format!("form `{form_name}` is not a form of `{}`", owner.as_str()),
            )
            .at_path(format!("ops[{op_index}].args.values")));
        }
        if !owner_text.contains(&format!("<{property}>"))
            && !owner_text.contains(&format!("<{property}/>"))
        {
            return Err(bad(
                op_index,
                "values",
                format!("`{}` has no `{property}` slot", owner.as_str()),
            ));
        }
        let (updated, _) = crate::codec::replace_form_default_property(
            &owner_text,
            property,
            &format!("{}.Form.{form_name}", owner.as_str()),
            true,
        );
        owner_text = updated;
    }
    let owner_postimage = with_bom(
        &unica_xml_format::text::lxml_tree_serialized_text_like_source_preserving_final_newline(
            &owner_text,
            &owner_source,
        ),
    );
    if owner_postimage == owner_preimage {
        return Ok(());
    }
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        owner_relative,
        DomainEvent::new(DomainEventKind::MetadataChanged, owner.as_str().to_string()),
        op_index,
    ));
    Ok(())
}
pub fn stage_form_remove(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    form: &str,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    unica_source_designer::registration::require_untouched_staged_state(
        staged,
        "form.remove",
        &at_path,
    )?;
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let owner_preimage = read_required(
        staged,
        &owner_relative,
        "the form owner descriptor",
        &at_path,
    )?;
    let owner_source = utf8_body(&owner_preimage, "the form owner descriptor", &at_path)?;
    let forms_dir = owner_relative.with_extension("").join("Forms");
    let descriptor_relative = forms_dir.join(format!("{form}.xml"));
    let descriptor_preimage = read_required(
        staged,
        &descriptor_relative,
        "the form descriptor",
        &at_path,
    )?;
    let mut touched = Vec::new();
    let payload_relative = forms_dir.join(form);
    let files = staged
        .enumerate_tree(&payload_relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    for relative in files {
        let preimage = read_required(staged, &relative, "a form payload file", &at_path)?;
        staged
            .remove(&relative, &preimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
        touched.push(relative);
    }
    staged
        .remove(&descriptor_relative, &descriptor_preimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    staged
        .remove_owned_directories(&payload_relative, &descriptor_relative, &forms_dir)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?;
    touched.push(descriptor_relative);
    let (deregistered, removed) =
        unica_source_designer::registration::remove_metadata_child_text_with_flag(
            &owner_source,
            "Form",
            form,
        );
    if !removed {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("`{}` does not register form `{form}`", owner.as_str()),
        )
        .at_path(at_path));
    }
    let owner_text =
        clear_default_form_slots(&deregistered, &format!("{}.Form.{form}", owner.as_str()));
    let owner_postimage = unica_source_designer::registration::preserve_descriptor_image(
        &owner_preimage,
        &owner_source,
        &owner_text,
    );
    staged
        .replace(&owner_relative, &owner_preimage, owner_postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    touched.push(owner_relative);
    provisional.push(ProvisionalApplyEffect::spanning(
        touched,
        DomainEvent::new(
            DomainEventKind::FormChanged,
            format!("{}.Form.{form}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}
fn column_type_error(
    error: crate::column_type::ColumnTypeError,
    op_index: usize,
) -> ApplyPlanError {
    match error {
        crate::column_type::ColumnTypeError::Missing(message) => {
            ApplyPlanError::new(ApplyPlanErrorKind::NotFound, message)
                .at_path(format!("ops[{op_index}].args.at"))
        }
        crate::column_type::ColumnTypeError::Invalid(message) => {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                .at_path(format!("ops[{op_index}].args.values.type"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn stage_form_edit(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    owner: &MetadataAddress,
    form: &str,
    subject: &FormEditSubject,
    definitions: &[FormEditStep],
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
    reader: &dyn unica_source_contract::discovery::ProjectDiscoveryRead,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let owner_relative = owner_descriptor_relative(owner, authority, op_index)?;
    let relative = owner_relative
        .with_extension("")
        .join("Forms")
        .join(form)
        .join("Ext/Form.xml");
    let preimage = read_required(staged, &relative, "the form (Ext/Form.xml)", &at_path)?;
    let mut xml_text = utf8_body(&preimage, "the form", &at_path)?;
    let root_start = {
        let document = roxmltree::Document::parse(&xml_text).map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!("the form is not well-formed XML: {error}"),
            )
            .at_path(at_path.clone())
        })?;
        let root = document.root_element();
        crate::root::require_form_root(root).map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error).at_path(at_path.clone())
        })?;
        root.range().start
    };
    check_form_edit_subject(&xml_text, subject, definitions, op_index)?;
    let absolute = authority.source_root().join(&relative);
    let args_path = format!("ops[{op_index}].args");
    // Plain-string titles, tooltips and input hints are written in the
    // default language of the source set (#1257). It is read only when a
    // definition writes such text: the definition is first applied without a
    // language and, if it needs one, applied again from the same image.
    let mut lang: Option<String> = None;
    for step in definitions {
        let definition = match step {
            FormEditStep::Definition(definition) => definition,
            FormEditStep::ColumnType {
                attribute,
                column,
                type_name,
            } => {
                crate::column_type::apply(&mut xml_text, attribute, column, type_name)
                    .map_err(|error| column_type_error(error, op_index))?;
                continue;
            }
            FormEditStep::Move { request, location } => {
                xml_text = crate::form_move::move_item(&xml_text, request).map_err(|error| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(
                        if error.field == "name" {
                            at_path.clone()
                        } else {
                            format!("{location}.{}", error.field)
                        },
                    )
                })?;
                continue;
            }
        };
        let apply = |xml_text: &mut String, lang: Option<&str>| {
            crate::codec::form_edit_apply_definition(
                xml_text, definition, &absolute, root_start, lang,
            )
        };
        let before = xml_text.clone();
        let result = match apply(&mut xml_text, lang.as_deref()) {
            Err(message)
                if lang.is_none() && message == crate::codec::FORM_TEXT_LANGUAGE_REQUIRED =>
            {
                xml_text = before;
                let resolved = unica_source_designer::language::staged_text_language(
                    staged,
                    authority.source_kind(),
                    op_index,
                )?;
                apply(&mut xml_text, Some(&resolved)).map(|_| lang = Some(resolved))
            }
            result => result.map(|_| ()),
        };
        result.map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(args_path.clone())
        })?;
    }
    // Перенос сохраняет исходное состояние BOM. Остальные определения
    // используют прежнюю нормализацию писателя формы.
    let is_move = definitions
        .iter()
        .any(|step| matches!(step, FormEditStep::Move { .. }));
    let preserves_bytes = is_move
        || definitions
            .iter()
            .any(|step| matches!(step, FormEditStep::ColumnType { .. }));
    let postimage = if preserves_bytes && !preimage.starts_with(UTF8_BOM) {
        xml_text.as_bytes().to_vec()
    } else {
        with_bom(&xml_text)
    };
    if !is_move && postimage == preimage {
        return Ok(());
    }
    let validation = crate::codec::validate_form_image(
        &Map::new(),
        reader,
        &absolute,
        &xml_text,
        crate::codec::HandlerSignatures::Check,
    );
    crate::codec::form_validation_require_valid(validation).map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::Postcondition, message).at_path(args_path)
    })?;
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::FormChanged,
            format!("{}.Form.{form}", owner.as_str()),
        ),
        op_index,
    ));
    Ok(())
}
#[derive(Debug, Clone)]
pub enum FormPlan {
    /// `form.add` on the owner: managed forms from the platform shapes.
    FormAdd {
        owner: MetadataAddress,
        forms: Vec<(String, String)>,
    },
    /// `form.set`: default-form slots of the owner.
    FormSet {
        owner: MetadataAddress,
        defaults: Vec<(String, String)>,
    },
    FormRemove {
        owner: MetadataAddress,
        form: String,
    },
    /// Definition-driven edits of one form's Form.xml.
    FormEdit {
        owner: MetadataAddress,
        form: String,
        subject: FormEditSubject,
        definitions: Vec<FormEditStep>,
    },
}

pub fn stage_form_plan(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    plan: &FormPlan,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
    reader: &dyn unica_source_contract::discovery::ProjectDiscoveryRead,
) -> Result<(), ApplyPlanError> {
    match plan {
        FormPlan::FormAdd { owner, forms } => {
            stage_form_add(staged, authority, owner, forms, op_index, provisional)?;
        }
        FormPlan::FormSet { owner, defaults } => {
            stage_form_set(staged, authority, owner, defaults, op_index, provisional)?;
        }
        FormPlan::FormRemove { owner, form } => {
            stage_form_remove(staged, authority, owner, form, op_index, provisional)?;
        }
        FormPlan::FormEdit {
            owner,
            form,
            subject,
            definitions,
        } => {
            stage_form_edit(
                staged,
                authority,
                owner,
                form,
                subject,
                definitions,
                op_index,
                provisional,
                reader,
            )?;
        }
    }
    Ok(())
}

const FORM_ADDRESS_SHAPE: &str =
    "form operations address `Owner.Name`, `Owner.Name.Form.F`, `Owner.Name.Form.F.Item.A[.Item.B…]` or `….Event.E` of the form or its item";

const FORM_OWNER_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub fn minimal_typed_form_content(_object_kind: &str, _object_name: &str) -> String {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
        "\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n",
        "</Form>"
    )
    .to_string()
}

#[cfg(test)]
#[path = "../tests/platform/dump_parity.rs"]
mod dump_parity_tests;

/// #2144: the form slots of each owner kind are those an 8.3.27 export
/// writes in its properties.
#[cfg(test)]
mod form_slot_kind_tests {
    #[test]
    fn form_slots_of_each_kind_match_an_8_3_27_export() {
        // The exports are compiled in: this component reads no files itself.
        macro_rules! export {
            ($path:literal) => {
                (
                    $path,
                    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../", $path)),
                )
            };
        }
        for (kind, (fixture, text)) in [
            ("Catalog", export!("crates/unica-forms-designer/tests/platform/dump_parity/Catalog.before.xml")),
            ("Document", export!("crates/unica-metadata-designer/tests/platform/dump_parity/Document.before.xml")),
            ("ChartOfAccounts", export!("crates/unica-metadata-designer/tests/platform/dump_parity/ChartsOfAccounts.before.xml")),
            ("ChartOfCalculationTypes", export!("crates/unica-metadata-designer/tests/platform/dump_parity/ChartsOfCalculationTypes.before.xml")),
            ("ChartOfCharacteristicTypes", export!("crates/unica-metadata-designer/tests/platform/dump_parity/ChartsOfCharacteristicTypes.before.xml")),
            ("Constant", export!("tests/fixtures/platform_8_3_27/meta_info_surface/constant/Constants/MainCurrency.xml")),
            ("Enum", export!("tests/fixtures/platform_8_3_27/meta_info_surface/enum/Enums/PresentationEnum.xml")),
            ("InformationRegister", export!("tests/fixtures/platform_8_3_27/meta_info_surface/register/InformationRegisters/CommandInterfaceRegister.xml")),
            ("Report", export!("tests/fixtures/platform_8_3_27/template-new-xml/seed/Reports/CorpusReport.xml")),
            ("ExternalDataProcessor", export!("tests/fixtures/platform_8_3_27/external_artifacts/epf/SyntheticProcessor.xml")),
            ("ExternalReport", export!("tests/fixtures/platform_8_3_27/external_artifacts/erf/SyntheticReport.xml")),
        ] {
            let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
            let owner = document.root_element().first_element_child().unwrap();
            assert_eq!(owner.tag_name().name(), kind, "{fixture}");
            let properties = owner
                .children()
                .find(|node| node.has_tag_name((super::FORM_OWNER_MD_NS, "Properties")))
                .unwrap();
            let exported = properties
                .children()
                .filter(|node| node.is_element())
                .map(|node| node.tag_name().name())
                .filter(|name| {
                    unica_forms::contracts::FORM_SLOTS
                        .iter()
                        .any(|(_, tag)| tag == name)
                })
                .collect::<std::collections::BTreeSet<_>>();
            let declared = unica_forms::contracts::form_slots_of(kind)
                .unwrap()
                .iter()
                .map(|key| super::default_form_property(key).unwrap())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(declared, exported, "{kind}");
        }
    }
}

#[cfg(test)]
mod attribute_type_tests {
    use super::form_attribute_type;

    #[test]
    fn bare_primitive_types_take_the_qualifiers_8_3_27_exports() {
        for (input, expected) in [
            ("String", "string"),
            ("Строка", "string"),
            ("Number", "decimal"),
            ("Date", "dateTime"),
            ("Boolean", "boolean"),
            ("date", "date"),
            ("decimal", "decimal"),
            ("string(10)", "string(10)"),
            ("CatalogRef.Товары", "CatalogRef.Товары"),
            ("String | Number", "string | decimal"),
            ("String+Date", "string | dateTime"),
        ] {
            assert_eq!(form_attribute_type(input), expected, "{input}");
        }
    }
}

#[cfg(test)]
mod column_type_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn column_type_properties_address_an_existing_column() {
        let args = json!({"at":"main:Catalog.Items.Form.Main.Attribute.Rows.Column.Value", "values":{"type":"DefinedType.Existing | CatalogRef.Additional"}});
        let result = parse_form_operation(
            FormOperation::PropsSet,
            args.as_object().unwrap(),
            0,
            "main",
        );
        assert!(result.is_ok(), "{result:?}");
    }
    #[test]
    fn column_type_rejects_other_operations_and_extra_values() {
        let at = "main:Catalog.Items.Form.Main.Attribute.Rows.Column.Value";
        for values in [
            json!({}),
            json!({"type":true}),
            json!({"type":"Boolean", "Title":"lost"}),
            json!({"type":""}),
            json!({"type":"Boolean | Boolean"}),
        ] {
            let args = json!({"at":at,"values":values});
            assert!(
                parse_form_operation(
                    FormOperation::PropsSet,
                    args.as_object().unwrap(),
                    0,
                    "main"
                )
                .is_err(),
                "{args}"
            );
        }
        for op in [
            FormOperation::Remove,
            FormOperation::ItemAdd,
            FormOperation::AttributeAdd,
            FormOperation::EventBind,
        ] {
            let args = json!({"at":at,"values":{"type":"Boolean"}});
            assert!(parse_form_operation(op, args.as_object().unwrap(), 0, "main").is_err());
        }
        let invalid_owner = json!({"at":"main:DefinedType.Existing.Form.Main.Attribute.Rows.Column.Value","values":{"Title":"wrong profile"}});
        assert!(parse_form_operation(
            FormOperation::PropsSet,
            invalid_owner.as_object().unwrap(),
            0,
            "main"
        )
        .is_err());
        for at in [
            "main:Catalog.Items.Form.Main.Attribute.Rows",
            "main:Catalog.Items.Form.Main.Attribute.Rows.Column",
            "main:Catalog.Items.Form.Main.Item.Value",
        ] {
            let args = json!({"at":at,"values":{"type":"Boolean"}});
            assert!(parse_form_operation(
                FormOperation::PropsSet,
                args.as_object().unwrap(),
                0,
                "main"
            )
            .is_err());
        }
    }
}
