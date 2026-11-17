//! The keys of the form description each compiled owner writes (#2970).
//!
//! The compiler reads a description key by key; a key it does not read, or a
//! value of a JSON type it does not read, used to vanish behind a successful
//! call. Every owner here lists exactly the keys its emitter writes, with the
//! JSON type it reads, and anything else is refused by name. The keys an
//! element writer writes are one list with the `item.add` contract
//! (`unica_forms::contracts::FORM_ELEMENT_KINDS`); this module adds only the
//! keys of the description itself.

use serde_json::{Map, Value};
use unica_forms::contracts::{
    form_element_kind_by_description_key, ElementKeyType, ElementKeys, FORM_DYNAMIC_LIST_KEYS,
};

/// The JSON shape a description key is read as: the element keys share it
/// with the `item.add` contract (`unica_forms::contracts`).
pub(super) type KeyType = ElementKeyType;

type Keys = ElementKeys;

use ElementKeyType::{Any, Array, Bool, Object, Str, Text};

const NAME: Keys = &[("name", Str)];
const EVENTS: Keys = &[("on", Array), ("handlers", Object)];
const CHILDREN: Keys = &[("children", Array)];

/// Table keys the compiler writes only for a dynamic list (`_dynList: true`).
pub(super) const TABLE_DYNAMIC_LIST_KEYS: &[&str] = FORM_DYNAMIC_LIST_KEYS;

/// The automatic command bar of the form is a description key only:
/// `item.add` adds no such element.
const AUTO_COMMAND_BAR: Keys = &[
    ("autoCmdBar", Any),
    ("autoCommandBar", Any),
    ("horizontalAlign", Str),
    ("autofill", Bool),
];

/// The keys of the form description, not of the element, that a kind
/// takes besides the keys its writer writes (`FORM_ELEMENT_KINDS`): its
/// discriminator, nested elements, events, and the button type and page
/// orientation the description names its own way.
fn description_only_keys(kind: &str) -> &'static [Keys] {
    match kind {
        "table" => &[&[("table", Any), ("columns", Array)], NAME, EVENTS],
        "label" => &[&[("label", Any)], NAME, EVENTS],
        "labelField" => &[&[("labelField", Any)], NAME, EVENTS],
        "button" => &[&[("button", Any), ("type", Str)], NAME],
        "cmdBar" => &[&[("cmdBar", Any), ("commandBar", Any)], NAME, CHILDREN],
        "pages" => &[&[("pages", Any)], NAME, CHILDREN, EVENTS],
        "page" => &[&[("page", Any), ("group", Str)], NAME, CHILDREN],
        "group" => &[&[("group", Any)], NAME, CHILDREN],
        "check" => &[&[("check", Any)], NAME, EVENTS],
        "input" => &[&[("input", Any)], NAME, EVENTS],
        "html" => &[&[("html", Any)], NAME, EVENTS],
        "textDoc" => &[&[("textDoc", Any)], NAME, EVENTS],
        _ => &[],
    }
}

/// The key groups a compiled element kind accepts, by its description key:
/// the keys its writer writes, one list with the `item.add` contract, and
/// the keys of the description only. A kind without events (button,
/// command bars, page, group) takes no `on`/`handlers`.
pub(super) fn element_keys(kind: &str) -> Vec<Keys> {
    if kind == "autoCmdBar" {
        return vec![AUTO_COMMAND_BAR, NAME];
    }
    let mut groups = description_only_keys(kind).to_vec();
    groups.extend(form_element_kind_by_description_key(kind).map(|element| element.keys));
    groups
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
        let Some(key_type) = key_type(&groups, key) else {
            let hint = replacement(&groups, key)
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
