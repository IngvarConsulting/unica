//! Контракты семейства Form (#1447): каждая операция публикует ровно те
//! ключи, которые пишет её писатель. Ключ, которого писатель не пишет,
//! отказывает `bad_value` до записи, а не теряется молча.

use crate::operations::FormOperation;
use serde_json::{json, Map, Value};
use unica_metadata::event::FormCallType;
use unica_operation_contract::{object, text, OperationContract};

const FORM_HELP: &str = "skills/form-edit/SKILL.md";

/// Вид элемента, который создаёт `item.add`: имя вида в `type`, его
/// синонимы и ключи, которые генератор элемента (`form::emit_form_*`)
/// записывает в XML этого вида.
pub struct FormElementKind {
    pub kind: &'static str,
    pub aliases: &'static [&'static str],
    pub keys: &'static [&'static str],
}

/// Флаги, которые пишет `emit_form_common_flags` у каждого вида.
#[cfg(test)]
const COMMON_FLAGS: [&str; 6] = [
    "visible",
    "hidden",
    "userVisible",
    "enabled",
    "disabled",
    "readOnly",
];

macro_rules! with_common_flags {
    ($($key:literal),* $(,)?) => {
        &[
            "visible", "hidden", "userVisible", "enabled", "disabled", "readOnly",
            $($key),*
        ]
    };
}

/// Ключи динамического списка у таблицы: генератор пишет их блоком только
/// при `_dynList: true`.
pub const FORM_DYNAMIC_LIST_KEYS: &[&str] = &[
    "autoRefresh",
    "autoRefreshPeriod",
    "choiceFoldersAndItems",
    "restoreCurrentRow",
    "showRoot",
    "allowRootChoice",
    "updateOnDataChange",
    "allowGettingCurrentRowURL",
];

pub const FORM_ELEMENT_KINDS: &[FormElementKind] = &[
    FormElementKind {
        kind: "InputField",
        aliases: &["InputField", "Input", "Field"],
        keys: with_common_flags![
            "path",
            "title",
            "tooltip",
            "tooltipRepresentation",
            "titleLocation",
            "multiLine",
            "passwordMode",
            "choiceButton",
            "clearButton",
            "spinButton",
            "dropListButton",
            "markIncomplete",
            "skipOnInput",
            "showInHeader",
            "headerHorizontalAlign",
            "autoMaxWidth",
            "autoMaxHeight",
            "footerDataPath",
            "width",
            "height",
            "multipleValueDataPath",
            "multipleValuePictureDataPath",
            "multipleValuePresentDataPath",
            "horizontalStretch",
            "verticalStretch",
            "horizontalAlign",
            "inputHint",
        ],
    },
    FormElementKind {
        kind: "CheckBox",
        aliases: &["CheckBox", "CheckBoxField"],
        keys: with_common_flags![
            "path",
            "title",
            "tooltip",
            "tooltipRepresentation",
            "titleLocation",
            "checkBoxType",
            "footerDataPath",
        ],
    },
    FormElementKind {
        kind: "LabelField",
        aliases: &["LabelField"],
        keys: with_common_flags!["path", "tooltip", "tooltipRepresentation", "footerDataPath"],
    },
    FormElementKind {
        kind: "HTMLDocumentField",
        aliases: &["HTMLDocumentField"],
        keys: with_common_flags![
            "path",
            "title",
            "tooltip",
            "tooltipRepresentation",
            "skipOnInput",
            "titleLocation",
            "width",
            "height",
            "autoMaxWidth",
            "autoMaxHeight",
        ],
    },
    FormElementKind {
        kind: "TextDocumentField",
        aliases: &["TextDocumentField"],
        keys: with_common_flags![
            "path",
            "title",
            "tooltip",
            "tooltipRepresentation",
            "titleLocation",
            "width",
            "height",
            "autoMaxWidth",
            "autoMaxHeight",
        ],
    },
    FormElementKind {
        kind: "Label",
        aliases: &["Label", "LabelDecoration"],
        keys: with_common_flags![
            "title",
            "formatted",
            "hyperlink",
            "width",
            "height",
            "autoMaxWidth",
            "autoMaxHeight",
            "tooltipRepresentation",
        ],
    },
    FormElementKind {
        kind: "Button",
        aliases: &["Button"],
        keys: with_common_flags![
            "representation",
            "command",
            "commandName",
            "stdCommand",
            "title",
            "tooltip",
            "tooltipRepresentation",
            "defaultButton",
            "picture",
            "locationInCommandBar",
            "backColor",
            "font",
        ],
    },
    FormElementKind {
        kind: "Group",
        aliases: &["Group", "UsualGroup"],
        keys: with_common_flags![
            "title",
            "orientation",
            "behavior",
            "collapsed",
            "representation",
            "titleDataPath",
            "currentRowUse",
            "showTitle",
            "showLeftMargin",
        ],
    },
    FormElementKind {
        kind: "Pages",
        aliases: &["Pages"],
        keys: with_common_flags!["title", "pagesRepresentation", "currentRowUse"],
    },
    FormElementKind {
        kind: "Page",
        aliases: &["Page"],
        keys: with_common_flags!["title", "orientation", "showTitle", "titleDataPath"],
    },
    FormElementKind {
        kind: "Table",
        aliases: &["Table"],
        keys: with_common_flags![
            "representation",
            "autoInsertNewRow",
            "path",
            "commandBarLocation",
            "initialTreeView",
            "enableDrag",
            "rowPictureDataPath",
            "tableAutofill",
            "_dynList",
            "autoRefresh",
            "autoRefreshPeriod",
            "choiceFoldersAndItems",
            "restoreCurrentRow",
            "showRoot",
            "allowRootChoice",
            "updateOnDataChange",
            "allowGettingCurrentRowURL",
        ],
    },
    FormElementKind {
        kind: "CommandBar",
        aliases: &["CommandBar"],
        keys: with_common_flags!["commandSource", "autofill"],
    },
];

