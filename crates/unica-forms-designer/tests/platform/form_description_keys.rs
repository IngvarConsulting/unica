//! Every description key a compiled owner accepts reaches its XML (#2970).
use super::super::form_compile_xml;
use super::{element_keys, TABLE_DYNAMIC_LIST_KEYS};
use serde_json::{json, Map, Value};

/// Keys a kind accepts whose absence and probe value write the same XML.
/// The list only shrinks: a key here is accepted without a trace.
const ACCEPTED_WITHOUT_TRACE: &[(&str, &str, &str)] = &[];

fn attributes() -> Value {
    json!([
        { "name": "Реквизит", "type": "string" },
        { "name": "Флаг", "type": "boolean" },
        { "name": "Документ", "type": "string" },
        { "name": "Строки", "type": "ValueTable", "columns": [
            { "name": "Колонка", "type": "string" },
            { "name": "Картинка", "type": "string" }
        ]},
        { "name": "Значения", "type": "ValueList" },
        { "name": "Список", "type": "DynamicList", "settings": { "mainTable": "Catalog.Товары" } }
    ])
}

/// The smallest element of the kind that compiles.
fn base(kind: &str) -> Map<String, Value> {
    let value = match kind {
        "table" => json!({ "table": "Таблица", "path": "Строки", "columns": [] }),
        "label" => json!({ "label": "Надпись" }),
        "labelField" => json!({ "labelField": "Надпись", "path": "Реквизит" }),
        "button" => json!({ "button": "Кнопка" }),
        "cmdBar" => json!({ "cmdBar": "Панель" }),
        "autoCmdBar" => json!({ "autoCmdBar": "ФормаКоманднаяПанель" }),
        "pages" => json!({ "pages": "Страницы", "children": [] }),
        "page" => json!({ "page": "Страница" }),
        "group" => json!({ "group": "vertical", "name": "Группа" }),
        "check" => json!({ "check": "Флажок", "path": "Флаг" }),
        "input" => json!({ "input": "Поле", "path": "Реквизит" }),
        "html" => json!({ "html": "Документ", "path": "Документ" }),
        other => panic!("no base element for `{other}`"),
    };
    value.as_object().unwrap().clone()
}

/// A value of `key` that differs from its absence; `None` for keys the base
/// element already carries.
fn probe(kind: &str, key: &str) -> Option<Value> {
    let value = match (kind, key) {
        (_, "name") => json!("Другое"),
        (_, k) if k == kind || (kind == "cmdBar" && k == "commandBar") => return None,
        (_, "autoCommandBar") if kind == "autoCmdBar" => return None,
        ("table" | "labelField" | "check" | "input" | "html", "path") => return None,
        ("pages" | "page" | "group" | "cmdBar", "children") => json!([{ "label": "Внутри" }]),
        ("table", "columns") => json!([{ "labelField": "Колонка", "path": "Строки.Колонка" }]),
        (_, "visible" | "userVisible" | "enabled") => json!(false),
        (_, "hidden" | "disabled" | "readOnly") => json!(true),
        (_, "tooltip") => json!("Подсказка"),
        (_, "tooltipRepresentation") => json!("Button"),
        (_, "title") => json!("Заголовок"),
        (_, "on") => json!([{ "event": event(kind), "handler": "Обработчик" }]),
        (_, "handlers") => json!({ event(kind): "Обработчик" }),
        (_, "width") => json!(20),
        (_, "height") => json!(3),
        (_, "autoMaxWidth" | "autoMaxHeight" | "showRoot" | "allowGettingCurrentRowURL") => {
            json!(false)
        }
        (_, "footerDataPath") => json!("Реквизит"),
        (_, "titleDataPath") => json!("Реквизит"),
        (_, "titleLocation") => json!("top"),
        (_, "currentRowUse") => json!("Use"),
        (_, "showTitle") => json!(false),
        (_, "skipOnInput") => json!(true),
        ("autoCmdBar", "autofill") => json!(false),
        (_, "autofill") => json!(true),
        (_, "horizontalAlign") => json!("Right"),
        ("table", "representation") => json!("tree"),
        ("group", "representation") => json!("weakSeparation"),
        ("button", "representation") => json!("picture"),
        ("group" | "page", "group") => json!("horizontal"),
        ("table", "_dynList") => json!(true),
        ("table", "tableAutofill") => json!(false),
        ("table", "autoRefreshPeriod") => json!(5),
        ("table", "choiceFoldersAndItems") => json!("Folders"),
        ("table", "updateOnDataChange") => json!("DontUpdate"),
        ("table", "commandBarLocation") => json!("Bottom"),
        ("table", "initialTreeView") => json!("ExpandAllLevels"),
        ("table", "rowPictureDataPath") => json!("Список.Картинка"),
        ("label", "formatted") => json!(true),
        ("button", "type") => json!("hyperlink"),
        ("button", "command" | "commandName" | "stdCommand") => json!("Закрыть"),
        ("button", "picture") => json!("StdPicture.Close"),
        ("button", "locationInCommandBar") => json!("InAdditionalSubmenu"),
        ("button", "backColor") => json!("style:ButtonBackColor"),
        ("button", "font") => json!("style:NormalTextFont"),
        ("cmdBar", "commandSource") => json!("Form"),
        ("pages", "pagesRepresentation") => json!("None"),
        ("group", "behavior") => json!("collapsible"),
        ("check", "checkBoxType") => json!("switcher"),
        ("input", "headerHorizontalAlign") => json!("Right"),
        ("input", "inputHint") => json!("Введите"),
        (
            "input",
            "multipleValueDataPath"
            | "multipleValuePictureDataPath"
            | "multipleValuePresentDataPath",
        ) => return None,
        (_, _) => json!(true),
    };
    Some(value)
}

