//! The keys of the form description each compiled owner writes (#2970).
//!
//! The compiler reads a description key by key; a key it does not read, or a
//! value of a JSON type it does not read, used to vanish behind a successful
//! call. Every owner here lists exactly the keys its emitter writes, with the
//! JSON type it reads, and anything else is refused by name.

use serde_json::{Map, Value};

/// The JSON shape a description key is read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyType {
    /// Any value: a kind discriminator, or a key its emitter validates itself
    /// with a more specific refusal (`width`, `height`).
    Any,
    Str,
    Bool,
    /// An integer or a string holding one.
    Integer,
    /// A string or a language map.
    Text,
    /// A string or an object (a style reference or its parts).
    StrOrObject,
    Array,
    Object,
}

impl KeyType {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Any => true,
            Self::Str => value.is_string(),
            Self::Bool => value.is_boolean(),
            Self::Integer => {
                value.as_i64().is_some()
                    || value
                        .as_str()
                        .is_some_and(|s| s.trim().parse::<i64>().is_ok())
            }
            Self::Text | Self::StrOrObject => value.is_string() || value.is_object(),
            Self::Array => value.is_array(),
            Self::Object => value.is_object(),
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::Any => "any value",
            Self::Str => "a string",
            Self::Bool => "a boolean",
            Self::Integer => "a number",
            Self::Text => "a string or a language map",
            Self::StrOrObject => "a string or an object",
            Self::Array => "an array",
            Self::Object => "an object",
        }
    }
}

type Keys = &'static [(&'static str, KeyType)];

use KeyType::{Any, Array, Bool, Integer, Object, Str, StrOrObject, Text};

const NAME: Keys = &[("name", Str)];
const FLAGS: Keys = &[
    ("visible", Bool),
    ("hidden", Bool),
    ("userVisible", Bool),
    ("enabled", Bool),
    ("disabled", Bool),
    ("readOnly", Bool),
];
const EVENTS: Keys = &[("on", Array), ("handlers", Object)];
const TOOLTIP: Keys = &[("tooltip", Text), ("tooltipRepresentation", Str)];
const CHILDREN: Keys = &[("children", Array)];

const TABLE: Keys = &[
    ("table", Any),
    ("representation", Str),
    ("autoInsertNewRow", Bool),
    ("path", Str),
    ("commandBarLocation", Str),
    ("initialTreeView", Str),
    ("enableDrag", Bool),
    ("rowPictureDataPath", Str),
    ("_dynList", Bool),
    ("autoRefresh", Bool),
    ("autoRefreshPeriod", Integer),
    ("choiceFoldersAndItems", Str),
    ("restoreCurrentRow", Bool),
    ("showRoot", Bool),
    ("allowRootChoice", Bool),
    ("updateOnDataChange", Str),
    ("allowGettingCurrentRowURL", Bool),
    ("tableAutofill", Bool),
    ("columns", Array),
];
/// Table keys the compiler writes only for a dynamic list (`_dynList: true`).
pub(super) const TABLE_DYNAMIC_LIST_KEYS: [&str; 8] = [
    "autoRefresh",
    "autoRefreshPeriod",
    "choiceFoldersAndItems",
    "restoreCurrentRow",
    "showRoot",
    "allowRootChoice",
    "updateOnDataChange",
    "allowGettingCurrentRowURL",
];
const LABEL: Keys = &[
    ("label", Any),
    ("hyperlink", Bool),
    ("width", Any),
    ("height", Any),
    ("autoMaxWidth", Bool),
    ("autoMaxHeight", Bool),
    ("tooltipRepresentation", Str),
    ("title", Text),
    ("formatted", Bool),
];
const LABEL_FIELD: Keys = &[("labelField", Any), ("path", Str), ("footerDataPath", Str)];
const BUTTON: Keys = &[
    ("button", Any),
    ("type", Str),
    ("representation", Str),
    ("command", Str),
    ("commandName", Str),
    ("stdCommand", Str),
    ("title", Str),
    ("defaultButton", Bool),
    ("picture", Str),
    ("locationInCommandBar", Str),
    ("backColor", Str),
    ("font", StrOrObject),
];
const COMMAND_BAR: Keys = &[
    ("cmdBar", Any),
    ("commandBar", Any),
    ("commandSource", Str),
    ("autofill", Bool),
];
const AUTO_COMMAND_BAR: Keys = &[
    ("autoCmdBar", Any),
    ("autoCommandBar", Any),
    ("horizontalAlign", Str),
    ("autofill", Bool),
];
const PAGES: Keys = &[
    ("pages", Any),
    ("title", Str),
    ("pagesRepresentation", Str),
    ("currentRowUse", Str),
];
const PAGE: Keys = &[
    ("page", Any),
    ("title", Str),
    ("group", Str),
    ("showTitle", Bool),
    ("titleDataPath", Str),
];
const GROUP: Keys = &[
    ("group", Any),
    ("title", Str),
    ("behavior", Str),
    ("collapsed", Bool),
    ("representation", Str),
    ("titleDataPath", Str),
    ("currentRowUse", Str),
    ("showTitle", Bool),
    ("showLeftMargin", Bool),
];
const CHECK: Keys = &[
    ("check", Any),
    ("path", Str),
    ("title", Str),
    ("titleLocation", Str),
    ("checkBoxType", Str),
    ("footerDataPath", Str),
];
const INPUT: Keys = &[
    ("input", Any),
    ("path", Str),
    ("title", Str),
    ("titleLocation", Str),
    ("multiLine", Bool),
    ("passwordMode", Bool),
    ("choiceButton", Bool),
    ("clearButton", Bool),
    ("spinButton", Bool),
    ("dropListButton", Bool),
    ("markIncomplete", Bool),
    ("skipOnInput", Bool),
    ("showInHeader", Bool),
    ("headerHorizontalAlign", Str),
    ("autoMaxWidth", Bool),
    ("autoMaxHeight", Bool),
    ("footerDataPath", Str),
    ("width", Any),
    ("height", Any),
    ("multipleValueDataPath", Str),
    ("multipleValuePictureDataPath", Str),
    ("multipleValuePresentDataPath", Str),
    ("horizontalStretch", Bool),
    ("verticalStretch", Bool),
    ("horizontalAlign", Str),
    ("inputHint", Str),
];
const DOCUMENT_FIELD: Keys = &[
    ("path", Str),
    ("skipOnInput", Bool),
    ("title", Str),
    ("titleLocation", Str),
    ("width", Any),
    ("height", Any),
    ("autoMaxWidth", Bool),
    ("autoMaxHeight", Bool),
];