/// Ключи вложенного дерева, companion-содержимого и событий, которые
/// `item.add` больше не принимает (Р2, #1447): элемент добавляется один,
/// вложенный — отдельным `item.add` на адресе контейнера, обработчик —
/// `event.bind` на адресе события.
pub const FORM_ELEMENT_NESTED_KEYS: &[&str] = &[
    "children",
    "columns",
    "elements",
    "commandBar",
    "autoCmdBar",
    "autoCommandBar",
    "contextMenu",
    "extendedTooltip",
];
pub const FORM_ELEMENT_EVENT_KEYS: &[&str] = &["on", "handlers"];

/// Перечисления 8.3.27, которыми ограничены значения ключей элемента.
/// Тест писателя сверяет их с его собственным справочником перечислений.
pub fn form_element_enum(kind: &str, key: &str) -> Option<&'static [&'static str]> {
    Some(match (kind, key) {
        (_, "titleLocation") => &["None", "Auto", "Left", "Top", "Right", "Bottom"],
        (_, "checkBoxType") => &["Auto", "CheckBox", "Tumbler", "Switcher"],
        (_, "headerHorizontalAlign" | "horizontalAlign") => &["Left", "Center", "Right", "Auto"],
        (_, "tooltipRepresentation") => &[
            "Auto",
            "None",
            "Button",
            "Balloon",
            "ShowAuto",
            "ShowTop",
            "ShowBottom",
            "ShowLeft",
            "ShowRight",
        ],
        (_, "orientation") => &[
            "Horizontal",
            "Vertical",
            "HorizontalIfPossible",
            "AlwaysHorizontal",
        ],
        (_, "behavior") => &["Usual", "Collapsible", "PopUp", "Auto"],
        ("Group", "representation") => &[
            "None",
            "StrongSeparation",
            "WeakSeparation",
            "NormalSeparation",
            "GroupBox",
            "Line",
            "Margin",
        ],
        ("Table", "representation") => &["List", "HierarchicalList", "Tree"],
        ("Button", "representation") => &["Text", "Picture", "PictureAndText", "Auto"],
        (_, "currentRowUse") => &["Use", "DontUse", "Auto"],
        (_, "pagesRepresentation") => &[
            "None",
            "TabsOnTop",
            "TabsOnBottom",
            "TabsOnLeftHorizontal",
            "TabsOnRightHorizontal",
            "Swipe",
            "Auto",
        ],
        (_, "commandBarLocation") => &["None", "Auto", "Top", "Bottom"],
        (_, "initialTreeView") => &["NoExpand", "ExpandTopLevel", "ExpandAllLevels"],
        (_, "choiceFoldersAndItems") => &["Items", "Folders", "FoldersAndItems"],
        (_, "updateOnDataChange") => &["Auto", "DontUpdate"],
        (_, "locationInCommandBar") => &[
            "Auto",
            "InAdditionalSubmenu",
            "InCommandBar",
            "InCommandBarAndInAdditionalSubmenu",
        ],
        _ => return None,
    })
}