fn event(kind: &str) -> &'static str {
    match kind {
        "table" => "Selection",
        "label" => "Click",
        "html" => "OnClick",
        "pages" => "OnCurrentPageChange",
        _ => "OnChange",
    }
}

fn compile(element: &Map<String, Value>) -> Result<String, String> {
    let definition = json!({ "attributes": attributes(), "elements": [element] });
    form_compile_xml(&definition, "2.20").map(|(xml, _)| xml)
}

#[test]
fn every_key_a_compiled_kind_accepts_reaches_its_xml() {
    let kinds = [
        "table",
        "label",
        "labelField",
        "button",
        "cmdBar",
        "autoCmdBar",
        "pages",
        "page",
        "group",
        "check",
        "input",
        "html",
    ];
    let mut silent = Vec::new();
    for kind in kinds {
        let mut element = base(kind);
        if kind == "table" {
            element.insert("path".to_string(), json!("Список"));
            element.insert("_dynList".to_string(), json!(true));
        }
        let before = compile(&element).unwrap_or_else(|error| panic!("{kind}: {error}"));
        for (key, _) in element_keys(kind).iter().flat_map(|group| group.iter()) {
            let Some(value) = probe(kind, key) else {
                continue;
            };
            if kind == "table" && *key == "_dynList" {
                continue;
            }
            // `handlers` names the handlers of the events listed in `on`.
            let mut before_key = element.clone();
            if *key == "handlers" {
                before_key.insert("on".to_string(), json!([event(kind)]));
            }
            let before = if *key == "handlers" {
                compile(&before_key).unwrap_or_else(|error| panic!("{kind}.on: {error}"))
            } else {
                before.clone()
            };
            let mut probed = before_key;
            probed.insert((*key).to_string(), value);
            let after = compile(&probed).unwrap_or_else(|error| panic!("{kind}.{key}: {error}"));
            let known = ACCEPTED_WITHOUT_TRACE
                .iter()
                .any(|(k, name, _)| *k == kind && name == key);
            if (after == before) != known {
                silent.push(format!("{kind}.{key}"));
            }
        }
    }
    assert!(silent.is_empty(), "accepted without a trace: {silent:?}");
}

#[test]
fn a_dynamic_list_table_key_outside_a_dynamic_list_is_refused() {
    for key in TABLE_DYNAMIC_LIST_KEYS {
        let mut element = base("table");
        element.insert(key.to_string(), probe("table", key).unwrap());
        assert_eq!(
            compile(&element).unwrap_err(),
            format!("key `{key}` of form element `Таблица` is written only for a dynamic list (`_dynList: true`)")
        );
    }
}