/// The key groups a compiled element kind writes, by its description key.
/// A kind without events (button, command bars, page, group) takes no `on`/`handlers`.
pub(super) fn element_keys(kind: &str) -> &'static [Keys] {
    match kind {
        "table" => &[TABLE, NAME, FLAGS, EVENTS],
        "label" => &[LABEL, NAME, FLAGS, EVENTS],
        "labelField" => &[LABEL_FIELD, NAME, FLAGS, TOOLTIP, EVENTS],
        "button" => &[BUTTON, NAME, FLAGS, TOOLTIP],
        "cmdBar" => &[COMMAND_BAR, NAME, FLAGS, CHILDREN],
        "autoCmdBar" => &[AUTO_COMMAND_BAR, NAME],
        "pages" => &[PAGES, NAME, FLAGS, CHILDREN, EVENTS],
        "page" => &[PAGE, NAME, FLAGS, CHILDREN],
        "group" => &[GROUP, NAME, FLAGS, CHILDREN],
        "check" => &[CHECK, NAME, FLAGS, TOOLTIP, EVENTS],
        "input" => &[INPUT, NAME, FLAGS, TOOLTIP, EVENTS],
        "html" => &[
            &[("html", Any)],
            DOCUMENT_FIELD,
            NAME,
            FLAGS,
            TOOLTIP,
            EVENTS,
        ],
        "textDoc" => &[
            &[("textDoc", Any)],
            DOCUMENT_FIELD,
            NAME,
            FLAGS,
            TOOLTIP,
            EVENTS,
        ],
        _ => &[],
    }
}

/// Keys the description used to take under another name, with the key the
/// compiler writes: the Russian names of the form-dsl specification and the
/// element keys it renamed.
const REPLACEMENTS: &[(&str, &str)] = &[
    ("пометка", "checked"),
    ("заголовок", "title"),
    ("ширина", "width"),
    ("высота", "height"),
    ("кнопкавыбора", "choiceButton"),
    ("кнопкаочистки", "clearButton"),
    ("кнопкавыпадающегосписка", "dropListButton"),
    ("кнопкасписковоговыбора", "choiceListButton"),
    ("быстрыйвыбор", "quickChoice"),
    ("формавыбора", "choiceForm"),
    ("историявыбораприводе", "choiceHistoryOnInput"),
    ("выборгруппиэлементов", "choiceFoldersAndItems"),
    ("фиксациявтаблице", "fixingInTable"),
    ("путькданнымподвала", "footerDataPath"),
    ("многострочныйрежим", "multiLine"),
    ("режимпароля", "passwordMode"),
    ("расположениезаголовка", "titleLocation"),
    ("цветтекста", "textColor"),
    ("цветфона", "backColor"),
    ("цветрамки", "borderColor"),
    ("цветтекстазаголовка", "titleTextColor"),
    ("цветфоназаголовка", "titleBackColor"),
    ("шрифтзаголовка", "titleFont"),
    ("цветтекстаподвала", "footerTextColor"),
    ("цветфонаподвала", "footerBackColor"),
    ("шрифтподвала", "footerFont"),
    ("шрифт", "font"),
    ("рамка", "border"),
    ("events", "on"),
    ("orientation", "group"),
];