fn language_map() -> Value {
    json!({"type":"object","minProperties":1,"additionalProperties":{"type":"string"},
        "description":"Text by language code, e.g. {\"ru\": \"…\", \"en\": \"…\"}."})
}

fn element_key_schema(kind: &str, key: &str) -> Value {
    if let Some(values) = form_element_enum(kind, key) {
        return json!({"type":"string","enum":values});
    }
    match key {
        "title" if kind == "Label" => json!({"anyOf":[
            {"type":"string"},
            language_map(),
            object(&[("text", json!({"anyOf":[{"type":"string"}, language_map()]})),
                ("formatted", json!({"type":"boolean"}))], &["text"]),
        ],
        "description":"Decoration text: a string in the configuration language, a language map, or {text, formatted}. Without a title the decoration shows its name."}),
        "title" => json!({"type":"string",
            "description":"Title in the configuration language."}),
        "inputHint" => {
            json!({"type":"string","description":"Input hint in the configuration language."})
        }
        "tooltip" => json!({"anyOf":[{"type":"string"}, language_map()],
            "description":"Tooltip: a string in the configuration language or a language map."}),
        "width" | "height" => json!({"type":"integer","minimum":0,
            "description":"Integer in 0..=4294967295."}),
        "autoRefreshPeriod" => json!({"type":"integer","minimum":0}),
        "font" => json!({"anyOf":[
            text(true),
            object(&[("ref",text(true)),("faceName",text(true)),("height",json!({"type":"number"})),
                ("bold",json!({"type":"boolean"})),("italic",json!({"type":"boolean"})),
                ("underline",json!({"type":"boolean"})),("strikeout",json!({"type":"boolean"})),
                ("kind",text(true)),("scale",json!({"type":"number"}))], &[]),
        ],"description":"A style font reference, or font attributes."}),
        "path"
        | "footerDataPath"
        | "titleDataPath"
        | "rowPictureDataPath"
        | "multipleValueDataPath"
        | "multipleValuePictureDataPath"
        | "multipleValuePresentDataPath" => json!({"type":"string","pattern":r"\S",
            "description":"Data path from the form attributes, e.g. Объект.Наименование."}),
        "command" => json!({"type":"string","pattern":r"\S",
            "description":"A form command name: written as Form.Command.<name>."}),
        "commandName" => json!({"type":"string","pattern":r"\S",
            "description":"A full command reference, e.g. CommonCommand.X."}),
        "stdCommand" => json!({"type":"string","pattern":r"\S",
            "description":"A standard command: Close → Form.StandardCommand.Close; Items.Add → Form.Item.Items.StandardCommand.Add."}),
        "picture" => json!({"type":"string","pattern":r"\S",
            "description":"Picture reference, e.g. StdPicture.Refresh."}),
        "backColor" => json!({"type":"string","pattern":r"\S",
            "description":"Color as the platform writes it, e.g. style:ButtonBackColor or web:Red."}),
        "commandSource" => json!({"type":"string","pattern":r"\S"}),
        "_dynList" => json!({"type":"boolean",
            "description":"The table shows a DynamicList attribute: writes the dynamic-list block; the dynamic-list keys need it."}),
        key if only_true_is_written(kind, key) => json!({"type":"boolean","enum":[true],
            "description":"Only true is written; an explicit false is not written by this operation yet."}),
        _ => json!({"type":"boolean"}),
    }
}