#[test]
fn a_key_the_kind_does_not_write_is_refused_by_name() {
    for (kind, key) in [
        ("input", "children"),
        ("input", "contextMenu"),
        ("input", "extendedTooltip"),
        ("table", "commandBar"),
        ("table", "children"),
        ("labelField", "title"),
        ("label", "tooltip"),
        ("group", "tooltip"),
        ("button", "on"),
        ("autoCmdBar", "children"),
        ("check", "мультистрока"),
    ] {
        let mut element = base(kind);
        element.insert(key.to_string(), json!([]));
        let name = element
            .get("name")
            .or_else(|| element.get(kind))
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        assert_eq!(
            compile(&element).unwrap_err(),
            format!("key `{key}` is not written for form element kind `{kind}` (element `{name}`)"),
            "{kind}.{key}"
        );
    }
}

/// A key the description used to take under another name names the key the
/// kind writes; a key no kind writes names nothing.
#[test]
fn a_renamed_key_names_its_replacement() {
    for (kind, key, tail) in [
        ("input", "Заголовок", "; use `title`"),
        ("input", "Многострочный режим", "; use `multiLine`"),
        ("button", "ЦветФона", "; use `backColor`"),
        ("table", "events", "; use `on`"),
        ("group", "orientation", "; use `group`"),
        ("label", "ЦветФона", ""),
        ("input", "contextMenu", ""),
    ] {
        let mut element = base(kind);
        element.insert(key.to_string(), json!("x"));
        let name = element
            .get("name")
            .or_else(|| element.get(kind))
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        assert_eq!(
            compile(&element).unwrap_err(),
            format!("key `{key}` is not written for form element kind `{kind}` (element `{name}`){tail}"),
            "{kind}.{key}"
        );
    }
}

#[test]
fn a_value_of_a_type_the_kind_does_not_read_is_refused() {
    for (kind, key, value, expected) in [
        ("input", "title", json!({"ru": "Заголовок"}), "a string"),
        ("input", "multiLine", json!("true"), "a boolean"),
        (
            "input",
            "userVisible",
            json!({"common": false}),
            "a boolean",
        ),
        (
            "button",
            "picture",
            json!({"ref": "StdPicture.Close"}),
            "a string",
        ),
        ("table", "columns", json!({}), "an array"),
    ] {
        let mut element = base(kind);
        element.insert(key.to_string(), value);
        let error = compile(&element).unwrap_err();
        assert!(
            error.starts_with(&format!("key `{key}` of form element `"))
                && error.ends_with(&format!("must be {expected}")),
            "{kind}.{key}: {error}"
        );
    }
}

#[test]
fn an_element_that_is_not_an_object_is_refused() {
    let error = form_compile_xml(&json!({ "elements": ["Поле"] }), "2.20").unwrap_err();
    assert_eq!(error, "a form element must be an object");
}

#[test]
fn attributes_parameters_and_commands_take_only_the_keys_they_write() {
    for (definition, expected) in [
        (
            json!({ "attributes": [{ "name": "А", "type": "string", "view": false }] }),
            "key `view` is not written for a form attribute (attribute `А`)",
        ),
        (
            json!({ "attributes": [{ "name": "А", "type": "string", "savedData": "yes" }] }),
            "key `savedData` of a form attribute (attribute `А`) must be a boolean",
        ),
        (
            json!({ "attributes": [{ "name": "А", "type": "string", "settings": { "mainTable": "Catalog.Товары" } }] }),
            "key `settings` is written only for a `DynamicList` attribute (attribute `А`)",
        ),
        (
            json!({ "attributes": [{ "name": "А", "type": "DynamicList", "settings": { "query": "ВЫБРАТЬ 1" } }] }),
            "key `query` is not written for dynamic list settings (attribute `А`)",
        ),
        (
            json!({ "attributes": [{ "name": "Т", "type": "ValueTable", "columns": [{ "name": "К", "type": "string", "fillCheck": true }] }] }),
            "key `fillCheck` is not written for an attribute column (column `К`)",
        ),
        (
            json!({ "parameters": [{ "name": "П", "type": "string", "use": true }] }),
            "key `use` is not written for a form parameter (parameter `П`)",
        ),
        (
            json!({ "commands": [{ "name": "К", "action": "К", "use": true }] }),
            "key `use` is not written for a form command (command `К`)",
        ),
        (
            json!({ "parameters": {} }),
            "form description key `parameters` must be an array",
        ),
        (
            json!({ "commands": ["К"] }),
            "an item of `commands` must be an object",
        ),
    ] {
        assert_eq!(form_compile_xml(&definition, "2.20").unwrap_err(), expected);
    }
}
