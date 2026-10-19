//! Argument contracts shared by the operation hint and the apply boundary.
//! The closed operation registry owns names and applicability; this module
//! describes and normalizes the arguments consumed by supported planners.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

mod dcs;
mod event;
mod form;
mod metadata;
mod mxl;
#[cfg(test)]
pub(crate) use form::{form_element_enum, FORM_ELEMENT_KINDS};
pub(crate) use form::{FORM_DYNAMIC_LIST_KEYS, FORM_ELEMENT_EVENT_KEYS, FORM_ELEMENT_NESTED_KEYS};
#[cfg(test)]
use metadata::IDENTIFIER_PATTERN;
pub(crate) use metadata::{
    metadata_borrow_input_schema, metadata_element_input_schema, metadata_help_create_input_schema,
    metadata_host_element_schema, metadata_object_create_input_schema,
    metadata_object_remove_input_schema, metadata_predefined_element_input_schema,
    metadata_relation_input_schema, MetadataSite,
};

pub(crate) const MXL_MAX_ROW: i64 = 10_000;
pub(crate) const MXL_MAX_COLUMN: i64 = 1_000;

/// Preserve the existing integer spelling accepted at the apply boundary,
/// including leading zeros and a leading plus; hints and planner share it.
pub(crate) fn parse_mxl_cell_address(key: &str) -> Option<(i64, i64)> {
    let rest = key.strip_prefix('R')?;
    let (row, column) = rest.split_once('C')?;
    let row = row.parse::<i64>().ok()?;
    let column = column.parse::<i64>().ok()?;
    ((1..=MXL_MAX_ROW).contains(&row) && (1..=MXL_MAX_COLUMN).contains(&column))
        .then_some((row, column))
}

pub(crate) struct OperationContract {
    pub(crate) schema: Value,
    pub(crate) target: &'static str,
    pub(crate) effect: &'static str,
    pub(crate) notes: &'static str,
    pub(crate) example_args: Value,
    /// Plugin reference that explains the use case of the operation.
    pub(crate) help: &'static str,
}

const REPORTS_HELP: &str = "references/use-cases/reports-printing.md";

#[derive(Debug, PartialEq)]
pub(crate) struct ContractError {
    pub(crate) path: String,
    pub(crate) message: String,
}

/// A borrowed part of an existing argument contract. Decoders visit it in
/// their established diagnostic order, rather than normalizing an entire
/// request before choosing its target or converting domain values.
#[derive(Clone, Copy)]
pub(crate) struct ArgumentShape<'a> {
    schema: &'a Value,
}

impl<'a> ArgumentShape<'a> {
    pub(crate) fn new(schema: &'a Value) -> Self {
        Self { schema }
    }

    pub(crate) fn property(self, name: &str) -> Option<Self> {
        self.schema["properties"]
            .get(name)
            .map(Self::new)
            .or_else(|| {
                ["oneOf", "anyOf"].into_iter().find_map(|choice| {
                    self.schema[choice]
                        .as_array()?
                        .iter()
                        .find_map(|schema| Self::new(schema).property(name))
                })
            })
    }

    pub(crate) fn requires(self, name: &str) -> bool {
        self.required_names().any(|key| key == name)
    }