/// Флаги, которые писатель вида пишет только со значением `true`, а платформа
/// отличает явный `false` от отсутствия тега (выгрузка 8.3.27): `false`
/// потерялся бы молча, поэтому контракт его не принимает.
fn only_true_is_written(kind: &str, key: &str) -> bool {
    match kind {
        "InputField" => matches!(
            key,
            "clearButton"
                | "spinButton"
                | "dropListButton"
                | "markIncomplete"
                | "skipOnInput"
                | "horizontalStretch"
                | "verticalStretch"
        ),
        "CommandBar" => key == "autofill",
        _ => false,
    }
}

fn element_item_schema(kinds: &[&str]) -> Value {
    let variants: Vec<Value> = FORM_ELEMENT_KINDS
        .iter()
        .filter(|kind| kinds.is_empty() || kinds.contains(&kind.kind))
        .map(|kind| {
            let mut kind_schema = json!({"type":"string","enum":kind.aliases});
            if kind.kind == "InputField" {
                kind_schema["default"] = json!("InputField");
            }
            let mut fields = vec![
                (
                    "name",
                    json!({"type":"string","pattern":r"\S",
                        "description":"New element name, unused in the form."}),
                ),
                ("type", kind_schema),
                (
                    "after",
                    json!({"type":"string","pattern":r"\S",
                        "description":"Sibling in the same container the element follows."}),
                ),
            ];
            fields.extend(
                kind.keys
                    .iter()
                    .map(|key| (*key, element_key_schema(kind.kind, key))),
            );
            // Без `type` элемент — поле ввода: другие виды требуют его явно,
            // и стандартный валидатор схемы выбирает тот же вариант.
            let required: &[&str] = match kind.kind {
                "InputField" => &["name"],
                "TextDocumentField" => &["name", "type", "path"],
                _ => &["name", "type"],
            };
            object(&fields, required)
        })
        .collect();
    json!({"type":"object","x-unica-discriminator":"type","anyOf":variants})
}

fn attribute_item_schema() -> Value {
    object(
        &[
            (
                "name",
                json!({"type":"string","pattern":r"\S",
                    "description":"New attribute name, unused in the form."}),
            ),
            (
                "title",
                json!({"type":"string","description":"Title in the configuration language."}),
            ),
            (
                "type",
                json!({"type":"string","pattern":r"\S",
                    "description":"Type: String, Boolean, Number, Date, string(100), decimal(15,2), CatalogObject.X, ValueTable, DynamicList, or `A | B`. Without type the attribute has an empty type."}),
            ),
            ("main", json!({"type":"boolean"})),
            ("savedData", json!({"type":"boolean"})),
            (
                "fillChecking",
                json!({"type":"string","enum":["ShowError"],
                    "description":"Fill check, written as <FillCheck>ShowError</FillCheck>; omit it for no check."}),
            ),
            (
                "columns",
                json!({"type":"array","minItems":1,"items":object(&[
                    ("name",text(true)),
                    ("title",json!({"type":"string"})),
                    ("type",text(true)),
                ], &["name","type"]),
                "description":"Columns of a ValueTable or ValueTree attribute."}),
            ),
        ],
        &["name"],
    )
}

const FORM_SLOTS: [&str; 13] = [
    "defaultObjectForm",
    "defaultListForm",
    "defaultChoiceForm",
    "defaultFolderForm",
    "defaultFolderChoiceForm",
    "defaultRecordForm",
    "defaultForm",
    "defaultSettingsForm",
    "defaultVariantForm",
    "defaultSaveForm",
    "auxiliaryObjectForm",
    "auxiliaryListForm",
    "auxiliaryChoiceForm",
];