/// The key the kind writes in place of `key`, when one is known.
fn replacement(groups: &[Keys], key: &str) -> Option<&'static str> {
    let normalized: String = key
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    REPLACEMENTS
        .iter()
        .find(|(from, _)| *from == normalized)
        .map(|(_, to)| *to)
        .filter(|to| key_type(groups, to).is_some())
}

fn key_type(groups: &[Keys], key: &str) -> Option<KeyType> {
    groups
        .iter()
        .flat_map(|group| group.iter())
        .find(|(name, _)| *name == key)
        .map(|(_, key_type)| *key_type)
}

/// Refuses an element key the kind does not write, a value of a type it does
/// not read, and a dynamic-list table key outside a dynamic list.
pub(super) fn check_element_keys(
    kind: &str,
    name: &str,
    object: &Map<String, Value>,
) -> Result<(), String> {
    let groups = element_keys(kind);
    for (key, value) in object {
        let Some(key_type) = key_type(groups, key) else {
            let hint = replacement(groups, key)
                .map(|to| format!("; use `{to}`"))
                .unwrap_or_default();
            return Err(format!(
                "key `{key}` is not written for form element kind `{kind}` (element `{name}`){hint}"
            ));
        };
        if !key_type.accepts(value) {
            return Err(format!(
                "key `{key}` of form element `{name}` must be {}",
                key_type.describe()
            ));
        }
    }
    if kind == "table" && object.get("_dynList").and_then(Value::as_bool) != Some(true) {
        if let Some(key) = TABLE_DYNAMIC_LIST_KEYS
            .iter()
            .find(|key| object.contains_key(**key))
        {
            return Err(format!(
                "key `{key}` of form element `{name}` is written only for a dynamic list (`_dynList: true`)"
            ));
        }
    }
    Ok(())
}

const ATTRIBUTE: Keys = &[
    ("name", Str),
    ("title", Str),
    ("type", Str),
    ("main", Bool),
    ("savedData", Bool),
    ("fillChecking", Str),
    ("settings", Object),
    ("columns", Array),
];
const ATTRIBUTE_COLUMN: Keys = &[("name", Str), ("type", Str), ("title", Str)];
const DYNAMIC_LIST_SETTINGS: Keys = &[
    ("manualQuery", Bool),
    ("dynamicDataRead", Bool),
    ("mainTable", Str),
];
const PARAMETER: Keys = &[("name", Str), ("type", Str), ("key", Bool)];
const COMMAND: Keys = &[
    ("name", Str),
    ("title", Str),
    ("tooltip", Text),
    ("shortcut", Str),
    ("picture", Str),
    ("action", Str),
    ("representation", Str),
];

/// Refuses a key the owner does not write and a value of a type it does not read.
/// `owner` is the phrase for the owner, `role` the word that precedes its name.
fn check_owner_keys(
    owner: &str,
    role: &str,
    name: &str,
    keys: Keys,
    object: &Map<String, Value>,
) -> Result<(), String> {
    for (key, value) in object {
        let Some(key_type) = key_type(&[keys], key) else {
            return Err(format!(
                "key `{key}` is not written for {owner} ({role} `{name}`)"
            ));
        };
        if !key_type.accepts(value) {
            return Err(format!(
                "key `{key}` of {owner} ({role} `{name}`) must be {}",
                key_type.describe()
            ));
        }
    }
    Ok(())
}

fn item_name(object: &Map<String, Value>) -> &str {
    object.get("name").and_then(Value::as_str).unwrap_or("")
}

/// Every item of `attributes`, `parameters` and `commands` is an object that
/// carries only the keys its emitter writes.
pub(super) fn check_root_item_keys(defn: &Value) -> Result<(), String> {
    for (list, owner, role, keys) in [
        ("attributes", "a form attribute", "attribute", ATTRIBUTE),
        ("parameters", "a form parameter", "parameter", PARAMETER),
        ("commands", "a form command", "command", COMMAND),
    ] {
        let Some(items) = defn.get(list) else {
            continue;
        };
        let Some(items) = items.as_array() else {
            return Err(format!("form description key `{list}` must be an array"));
        };
        for item in items {
            let Some(object) = item.as_object() else {
                return Err(format!("an item of `{list}` must be an object"));
            };
            let name = item_name(object);
            check_owner_keys(owner, role, name, keys, object)?;
            if list != "attributes" {
                continue;
            }
            if let Some(settings) = object.get("settings").and_then(Value::as_object) {
                if object.get("type").and_then(Value::as_str) != Some("DynamicList") {
                    return Err(format!(
                        "key `settings` is written only for a `DynamicList` attribute (attribute `{name}`)"
                    ));
                }
                check_owner_keys(
                    "dynamic list settings",
                    "attribute",
                    name,
                    DYNAMIC_LIST_SETTINGS,
                    settings,
                )?;
            }
            for column in object
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
            {
                check_owner_keys(
                    "an attribute column",
                    "column",
                    item_name(column),
                    ATTRIBUTE_COLUMN,
                    column,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/platform/form_description_keys.rs"]
mod tests;