    pub(crate) fn property_names(self) -> std::vec::IntoIter<&'a str> {
        let mut names: Vec<_> = self.schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|properties| properties.keys().map(String::as_str))
            .collect();
        for choice in ["oneOf", "anyOf"] {
            for branch in self.schema[choice].as_array().into_iter().flatten() {
                names.extend(Self::new(branch).property_names());
            }
        }
        names.into_iter()
    }

    pub(crate) fn aliases(self) -> impl Iterator<Item = &'a str> {
        self.schema["x-unica-aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    pub(crate) fn required_names(self) -> impl Iterator<Item = &'a str> {
        self.schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    pub(crate) fn items(self) -> Option<Self> {
        self.schema.get("items").map(Self::new)
    }

    pub(crate) fn additional_properties(self) -> Option<Self> {
        self.schema
            .get("additionalProperties")
            .filter(|schema| schema.is_object())
            .map(Self::new)
    }

    pub(crate) fn variant(self, discriminator: &str, value: &str) -> Option<Self> {
        self.schema["oneOf"].as_array()?.iter().find_map(|schema| {
            schema["properties"][discriminator]["enum"]
                .as_array()
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|candidate| candidate.as_str() == Some(value))
                })
                .then(|| Self::new(schema))
        })
    }

    /// Uses the existing validator's supported primitive/key constraints.
    /// Compound Metadata invariants remain with their typed converters;
    /// this is deliberately not a full JSON Schema interpreter.
    pub(crate) fn validate_field(self, value: &Value, path: &str) -> Result<(), ContractError> {
        if value.is_object() || value.is_array() {
            // The typed converter visits the members and checks compound
            // constraints in its established order. Check only the container
            // type here, so a later domain error cannot overtake an earlier
            // conversion error in another item.
            self.validate_type(value, path)
        } else {
            validate(self.schema, value, path)
        }
    }

    pub(crate) fn validate_type(self, value: &Value, path: &str) -> Result<(), ContractError> {
        let schema = json!({"type": self.schema["type"]});
        // Metadata's existing typed numeric conversion accepts integral JSON
        // spellings such as 1.0 and 1e0. Checking their shape must not impose
        // the legacy normalizer's narrower integer serialization spelling.
        if self.schema["type"] == "integer"
            && value
                .as_f64()
                .is_some_and(|number| number.is_finite() && number.fract() == 0.0)
        {
            return Ok(());
        }
        validate(&schema, value, path)
    }
}

pub(super) fn text(nonempty: bool) -> Value {
    if nonempty {
        json!({"type":"string", "pattern":r"\S"})
    } else {
        json!({"type":"string"})
    }
}

pub(super) fn object(fields: &[(&str, Value)], required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false,
        "properties":fields.iter().map(|(key,value)| (key.to_string(),value.clone())).collect::<Map<_,_>>(),
        "required":required})
}

impl OperationContract {
    pub(crate) fn form_move() -> Self {
        Self {
            schema: object(&[("at", text(true)), ("values", object(&[("into", json!({"anyOf":[text(true), {"type":"null"}]})), ("after", text(true))], &["into"]))], &["values"]),
            target: "The existing Item at its full current path: Owner.Name.Form.F.Item.A[.Item.B…]. The address alone selects the source.",
            effect: "Move an existing XML subtree, preserving IDs, properties, events and contained companions. Incompatible placements are refused without conversion.",
            notes: "Names are exact. into is required: use a container name, or null for the form root. Omit after for the end. after must be a direct child of the destination and cannot name the source. Exact placement is a no-op. A failure publishes nothing.",
            example_args: json!({"values":{"into":"Destination"}}),
            help: "skills/form-edit/SKILL.md",
        }
    }

    pub(crate) fn normalize(&self, args: &Value, path: &str) -> Result<Value, ContractError> {
        validate(&self.schema, args, path)?;
        let mut args = args.clone();
        defaults(&self.schema, &mut args);
        normalize_aliases(&self.schema, &mut args);
        Ok(args)
    }

    pub(crate) fn details(&self) -> Value {
        json!({"argsSchema":self.schema,"target":self.target,"effect":self.effect,
            "notes":self.notes,"exampleArgs":self.example_args,
            "exampleUse":"Read existing targets and required names through view. For creation, choose a new unused name. These argument examples do not establish that their names exist in this workspace.",
            "help":{"scope":"plugin","path":self.help},
            "execution":"Preview with at/ops; execute only data.executionToken from the successful plan, then read the post-image and check. Observe the same Task to terminal state; never repeat a mutation to wait for it."})
    }
}

/// Compiled grammar of an owned contract pattern. Every pattern comes from a
/// schema built in this crate, so the set is closed and the cache is bounded.
/// Compiling the Unicode identifier class anew for each validated argument
/// dominated apply preflight (#1441).
///
/// The shared `Arc` keeps one match cache warm across calls: cloning `Regex`
/// would start each validation with an empty one.
fn contract_pattern(pattern: &str) -> Arc<regex::Regex> {
    static COMPILED: OnceLock<Mutex<HashMap<String, Arc<regex::Regex>>>> = OnceLock::new();
    let mut compiled = COMPILED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(regex) = compiled.get(pattern) {
        return Arc::clone(regex);
    }
    let regex = Arc::new(regex::Regex::new(pattern).expect("owned contract pattern"));
    compiled.insert(pattern.to_owned(), Arc::clone(&regex));
    regex
}