pub fn form(operation: FormOperation) -> OperationContract {
    let at = || ("at", text(true));
    let (schema, target, effect, notes, example_args) = match operation {
        FormOperation::Add => (
            object(&[at(), ("items", json!({"type":"array","minItems":1,"items":object(&[
                ("name",json!({"type":"string","pattern":r"\S","description":"New form name, unused by the owner."})),
                ("type",json!({"type":"string","default":"ObjectForm","enum":[
                    "ObjectForm","ListForm","ChoiceForm","RecordForm","RecordSetForm","SettingsForm","FolderForm","Generic"]})),
            ], &["name"])}))], &["items"]),
            "The form owner: <source-set>:<Kind>.<Name>. Read its Form branch before choosing a new name.",
            "Create an empty form with its descriptor, Form.xml and module, register it in the owner and fill the owner's empty default slot of its purpose. ListForm, ChoiceForm and RecordForm/RecordSetForm have their own purpose; ObjectForm, SettingsForm, FolderForm and Generic all create an object-purpose form and fill DefaultObjectForm (DefaultForm for a report or data processor): point other slots with form.set.",
            "Fill the new form with item.add, attribute.add and event.bind at its address after the plan is executed; within one apply call a later operation may already address it.",
            json!({"items":[{"name":"Sample","type":"ObjectForm"}]}),
        ),
        FormOperation::Set => {
            let slot = json!({"type":"string","pattern":r"\S",
                "description":"A form name of this owner."});
            let mut values = object(
                &FORM_SLOTS.iter().map(|slot_name| (*slot_name, slot.clone())).collect::<Vec<_>>(),
                &[],
            );
            values["minProperties"] = json!(1);
            (
                object(&[at(), ("values", values)], &["values"]),
                "The form owner: <source-set>:<Kind>.<Name>.",
                "Point the owner's default and auxiliary form slots at the named forms. Unlisted slots stay.",
                "Properties of a form itself (title, size, behaviour) are written by props.set at the form address.",
                json!({"values":{"defaultObjectForm":"Sample"}}),
            )
        }
        FormOperation::Remove => (
            object(&[at()], &[]),
            "The existing form: <source-set>:<Kind>.<Name>.Form.<Form>.",
            "Remove the form with its files and clear owner slots that pointed at it.",
            "The form is named by its address; the operation takes no other arguments.",
            json!({}),
        ),
        FormOperation::ItemAdd => (
            object(&[at(), ("items", json!({"type":"array","minItems":1,"items":element_item_schema(&[])}))], &["items"]),
            "The form <source-set>:<Kind>.<Name>.Form.<Form> or its container item ….Item.<Group>[.Item.<Nested>…] (group, pages, page, table, command bar).",
            "Add each listed element, without nested content, at the end of the addressed container or after its sibling `after`. Identifiers and companion items (ContextMenu, ExtendedTooltip, table additions) are written by the operation.",
            "Each item is one element; its keys depend on its type. A nested element is added by a separate item.add at the address of its container; a handler is bound with event.bind at ….Item.<Element>.Event.<Event>. children, columns, elements, commandBar, autoCmdBar, autoCommandBar, contextMenu, extendedTooltip, html, on and handlers are refused. A command bar takes only buttons. Cross-field rules are checked by the planner: dynamic-list keys of a table need _dynList: true; a button takes one of command, commandName, stdCommand.",
            json!({"items":[{"name":"Sample","type":"InputField"}]}),
        ),
        FormOperation::ItemRemove => (
            object(&[at()], &[]),
            "The existing item: <source-set>:<Kind>.<Name>.Form.<Form>.Item.<Element>[.Item.<Nested>…].",
            "Remove the item with its nested items and companions.",
            "The item is named by its address; the operation takes no other arguments.",
            json!({}),
        ),
        FormOperation::EventBind => (
            object(&[at(), ("values", object(&[
                ("handler",json!({"type":"string","pattern":r"\S","description":"Handler procedure in the form module."})),
                ("callType",json!({"type":"string","enum":FormCallType::ALL.map(FormCallType::as_str),
                    "description":"Only in an extension form."})),
            ], &["handler"]))], &["values"]),
            "An event of the form or its item: <source-set>:<Kind>.<Name>.Form.<Form>[.Item.<Element>…].Event.<Event>, as view lists it.",
            "Bind the handler to the addressed event. The module procedure is not written; event.implement writes both.",
            "The event and its item are named by the address.",
            json!({"values":{"handler":"Sample"}}),
        ),
        FormOperation::PropsSet => return form_props_set(),
        FormOperation::ItemMove => return form_move(),
        FormOperation::AttributeAdd => return form_attribute_add(),
        FormOperation::CommandAdd => return form_command_add(),
    };
    OperationContract {
        schema,
        target,
        effect,
        notes,
        example_args,
        help: FORM_HELP,
        format_validators: &[],
    }
}

/// `props.set` at a form: the root properties of its `Form.xml` (#1984).
pub fn form_props_set() -> OperationContract {
    use crate::root_properties::{writable_root_properties, RootValue};
    let mut values = Map::new();
    values.insert(
        "Title".to_string(),
        json!({"anyOf":[
            {"type":"string","description":"Text in the default language of the source set; an empty string removes the title."},
            {"type":"object","additionalProperties":{"type":"string"},"minProperties":1,
                "description":"Text by language code: {\"ru\": \"…\", \"en\": \"…\"}."}
        ]}),
    );
    for property in writable_root_properties() {
        let schema = match property.value {
            RootValue::Flag => json!({"type":"boolean"}),
            RootValue::Size => json!({"type":"integer","minimum":0,"maximum":u32::MAX}),
            RootValue::Choice(choices) => json!({"type":"string","enum":choices}),
            RootValue::Reference => continue,
        };
        values.insert(property.name.to_string(), schema);
    }
    let values = json!({"type":"object","additionalProperties":false,"minProperties":1,
        "properties":values});
    OperationContract {
        schema: object(&[("at", text(true)), ("values", values)], &["values"]),
        target: "The form itself: <source-set>:<Kind>.<Name>.Form.<Form>.",
        effect: "Set the supplied root properties of the form, in the order 1C writes them. A value equal to the platform default is not written, as 1C does not export it. Other properties and the form content stay.",
        notes: "Names are the ones view shows for the form. AutoTitle is not changed with Title: set AutoTitle false to show only the title. AutoTime, UsePostingMode and RepostOnWrite belong to a document form, UseForFoldersAndItems to a catalog form, report properties to a report form; on another form they are refused. ReportResult, DetailsData, VariantAppearance, CustomSettingsFolder, SettingsStorage and GroupList are not written by this operation.",
        example_args: json!({"values":{"Title":"Products","AutoTitle":false,"Width":60}}),
        help: FORM_HELP,
        format_validators: &[],
    }
}

pub fn form_attribute_add() -> OperationContract {
    let (schema, target, effect, notes, example_args) = (
        object(&[("at", text(true)), ("items", json!({"type":"array","minItems":1,"items":attribute_item_schema()}))], &["items"]),
        "The form itself: <source-set>:<Kind>.<Name>.Form.<Form>. Read its Attribute branch before choosing names.",
        "Add form attributes with the listed properties.",
        "View, edit, functional options, use-always fields, save, value type, dynamic-list settings and role-specific visibility are not written by this operation and are refused.",
        json!({"items":[{"name":"Sample","type":"String"}]}),
    );
    OperationContract {
        schema,
        target,
        effect,
        notes,
        example_args,
        help: FORM_HELP,
        format_validators: &[],
    }
}