fn error(path: &str, message: impl Into<String>) -> ContractError {
    ContractError {
        path: path.into(),
        message: message.into(),
    }
}

fn validate(schema: &Value, value: &Value, path: &str) -> Result<(), ContractError> {
    if schema
        .get("not")
        .is_some_and(|excluded| validate(excluded, value, path).is_ok())
    {
        return Err(error(path, "argument selects an excluded alternative"));
    }
    let valid_type = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some(),
        Some("null") => value.is_null(),
        None => true,
        other => unreachable!("unsupported internal schema type: {other:?}"),
    };
    if !valid_type {
        return Err(error(path, format!("expected {}", schema["type"])));
    }
    if let Some(minimum) = schema["minimum"].as_i64() {
        if value.as_i64().is_none_or(|number| number < minimum) {
            return Err(error(path, "integer is below the supported minimum"));
        }
    }
    if schema["format"] == "unica-mxl-cell-address"
        && value
            .as_str()
            .is_none_or(|key| parse_mxl_cell_address(key).is_none())
    {
        return Err(error(
            path,
            format!("expected area-local R<row>C<column>, up to R{MXL_MAX_ROW}C{MXL_MAX_COLUMN}"),
        ));
    }
    if let Some(pattern) = schema["pattern"].as_str() {
        if !value
            .as_str()
            .is_some_and(|text| contract_pattern(pattern).is_match(text))
        {
            return Err(error(
                path,
                "text does not match the supported argument grammar",
            ));
        }
    }
    if let Some(values) = schema["enum"].as_array() {
        if !values.contains(value) {
            return Err(error(
                path,
                "value is not one of the supported alternatives",
            ));
        }
    }
    // Различитель выбирает вариант по значению ключа (вид элемента формы):
    // ошибка варианта называет точный путь, а не «ни одна альтернатива».
    // Без ключа выбирается вариант, у которого различитель имеет `default`.
    if let (Some(alternatives), Some(key), Some(object)) = (
        schema["anyOf"].as_array(),
        schema["x-unica-discriminator"].as_str(),
        value.as_object(),
    ) {
        let selected = object.get(key);
        let variant = alternatives.iter().find(|variant| {
            let discriminator = &variant["properties"][key];
            match selected {
                Some(selected) => discriminator["enum"]
                    .as_array()
                    .is_some_and(|values| values.contains(selected)),
                None => discriminator.get("default").is_some(),
            }
        });
        return match variant {
            Some(variant) => validate(variant, value, path),
            None => Err(error(
                &format!("{path}.{key}"),
                "value is not one of the supported alternatives",
            )),
        };
    }
    if let Some(alternatives) = schema["anyOf"].as_array() {
        if !alternatives
            .iter()
            .any(|choice| validate(choice, value, path).is_ok())
        {
            return Err(error(
                path,
                "one of the supported argument alternatives must match",
            ));
        }
    }
    if let Some(object) = value.as_object() {
        if schema["minProperties"]
            .as_u64()
            .is_some_and(|minimum| (object.len() as u64) < minimum)
        {
            return Err(error(path, "object must not be empty"));
        }
        if let Some(names) = schema.get("propertyNames") {
            for key in object.keys() {
                validate(names, &Value::String(key.clone()), &format!("{path}.{key}"))?;
            }
        }
        if schema["additionalProperties"].is_object() {
            for (key, value) in object {
                if schema["properties"].get(key).is_none() {
                    validate(
                        &schema["additionalProperties"],
                        value,
                        &format!("{path}.{key}"),
                    )?;
                }
            }
        }
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(error(
                        &format!("{path}.{key}"),
                        "required argument is missing",
                    ));
                }
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (key, value) in object {
                match properties.get(key) {
                    Some(child) => validate(child, value, &format!("{path}.{key}"))?,
                    None if schema["additionalProperties"] == false => {
                        return Err(error(&format!("{path}.{key}"), "unknown argument"))
                    }
                    None => {}
                }
            }
        }
    }
    if let Some(values) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|minimum| (values.len() as u64) < minimum)
        {
            return Err(error(path, "array must not be empty"));
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        for (index, value) in values.iter().enumerate() {
            validate(items, value, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn defaults(schema: &Value, value: &mut Value) {
    if let (Some(properties), Some(object)) =
        (schema["properties"].as_object(), value.as_object_mut())
    {
        for (key, child) in properties {
            if !object.contains_key(key) {
                if let Some(default) = child.get("default") {
                    object.insert(key.clone(), default.clone());
                }
            }
            if let Some(value) = object.get_mut(key) {
                defaults(child, value);
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            defaults(items, value);
        }
    }
}

fn normalize_aliases(schema: &Value, value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        if let Some(names) = schema["x-unica-aliases"].as_array() {
            let names: Vec<_> = names.iter().filter_map(Value::as_str).collect();
            let selected = names.iter().find_map(|name| object.get(*name).cloned());
            for name in &names {
                object.remove(*name);
            }
            if let (Some(canonical), Some(selected)) = (names.first(), selected) {
                object.insert((*canonical).to_string(), selected);
            }
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (key, child) in properties {
                if let Some(value) = object.get_mut(key) {
                    normalize_aliases(child, value);
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            normalize_aliases(items, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::apply::{OperationFamily, OperationRegistry, IMPLEMENTED_APPLY_OPERATIONS};
    use crate::domain::role_operation::RoleOperation;

    fn dcs_contract(name: &str) -> Option<OperationContract> {
        crate::domain::dcs_operation::Primitive::named(name)
            .map(crate::domain::dcs_operation::Primitive::contract)
    }

    #[test]
    fn typed_metadata_contracts_cover_each_registered_identity_and_site() {
        let sites = std::iter::once(None).chain(
            crate::domain::metadata::MetadataKind::ALL
                .iter()
                .flat_map(|owner| {
                    [false, true].map(|in_section| {
                        Some(MetadataSite {
                            owner: *owner,
                            in_section,
                        })
                    })
                }),
        );
        for site in sites {
            for descriptor in OperationRegistry::closed()
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.family() == OperationFamily::Metadata)
            {
                let operation = descriptor
                    .metadata_operation()
                    .expect("every Metadata registration is typed");
                let contract = operation.contract_at(site);
                assert_eq!(contract.schema["type"], "object", "{}", operation.name());
                assert_eq!(contract.schema["additionalProperties"], false);
                assert_eq!(contract.schema["properties"]["at"]["type"], "string");
                assert!(descriptor.argument_contract().is_some());
            }
        }
    }

    /// Скомпилированная грамматика берётся из кеша по своему шаблону: повторная
    /// проверка не подменяет одну грамматику другой (#1441).
    #[test]
    fn a_contract_pattern_keeps_its_own_grammar_on_repeated_validation() {
        let identifier = json!({"type": "string", "pattern": IDENTIFIER_PATTERN});
        let non_blank = json!({"type": "string", "pattern": r"\S"});
        for _ in 0..2 {
            assert!(validate(&identifier, &json!("Реквизит_1"), "args").is_ok());
            assert!(validate(&identifier, &json!("1Реквизит"), "args").is_err());
            assert!(validate(&non_blank, &json!("1Реквизит"), "args").is_ok());
            assert!(validate(&non_blank, &json!(" "), "args").is_err());
        }
    }

    #[test]
    fn mxl_and_template_contracts_cover_examples_and_keep_legacy_value_grammar() {
        let registry = OperationRegistry::closed();
        for name in ["mxl.set", "area.add", "template.add"] {
            let descriptor = registry
                .descriptors()
                .iter()
                .find(|d| d.name() == name)
                .unwrap();
            let contract = descriptor.argument_contract().expect("shared contract");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
        }
    }

    /// Контракт прав публикует ту же форму, что принимает планировщик
    /// (#1354): объект — только `Kind.Name`, право — значение или
    /// ограничение, объект записи прав не аргумент `right.set`.
    #[test]
    fn role_contracts_publish_the_entry_shape() {
        let registry = OperationRegistry::closed();
        for descriptor in registry
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.family() == OperationFamily::Role)
        {
            let op = descriptor.name();
            let contract = descriptor
                .argument_contract()
                .expect("every implemented Role operation publishes its argument contract");
            assert!(
                contract.normalize(&contract.example_args, "args").is_ok(),
                "{op}"
            );
        }
        let create = RoleOperation::Create.contract();
        for name in [
            "Reader",
            "МенеджерПродаж",
            "_Reader2",
            " Reader ",
            "\tЧитатель\r\n",
            "\u{a0}Ёж\u{a0}",
        ] {
            assert!(
                create
                    .normalize(&json!({"values":{"name":name}}), "args")
                    .is_ok(),
                "{name:?}"
            );
        }
        for values in [
            json!({}),
            json!({"name":""}),
            json!({"name":"  "}),
            json!({"name":1}),
            json!({"name":null}),
            json!({"name":"1Reader"}),
            json!({"name":"Role.Reader"}),
            json!({"name":"Reader/Other"}),
            json!({"name":"Reader", "synonym":"Ignored"}),
        ] {
            assert!(
                create.normalize(&json!({"values":values}), "args").is_err(),
                "{values}"
            );
        }
        assert!(create.example_args.get("at").is_none());
        let add = RoleOperation::RightAdd.contract();
        for (args, path) in [
            (json!({"items":[]}), "args.items"),
            (
                json!({"items":[{"object":"Catalog.A","rights":{}}]}),
                "args.items[0].rights",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":"yes"}}]}),
                "args.items[0].rights.Read",
            ),
            (
                json!({"items":[{"object":"DataProcessor.A.Command","rights":{"Use":true}}]}),
                "args.items[0].object",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":true},"rls":"x"}]}),
                "args.items[0].rls",
            ),
        ] {
            let error = add.normalize(&args, "args").unwrap_err();
            assert_eq!(error.path, path, "{args}");
        }
        // Вложенный объект — одна запись роли (#1356).
        assert!(add
            .normalize(
                &json!({"items":[{"object":"DataProcessor.A.Command.B","rights":{"Use":true}}]}),
                "args"
            )
            .is_ok());
        let set = RoleOperation::RightSet.contract();
        assert!(set
            .normalize(
                &json!({"values":{"right":"Read","rls":"ГДЕ ИСТИНА"}}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"values":{"right":"Read"}}),
            json!({"values":{"object":"Catalog.A","right":"Read","value":true}}),
            json!({"values":{"right":"Read","value":"true"}}),
        ] {
            assert!(set.normalize(&args, "args").is_err(), "{args}");
        }
    }

    #[test]
    fn mxl_contract_checks_dynamic_keys_scalars_and_exact_integer_values() {
        let contract = crate::domain::mxl_operation::MxlOperation::Set.contract();
        for key in ["R1C1", "R01C01", "R+1C+1", "R10000C1000"] {
            assert!(
                contract
                    .normalize(
                        &json!({"values":{"cells":{key:null},"columns":1001}}),
                        "args"
                    )
                    .is_ok(),
                "{key}"
            );
        }
        for values in [
            json!({"cells":[]}),
            json!({"cells":{}}),
            json!({"cells":{"R0C1":"x"}}),
            json!({"cells":{"R10001C1":"x"}}),
            json!({"cells":{"R1C1001":"x"}}),
            json!({"cells":{"R1C1":{}}}),
            json!({"cells":{"R1C1":"x"},"columns":0}),
            json!({"cells":{"R1C1":"x"},"columns":1.0}),
            serde_json::from_str(r#"{"cells":{"R1C1":"x"},"columns":1e0}"#).unwrap(),
            json!({"cells":{"R1C1":"x"},"columns":u64::MAX}),
            json!({"cells":{"R1C1":"x"},"style":"bold"}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err(),
                "{values}"
            );
        }
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x","R1C2":42,"R1C3":true,"R1C4":null}}}),
                "args"
            )
            .is_ok());
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x"},"columns":i64::MAX}}),
                "args"
            )
            .is_ok());
        // Область называет адрес (#1354): ключ, который её называл, снят.
        let error = contract
            .normalize(&json!({"values":{"area":"A","cells":{"R1C1":"x"}}}), "args")
            .unwrap_err();
        assert_eq!(error.path, "args.values.area");
        // `area.add` называет новую область и её начальные ячейки.
        let area = crate::domain::mxl_operation::MxlOperation::AreaAdd.contract();
        assert!(area
            .normalize(
                &json!({"items":[{"name":"A","cells":{"R1C1":"x"},"columns":2}]}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"items":[]}),
            json!({"items":[{"name":"A"}]}),
            json!({"items":[{"cells":{"R1C1":"x"}}]}),
            json!({"items":[{"name":"A","cells":{"R0C1":"x"}}]}),
        ] {
            assert!(area.normalize(&args, "args").is_err(), "{args}");
        }
        let creation = OperationContract::metadata_template_add();
        for kind in [
            Value::Null,
            json!(7),
            json!(true),
            json!({}),
            json!("SpreadsheetDocument"),
        ] {
            assert!(creation
                .normalize(
                    &json!({"items":[{"name":"Layout","templateType":kind,"synonym":7}]}),
                    "args"
                )
                .is_ok());
        }
        assert_eq!(
            creation
                .normalize(&json!({"items":[{"name":"Layout"}]}), "args")
                .unwrap()["items"][0]["templateType"],
            "SpreadsheetDocument"
        );
        assert!(creation
            .normalize(
                &json!({"items":[{"name":"Layout","templateType":"Unknown"}]}),
                "args"
            )
            .is_err());
        assert!(creation.normalize(&json!({"items":[]}), "args").is_err());
        assert_eq!(parse_mxl_cell_address("R01C+01"), Some((1, 1)));
    }

    #[test]
    fn form_move_contract_selects_one_source_and_preserves_exact_names() {
        for (args, accepted) in [
            (
                json!({"values":{"into":" Group ","after":" Previous "}}),
                true,
            ),
            (json!({"values":{"into":null}}), true),
            (json!({"values":{}}), false),
            (json!({"values":{"into":false}}), false),
            (json!({"values":{"into":""}}), false),
            (json!({"values":{"into":"G","name":"A"}}), false),
            (json!({"items":[{"name":"A","into":"G"}]}), false),
            (json!({"values":{"into":"G","after":null}}), false),
            (json!({"values":{"into":"G","before":"A"}}), false),
        ] {
            let contract = OperationContract::form_move();
            let validator = jsonschema::validator_for(&contract.schema).unwrap();
            assert_eq!(validator.is_valid(&args), accepted, "{args}");
            let normalized = contract.normalize(&args, "ops[3].args");
            assert_eq!(normalized.is_ok(), accepted, "{args}: {normalized:?}");
            if accepted {
                assert_eq!(normalized.unwrap(), args);
            }
        }
        let error = OperationContract::form_move()
            .normalize(&json!({"values":{"into":false}}), "ops[3].args")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.into");
    }

    #[test]
    fn every_implemented_dcs_operation_has_a_valid_example_and_small_contract() {
        let registry = OperationRegistry::closed();
        let mut count = 0;
        for descriptor in registry.descriptors().iter().filter(|descriptor| {
            descriptor.family() == OperationFamily::Dcs
                && IMPLEMENTED_APPLY_OPERATIONS.contains(&descriptor.name())
        }) {
            let contract = descriptor
                .argument_contract()
                .unwrap_or_else(|| panic!("{} lacks its DCS contract", descriptor.name()));
            contract
                .normalize(&contract.example_args, "args")
                .unwrap_or_else(|error| panic!("{}: {error:?}", descriptor.name()));
            assert!(serde_json::to_vec(&contract.details()).unwrap().len() < 16 * 1024);
            count += 1;
        }
        assert_eq!(count, 53);
    }

    #[test]
    fn dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary() {
        let query = dcs_contract("query.patch").unwrap();
        assert_eq!(
            query
                .normalize(&json!({"values":{"find":"old"}}), "args")
                .unwrap(),
            json!({"values":{"find":"old","replace":"","once":false}})
        );
        let structure = dcs_contract("structure.set").unwrap();
        assert_eq!(
            structure.normalize(&json!({"values":{}}), "args").unwrap(),
            json!({"values":{"groupBy":[],"details":true}})
        );
        // Набор и вариант называет адрес: ключи, которые его дублировали,
        // контракт не принимает (П2, #1353).
        for (op, values, key) in [
            (
                "query.set",
                json!({"dataSet":"D","query":"ВЫБРАТЬ 1"}),
                "dataSet",
            ),
            ("query.patch", json!({"dataSet":"D","find":"1"}), "dataSet"),
            ("structure.set", json!({"variant":"V"}), "variant"),
            (
                "structure.patch",
                json!({"variant":"V","name":"G","groupBy":["A"]}),
                "variant",
            ),
            // Подузел называет адрес (П3, #1354).
            ("variant.set", json!({"name":"V","title":"T"}), "name"),
            ("parameter.set", json!({"name":"P","title":"T"}), "name"),
            (
                "parameter.rename",
                json!({"name":"P","newName":"Q"}),
                "name",
            ),
            ("field.set", json!({"field":"F","title":"T"}), "field"),
            ("field.set", json!({"dataPath":"F","title":"T"}), "dataPath"),
            (
                "fieldRole.set",
                json!({"field":"F","role":{"dimension":true}}),
                "field",
            ),
        ] {
            let error = dcs_contract(op)
                .unwrap()
                .normalize(&json!({ "values": values }), "args")
                .unwrap_err();
            assert_eq!(error.path, format!("args.values.{key}"), "{op}");
            assert_eq!(error.message, "unknown argument", "{op}");
        }
        let field = dcs_contract("field.add").unwrap();
        for args in [
            json!({"items":[{"name":"Alias"}]}),
            json!({"items":[{"dataPath":"Path","name":"Alias","title":""}]}),
            json!({"items":[{"dataPath":"Path","name":"","title":""}]}),
        ] {
            assert!(field.normalize(&args, "args").is_ok());
        }
        for args in [
            json!({"items":[{}]}),
            json!({"items":[{"name":"  \n"}]}),
            json!({"items":[{"name":7}]}),
            json!({"items":[{"dataPath":"","name":"Fallback"}]}),
        ] {
            assert!(field.normalize(&args, "args").is_err());
        }
        let role = dcs_contract("fieldRole.set").unwrap();
        for value in [
            json!({"dimension":true}),
            json!({"periodType":"Main"}),
            json!({"balanceType":"OpeningBalance"}),
            json!({"parentDimension":"Parent","accountField":"Account"}),
        ] {
            assert!(role
                .normalize(&json!({"values":{"role":value}}), "args")
                .is_ok());
        }
        assert!(role
            .normalize(&json!({"values":{"role":"dimension"}}), "args")
            .is_err());
        // Правка без свойств ничего не меняет и отвергается.
        for op in ["field.set", "parameter.set"] {
            assert!(
                dcs_contract(op)
                    .unwrap()
                    .normalize(&json!({"values":{}}), "args")
                    .is_err(),
                "{op}"
            );
        }
        assert!(dcs_contract("variant.remove")
            .unwrap()
            .normalize(&json!({"values":{"name":"V"}}), "args")
            .is_err());
    }

    #[test]
    fn dcs_link_removal_accepts_its_selector_and_refuses_patch_properties() {
        let remove = dcs_contract("dataSetLink.remove").unwrap();
        let set = dcs_contract("dataSetLink.set").unwrap();
        let selector = set.example_args["values"]["selector"].clone();
        let selected = json!({"values":{"selector":selector}});
        assert!(remove.normalize_dcs(&selected, "ops[4].args").is_ok());
        for (key, value) in [
            ("parameter", json!("Period")),
            ("parameterListAllowed", json!(true)),
            ("required", json!(false)),
            ("condition", json!("Amount > 0")),
            ("startExpression", json!("Amount")),
        ] {
            let mut args = selected.clone();
            args["values"][key] = value;
            assert!(set.normalize_dcs(&args, "ops[4].args").is_ok(), "{key}");
            assert!(
                remove.normalize_dcs(&args, "ops[4].args").is_err(),
                "link removal silently accepted patch property {key}"
            );
        }
    }

    #[test]
    fn dcs_structure_add_refuses_grouping_on_tables_and_charts() {
        let contract = dcs_contract("structure.add").unwrap();
        assert!(contract
            .normalize_dcs(&contract.example_args, "args")
            .is_ok());
        let mut default_group = contract.example_args.clone();
        default_group["items"][0]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        assert_eq!(
            contract.normalize_dcs(&default_group, "args").unwrap()["items"][0]["kind"],
            "group"
        );
        for kind in ["table", "chart"] {
            let mut args = contract.example_args.clone();
            args["items"][0]["kind"] = json!(kind);
            args["items"][0].as_object_mut().unwrap().remove("groupBy");
            assert!(contract.normalize_dcs(&args, "args").is_ok(), "{kind}");
            for group_by in [json!(["Amount"]), json!([])] {
                args["items"][0]["groupBy"] = group_by;
                assert!(
                    contract.normalize_dcs(&args, "args").is_err(),
                    "{kind} silently accepted groupBy"
                );
            }
        }
    }

    #[test]
    fn dcs_auto_order_refuses_explicit_direction_and_field_order_keeps_defaults() {
        let contract = dcs_contract("order.add").unwrap();
        let default_field = contract
            .normalize_dcs(&contract.example_args, "args")
            .unwrap();
        assert_eq!(default_field["items"][0]["kind"], "Field");
        assert_eq!(default_field["items"][0]["direction"], "Asc");
        let mut auto = json!({"items":[{"kind":"Auto"}]});
        let normalized_auto = contract.normalize_dcs(&auto, "args").unwrap();
        assert!(normalized_auto["items"][0].get("direction").is_none());
        for direction in ["Asc", "Desc"] {
            let mut field = contract.example_args.clone();
            field["items"][0]["direction"] = json!(direction);
            assert!(
                contract.normalize_dcs(&field, "args").is_ok(),
                "{direction}"
            );
            auto["items"][0]["direction"] = json!(direction);
            assert!(
                contract.normalize_dcs(&auto, "args").is_err(),
                "Auto order silently accepted {direction} direction"
            );
        }
    }

    #[test]
    fn dcs_value_type_requires_a_corresponding_value_or_list() {
        for (operation, args) in [
            (
                "parameter.add",
                json!({"items":[{"name":"Period","valueType":"xs:string"}]}),
            ),
            ("parameter.set", json!({"values":{"valueType":"xs:string"}})),
            (
                "filter.add",
                json!({"items":[{"field":"Amount","valueType":"xs:string"}]}),
            ),
            (
                "filter.set",
                json!({"values":{"field":"Amount","valueType":"xs:string"}}),
            ),
            (
                "dataParameter.set",
                json!({"values":{"name":"Period","valueType":"xs:string"}}),
            ),
            (
                "outputParameter.set",
                json!({"values":{"name":"Period","valueType":"xs:string"}}),
            ),
        ] {
            let contract = dcs_contract(operation).unwrap();
            assert!(contract
                .normalize_dcs(&contract.example_args, "args")
                .is_ok());
            let mut with_value = args.clone();
            let values = if with_value.get("items").is_some() {
                &mut with_value["items"][0]
            } else {
                &mut with_value["values"]
            };
            values["value"] = json!("literal");
            assert!(
                contract.normalize_dcs(&with_value, "args").is_ok(),
                "{operation}"
            );
            if operation.starts_with("parameter.") {
                for (key, list) in [
                    ("values", json!(["literal"])),
                    ("availableValues", json!([{"value":"literal"}])),
                    ("values", json!([])),
                    ("availableValues", json!([])),
                ] {
                    let mut with_list = args.clone();
                    let values = if with_list.get("items").is_some() {
                        &mut with_list["items"][0]
                    } else {
                        &mut with_list["values"]
                    };
                    values[key] = list;
                    assert!(
                        contract.normalize_dcs(&with_list, "args").is_ok(),
                        "{operation}.{key}"
                    );
                }
            }
            assert!(
                contract.normalize_dcs(&args, "args").is_err(),
                "{operation} silently accepted valueType without its value"
            );
        }
        let appearance = dcs_contract("conditionalAppearance.add").unwrap();
        let mut args = appearance.example_args.clone();
        args["items"][0]["filter"] =
            json!([{"field":"Amount","valueType":"xs:string","value":"literal"}]);
        assert!(appearance.normalize_dcs(&args, "args").is_ok());
        args["items"][0]["filter"][0]
            .as_object_mut()
            .unwrap()
            .remove("value");
        assert!(
            appearance.normalize_dcs(&args, "args").is_err(),
            "appearance filter silently accepted valueType without its value"
        );
    }
}