pub fn form_command_add() -> OperationContract {
    let (schema, target, effect, notes, example_args) = (
        object(
            &[
                ("at", text(true)),
                (
                    "items",
                    json!({"type":"array","minItems":1,"items":object(&[
                        ("name",json!({"type":"string","pattern":r"\S",
                            "description":"New command name, unused in the form."})),
                        ("title",json!({"type":"string","pattern":r"\S",
                            "description":"Title in the configuration language."})),
                        ("tooltip",json!({"type":"string","pattern":r"\S",
                            "description":"Tooltip in the configuration language."})),
                        ("action",json!({"type":"string","pattern":r"\S",
                            "description":"Handler procedure in the form module; defaults to the command name."})),
                        ("shortcut",json!({"type":"string","pattern":r"\S",
                            "description":"Keyboard shortcut as the platform writes it, e.g. F5."})),
                        ("picture",json!({"type":"string","pattern":r"^(StdPicture|CommonPicture)\.[^.\s]+$",
                            "description":"Picture reference StdPicture.<Name> or CommonPicture.<Name>; written with LoadTransparent=true."})),
                        ("representation",json!({"type":"string","enum":["Auto","Text","Picture","TextPicture"],
                            "description":"Command DefaultRepresentation; the button set (PictureAndText, None, Compact) is not valid for a command."})),
                    ], &["name"])}),
                ),
            ],
            &["items"],
        ),
        "The form itself: <source-set>:<Kind>.<Name>.Form.<Form>. Read its Command branch before choosing names.",
        "Add form commands with the listed properties, written in the platform's FormCommand order. Place a button bound to the command with item.add.",
        "A property outside this schema is refused before any write; use, functionalOptions, currentRowUse, modifiesSavedData and the associated table are not written by this operation.",
        json!({"items":[{"name":"Refresh","title":"Refresh","tooltip":"Refresh the list","shortcut":"F5","picture":"StdPicture.Refresh","representation":"TextPicture"}]}),
    );
    OperationContract {
        schema,
        target,
        effect,
        notes,
        example_args,
        help: FORM_HELP,
        format_validators: &[],
    }
}

/// Контракт `item.add` на командной панели: она принимает только
/// кнопки, и контракт узла публикует один вид.
pub fn form_item_add_in_command_bar() -> OperationContract {
    let mut contract = form(FormOperation::ItemAdd);
    contract.schema["properties"]["items"]["items"] = element_item_schema(&["Button"]);
    contract.target = "A command bar: the form's ….Item.<Form>КоманднаяПанель, a table's ….Item.<Table>.Item.<Table>КоманднаяПанель, or a CommandBar item.";
    contract.example_args =
        json!({"items":[{"name":"Sample","type":"Button","stdCommand":"Close"}]});
    contract
}

pub fn form_move() -> OperationContract {
    OperationContract {
        schema: object(&[("at", text(true)), ("values", object(&[("into", json!({"anyOf":[text(true), {"type":"null"}]})), ("after", text(true))], &["into"]))], &["values"]),
        target: "The existing Item at its full current path: Owner.Name.Form.F.Item.A[.Item.B…]. The address alone selects the source.",
        effect: "Move an existing XML subtree, preserving IDs, properties, events and contained companions. Incompatible placements are refused without conversion.",
        notes: "Names are exact. into is required: use a container name, or null for the form root. Omit after for the end. after must be a direct child of the destination and cannot name the source. Exact placement is a no-op. A failure publishes nothing.",
        example_args: json!({"values":{"into":"Destination"}}),
        help: "skills/form-edit/SKILL.md",
        format_validators: &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_kind_lists_the_common_flags_once() {
        for kind in FORM_ELEMENT_KINDS {
            for flag in COMMON_FLAGS {
                assert_eq!(
                    kind.keys.iter().filter(|key| **key == flag).count(),
                    1,
                    "{} {flag}",
                    kind.kind
                );
            }
        }
    }

    /// Ключ, которого писатель вида не пишет, отказывает по точному пути,
    /// а не по общему «ни одна альтернатива не подошла» (#1447).
    #[test]
    fn item_add_refuses_a_key_of_another_kind_at_its_exact_path() {
        let contract = form(FormOperation::ItemAdd);
        for (item, path) in [
            (
                json!({"name":"A","type":"InputField","backColor":"web:Red"}),
                "args.items[0].backColor",
            ),
            (json!({"name":"A","font":"Normal"}), "args.items[0].font"),
            (
                json!({"name":"A","type":"Group","backColor":"web:Red"}),
                "args.items[0].backColor",
            ),
            (
                json!({"name":"A","type":"Table","title":"T"}),
                "args.items[0].title",
            ),
            (
                json!({"name":"A","type":"Spreadsheet"}),
                "args.items[0].type",
            ),
            (
                json!({"name":"A","userVisible":{"common":false}}),
                "args.items[0].userVisible",
            ),
            (
                json!({"name":"A","titleLocation":"top"}),
                "args.items[0].titleLocation",
            ),
            (
                json!({"name":"A","title":{"ru":"A"}}),
                "args.items[0].title",
            ),
            (
                json!({"name":"A","horizontalStretch":false}),
                "args.items[0].horizontalStretch",
            ),
            (
                json!({"name":"A","type":"CommandBar","autofill":false}),
                "args.items[0].autofill",
            ),
        ] {
            let error = contract
                .normalize(&json!({"items":[item.clone()]}), "args")
                .unwrap_err();
            assert_eq!(error.path, path, "{item}");
        }
        for item in [
            json!({"name":"A"}),
            json!({"name":"A","type":"Button","backColor":"web:Red","font":{"bold":true}}),
            json!({"name":"A","type":"Label","title":{"text":"A","formatted":true}}),
            json!({"name":"A","type":"Table","_dynList":true,"autoRefresh":true}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"items":[item.clone()]}), "args")
                    .is_ok(),
                "{item}"
            );
        }
    }

    #[test]
    fn text_document_field_requires_a_binding_path_in_the_closed_contract() {
        let contract = form(FormOperation::ItemAdd);
        let valid = json!({"name":"Protocol","type":"TextDocumentField","path":"ProtocolValue"});
        assert!(contract
            .normalize(&json!({"items":[valid]}), "args")
            .is_ok());
        for item in [
            json!({"name":"Protocol","type":"TextDocumentField"}),
            json!({"name":"Protocol","type":"TextDocumentField","path":""}),
        ] {
            let error = contract
                .normalize(&json!({"items":[item]}), "args")
                .unwrap_err();
            assert_eq!(error.path, "args.items[0].path");
        }
    }

    #[test]
    fn form_contracts_accept_their_examples() {
        for op in [
            FormOperation::Add,
            FormOperation::Set,
            FormOperation::Remove,
            FormOperation::ItemAdd,
            FormOperation::ItemRemove,
            FormOperation::ItemMove,
            FormOperation::AttributeAdd,
            FormOperation::CommandAdd,
            FormOperation::EventBind,
        ] {
            let contract = form(op);
            assert!(
                contract.normalize(&contract.example_args, "args").is_ok(),
                "{op:?}"
            );
        }
    }

    #[test]
    fn form_attribute_add_refuses_properties_it_does_not_write() {
        let contract = crate::operations::FormOperation::AttributeAdd.contract();
        for key in [
            "view",
            "edit",
            "functionalOptions",
            "useAlways",
            "save",
            "fillCheck",
            "valueType",
            "settings",
            "chart",
            "planner",
            "additionalColumns",
        ] {
            let error = contract
                .normalize(
                    &json!({"items":[{"name":"A","type":"String",key:true}]}),
                    "args",
                )
                .unwrap_err();
            assert_eq!(error.path, format!("args.items[0].{key}"));
        }
        assert!(contract
            .normalize(
                &json!({"items":[{"name":"A","fillChecking":"DontCheck"}]}),
                "args"
            )
            .is_err());
        assert!(contract
            .normalize(
                &json!({"items":[{"name":"A","type":"String","fillChecking":"ShowError"}]}),
                "args"
            )
            .is_ok());
    }
}
