//! Structural argument contracts and validation shared by operation owners.
//! Family schemas, capability rules and registrations belong to their producers.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// One immutable schema-format binding owned by the schema producer.
/// The generic engine admits the names before validating any argument values.
#[derive(Clone, Copy)]
pub struct FormatValidator {
    pub name: &'static str,
    pub validate: fn(value: &Value) -> Result<(), String>,
}

pub struct OperationContract {
    pub schema: Value,
    pub target: &'static str,
    pub effect: &'static str,
    pub notes: &'static str,
    pub example_args: Value,
    /// Plugin reference that explains the use case of the operation.
    pub help: &'static str,
    /// Immutable validation owned by the contract's schema producer.
    pub format_validators: &'static [FormatValidator],
}

#[derive(Debug, PartialEq)]
pub struct ContractError {
    pub path: String,
    pub message: String,
}

/// A borrowed part of an existing argument contract. Decoders visit it in
/// their established diagnostic order, rather than normalizing an entire
/// request before choosing its target or converting domain values.
#[derive(Clone, Copy)]
pub struct ArgumentShape<'a> {
    schema: &'a Value,
}

impl<'a> ArgumentShape<'a> {
    pub fn new(schema: &'a Value) -> Self {
        Self { schema }
    }

    pub fn property(self, name: &str) -> Option<Self> {
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

    pub fn requires(self, name: &str) -> bool {
        self.required_names().any(|key| key == name)
    }

    pub fn property_names(self) -> std::vec::IntoIter<&'a str> {
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

    pub fn aliases(self) -> impl Iterator<Item = &'a str> {
        self.schema["x-unica-aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    pub fn alias_groups(self) -> impl Iterator<Item = Vec<&'a str>> {
        self.schema["x-unica-alias-groups"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|group| {
                group
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect()
            })
    }

    pub fn minimum_properties(self) -> Option<u64> {
        self.schema["minProperties"].as_u64()
    }

    pub fn has_default(self) -> bool {
        self.schema.get("default").is_some()
    }

    pub fn required_names(self) -> impl Iterator<Item = &'a str> {
        self.schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
    }

    pub fn items(self) -> Option<Self> {
        self.schema.get("items").map(Self::new)
    }

    pub fn additional_properties(self) -> Option<Self> {
        self.schema
            .get("additionalProperties")
            .filter(|schema| schema.is_object())
            .map(Self::new)
    }

    pub fn variant(self, discriminator: &str, value: &str) -> Option<Self> {
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
    /// Compound domain invariants remain with their typed converters;
    /// this is deliberately not a full JSON Schema interpreter.
    pub fn validate_field(self, value: &Value, path: &str) -> Result<(), ContractError> {
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

    pub fn validate_type(self, value: &Value, path: &str) -> Result<(), ContractError> {
        let schema = json!({"type": self.schema["type"]});
        // Existing typed numeric conversion accepts integral JSON
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

pub fn text(nonempty: bool) -> Value {
    if nonempty {
        json!({"type":"string", "pattern":r"\S"})
    } else {
        json!({"type":"string"})
    }
}

pub fn object(fields: &[(&str, Value)], required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false,
        "properties":fields.iter().map(|(key,value)| (key.to_string(),value.clone())).collect::<Map<_,_>>(),
        "required":required})
}

impl OperationContract {
    pub fn normalize(&self, args: &Value, path: &str) -> Result<Value, ContractError> {
        let validator = Validator {
            format_validators: self.format_validators,
        };
        validator.admit_formats(&self.schema, path)?;
        validator.validate(&self.schema, args, path)?;
        let mut args = args.clone();
        apply_defaults(&self.schema, &mut args);
        normalize_aliases(&self.schema, &mut args);
        Ok(args)
    }

    pub fn details(&self) -> Value {
        json!({"argsSchema":published_schema(&self.schema),"target":self.target,"effect":self.effect,
            "notes":self.notes,"exampleArgs":self.example_args,
            "exampleUse":"Read existing targets and required names through view. For creation, choose a new unused name. These argument examples do not establish that their names exist in this workspace.",
            "help":{"scope":"plugin","path":self.help},
            "execution":"Preview with at/ops; execute only data.executionToken from the successful plan, then read the post-image and check. Observe the same Task to terminal state; never repeat a mutation to wait for it."})
    }
}

/// Publish one spelling per declared property group, keeping the acceptance
/// schema intact for legacy callers and diagnostic provenance. Only schema
/// positions are visited: enum/default/example objects are user values.
fn published_schema(schema: &Value) -> Value {
    let mut published = schema.clone();
    project_schema(&mut published);
    published
}

fn project_schema(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_object_mut) {
            for child in children.values_mut() {
                project_schema(child);
            }
        }
    }
    for key in [
        "items",
        "additionalItems",
        "additionalProperties",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "propertyNames",
    ] {
        if let Some(child) = object.get_mut(key) {
            if let Some(children) = child.as_array_mut() {
                for child in children {
                    project_schema(child);
                }
            } else {
                project_schema(child);
            }
        }
    }
    for key in ["oneOf", "anyOf", "allOf", "prefixItems"] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_array_mut) {
            for child in children {
                project_schema(child);
            }
        }
    }
    let Some(groups) = object
        .get("x-unica-alias-groups")
        .and_then(Value::as_array)
        .cloned()
    else {
        return;
    };
    let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    // Do not invent a canonical shape for an unrecognised declaration.
    let mut aliases = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for group in &groups {
        let Some(names) = group.as_array() else {
            return;
        };
        if names.len() < 2 {
            return;
        }
        let Some(canonical) = names[0].as_str().and_then(|name| properties.get(name)) else {
            return;
        };
        for (index, name) in names.iter().enumerate() {
            let Some(name) = name.as_str() else {
                return;
            };
            if !seen.insert(name) || properties.get(name) != Some(canonical) {
                return;
            }
            if index > 0 {
                aliases.push(name.to_owned());
            }
        }
    }
    for alias in aliases {
        properties.remove(&alias);
    }
    // This is the exact collision constraint emitted with alias groups.
    // Other `not` constraints remain authoritative and are never discarded.
    let collisions =
        json!({"anyOf": groups.iter().map(|names| json!({"required":names})).collect::<Vec<_>>()});
    if object.get("not") == Some(&collisions) {
        object.remove("not");
    }
    object.remove("x-unica-alias-groups");
}

/// Compiled grammar of an owned contract pattern. Every pattern comes from a
/// schema supplied by an operation owner; runtime arguments never select a pattern.
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

struct Validator<'a> {
    format_validators: &'a [FormatValidator],
}

impl Validator<'_> {
    fn binding(&self, name: &str, path: &str) -> Result<&FormatValidator, ContractError> {
        let mut matches = self
            .format_validators
            .iter()
            .filter(|binding| binding.name == name);
        let binding = matches
            .next()
            .ok_or_else(|| error(path, format!("argument format has no validator: {name}")))?;
        if matches.next().is_some() {
            return Err(error(
                path,
                format!("argument format has multiple validators: {name}"),
            ));
        }
        Ok(binding)
    }

    fn admit_formats(&self, schema: &Value, path: &str) -> Result<(), ContractError> {
        if let Some(name) = schema["format"].as_str() {
            self.binding(name, path)?;
        }
        if let Some(properties) = schema["properties"].as_object() {
            for (name, child) in properties {
                self.admit_formats(child, &format!("{path}.{name}"))?;
            }
        }
        for key in ["not", "items", "additionalProperties", "propertyNames"] {
            if let Some(child) = schema.get(key).filter(|child| child.is_object()) {
                self.admit_formats(child, path)?;
            }
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(variants) = schema[key].as_array() {
                for variant in variants {
                    self.admit_formats(variant, path)?;
                }
            }
        }
        Ok(())
    }

    fn validate(&self, schema: &Value, value: &Value, path: &str) -> Result<(), ContractError> {
        if schema
            .get("not")
            .is_some_and(|excluded| self.validate(excluded, value, path).is_ok())
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
        if let Some(format) = schema["format"].as_str() {
            let binding = self.binding(format, path)?;
            (binding.validate)(value).map_err(|message| error(path, message))?;
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
                Some(variant) => self.validate(variant, value, path),
                None => Err(error(
                    &format!("{path}.{key}"),
                    "value is not one of the supported alternatives",
                )),
            };
        }
        if let Some(alternatives) = schema["anyOf"].as_array() {
            if !alternatives
                .iter()
                .any(|choice| self.validate(choice, value, path).is_ok())
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
                    self.validate(names, &Value::String(key.clone()), &format!("{path}.{key}"))?;
                }
            }
            if schema["additionalProperties"].is_object() {
                for (key, value) in object {
                    if schema["properties"].get(key).is_none() {
                        self.validate(
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
                        Some(child) => self.validate(child, value, &format!("{path}.{key}"))?,
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
                self.validate(items, value, &format!("{path}[{index}]"))?;
            }
        }
        Ok(())
    }
}

fn validate(schema: &Value, value: &Value, path: &str) -> Result<(), ContractError> {
    let validator = Validator {
        format_validators: &[],
    };
    validator.admit_formats(schema, path)?;
    validator.validate(schema, value, path)
}

pub fn apply_defaults(schema: &Value, value: &mut Value) {
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
                apply_defaults(child, value);
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array_mut()) {
        for value in values {
            apply_defaults(items, value);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationSkeleton {
    From,
    Items,
    Values,
    Text,
    Target,
}

impl OperationSkeleton {
    /// The single root argument key the operation expects; the published
    /// `can` dictionary and refusal texts both speak this name.
    pub const fn key(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::Items => "items",
            Self::Values => "values",
            Self::Text => "text",
            Self::Target => "at",
        }
    }

    pub fn args(self) -> Map<String, Value> {
        let value = match self {
            Self::From => Value::String(String::new()),
            Self::Items => Value::Array(Vec::new()),
            Self::Values => Value::Object(Map::new()),
            Self::Text => Value::String(String::new()),
            Self::Target => return Map::new(),
        };
        Map::from_iter([(self.key().to_string(), value)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contract(schema: Value, format_validators: &'static [FormatValidator]) -> OperationContract {
        OperationContract {
            schema,
            target: "target",
            effect: "effect",
            notes: "notes",
            example_args: json!({}),
            help: "help",
            format_validators,
        }
    }

    #[test]
    fn public_alias_projection_preserves_runtime_and_non_schema_values() {
        let group = json!({"type":"object", "additionalProperties":false,
            "properties":{"Mode":{"type":"string","enum":["A","B"]},"mode":{"type":"string","enum":["A","B"]}},
            "x-unica-alias-groups":[["Mode","mode"]],
            "not":{"anyOf":[{"required":["Mode","mode"]}]}});
        let schema = json!({"type":"object", "properties":{
            "values":group,
            "items":{"type":"array","items":{"oneOf":[group]}},
            "literal":{"enum":[group],"default":group,"examples":[group]}
        }});
        let contract = contract(schema.clone(), &[]);
        let published = contract.details()["argsSchema"].clone();
        for node in [
            &published["properties"]["values"],
            &published["properties"]["items"]["items"]["oneOf"][0],
        ] {
            assert_eq!(
                node["properties"],
                json!({"Mode":{"type":"string","enum":["A","B"]}})
            );
            assert!(node.get("not").is_none());
            assert!(node.get("x-unica-alias-groups").is_none());
        }
        assert_eq!(
            published["properties"]["literal"],
            schema["properties"]["literal"]
        );
        assert_eq!(contract.schema, schema);
        for name in ["Mode", "mode"] {
            let args = json!({"values":{name:"A"}});
            assert_eq!(
                contract.normalize(&args, "args").unwrap(),
                json!({"values":{name:"A"}, "literal":group})
            );
            assert!(contract
                .normalize(&json!({"values":{name:"invalid"}}), "args")
                .is_err());
        }
        assert!(contract
            .normalize(&json!({"values":{"Mode":"A","mode":"A"}}), "args")
            .is_err());
    }

    #[test]
    fn public_projection_keeps_other_constraints_and_dcs_alias_metadata() {
        let schema = json!({"type":"object", "properties":{
            "Mode":{"type":"string","enum":["A","B"]},
            "mode":{"type":"string","enum":["A","B"]},
            "Other":{"type":"string","enum":["A","B"]}},
            "x-unica-alias-groups":[["Mode","mode"]],
            "x-unica-aliases":["Mode","mode"],
            "not":{"anyOf":[{"required":["Mode","mode"]},{"required":["Mode","Other"]}]}});
        let published = contract(schema.clone(), &[]).details()["argsSchema"].clone();
        assert_eq!(published["not"], schema["not"]);
        assert_eq!(published["x-unica-aliases"], schema["x-unica-aliases"]);
        assert_eq!(
            published["properties"]["Other"],
            schema["properties"]["Other"]
        );
    }

    fn lower_word(value: &Value) -> Result<(), String> {
        value
            .as_str()
            .filter(|word| word.chars().all(|c| c.is_ascii_lowercase()))
            .map(|_| ())
            .ok_or_else(|| "expected a lower-case word".into())
    }

    fn upper_word(value: &Value) -> Result<(), String> {
        value
            .as_str()
            .filter(|word| word.chars().all(|c| c.is_ascii_uppercase()))
            .map(|_| ())
            .ok_or_else(|| "expected an upper-case word".into())
    }

    const LOWER: &[FormatValidator] = &[FormatValidator {
        name: "test-word",
        validate: lower_word,
    }];
    const UPPER: &[FormatValidator] = &[FormatValidator {
        name: "test-word",
        validate: upper_word,
    }];

    #[test]
    fn format_validation_stays_with_each_contract_and_reaches_nested_keys() {
        let schema = json!({"type":"object","properties":{"items":{"type":"array","items":{
            "type":"object","propertyNames":{"type":"string","format":"test-word"}
        }}}});
        let lower = contract(schema.clone(), LOWER);
        let upper = contract(schema, UPPER);
        let input = json!({"items":[{"lower":true}]});
        for _ in 0..2 {
            assert_eq!(lower.normalize(&input, "ops[2].args").unwrap(), input);
            assert_eq!(
                upper.normalize(&input, "ops[2].args").unwrap_err(),
                ContractError {
                    path: "ops[2].args.items[0].lower".into(),
                    message: "expected an upper-case word".into(),
                }
            );
        }
    }

    #[test]
    fn declared_formats_refuse_without_an_owner_or_when_unknown_to_the_owner() {
        let schema = json!({"type":"string","format":"test-unknown"});
        assert_eq!(
            contract(schema.clone(), &[])
                .normalize(&json!("value"), "args")
                .unwrap_err(),
            ContractError {
                path: "args".into(),
                message: "argument format has no validator: test-unknown".into(),
            }
        );
        assert_eq!(
            contract(schema, LOWER)
                .normalize(&json!("value"), "args")
                .unwrap_err(),
            ContractError {
                path: "args".into(),
                message: "argument format has no validator: test-unknown".into(),
            }
        );
    }

    #[test]
    fn unbound_formats_cannot_hide_in_negation_alternatives_or_omitted_properties() {
        for schema in [
            json!({"not":{"format":"test-unknown"}}),
            json!({"anyOf":[{"type":"string"},{"format":"test-unknown"}]}),
            json!({"oneOf":[{"type":"string"},{"format":"test-unknown"}]}),
            json!({"properties":{"optional":{"format":"test-unknown"}}}),
        ] {
            for bindings in [&[][..], LOWER] {
                let error = contract(schema.clone(), bindings)
                    .normalize(&json!("value"), "args")
                    .unwrap_err();
                assert_eq!(
                    error.message,
                    "argument format has no validator: test-unknown"
                );
            }
        }
    }

    #[test]
    fn a_format_name_cannot_have_two_competing_validators() {
        const DUPLICATE: &[FormatValidator] = &[
            FormatValidator {
                name: "test-word",
                validate: lower_word,
            },
            FormatValidator {
                name: "test-word",
                validate: upper_word,
            },
        ];
        let error = contract(json!({"format":"test-word"}), DUPLICATE)
            .normalize(&json!("value"), "args")
            .unwrap_err();
        assert_eq!(
            error.message,
            "argument format has multiple validators: test-word"
        );
    }

    #[test]
    fn primitive_type_and_minimum_errors_keep_priority_over_owned_formats() {
        let text_contract = contract(json!({"type":"string","format":"test-word"}), LOWER);
        assert_eq!(
            text_contract
                .normalize(&json!(7), "args")
                .unwrap_err()
                .message,
            "expected \"string\""
        );
        let number_contract = contract(
            json!({"type":"integer","minimum":2,"format":"test-word"}),
            LOWER,
        );
        assert_eq!(
            number_contract
                .normalize(&json!(1), "args")
                .unwrap_err()
                .message,
            "integer is below the supported minimum"
        );
    }

    #[test]
    fn nested_defaults_and_aliases_keep_the_existing_normalization_order() {
        let schema = json!({"type":"object","properties":{"items":{"type":"array","items":{
            "type":"object","x-unica-aliases":["field","legacy"],
            "properties":{"field":{"type":"string"},"legacy":{"type":"string"},"use":{"type":"boolean","default":true}}
        }}}});
        assert_eq!(
            contract(schema, &[])
                .normalize(&json!({"items":[{"legacy":"x"}]}), "args")
                .unwrap(),
            json!({"items":[{"field":"x","use":true}]})
        );
    }

    #[test]
    fn typed_shapes_keep_numeric_and_compound_checks_with_the_typed_converter() {
        let integer = json!({"type":"integer"});
        assert!(ArgumentShape::new(&integer)
            .validate_type(&json!(1.0), "args")
            .is_ok());
        assert!(contract(integer, &[])
            .normalize(&json!(1.0), "args")
            .is_err());
        let object =
            json!({"type":"object","required":["value"],"properties":{"value":{"type":"string"}}});
        assert!(ArgumentShape::new(&object)
            .validate_field(&json!({}), "args")
            .is_ok());
        assert_eq!(
            contract(object, &[])
                .normalize(&json!({}), "args")
                .unwrap_err()
                .path,
            "args.value"
        );
    }

    /// Скомпилированная грамматика берётся из кеша по своему шаблону: повторная
    /// проверка не подменяет одну грамматику другой (#1441).
    #[test]
    fn a_contract_pattern_keeps_its_own_grammar_on_repeated_validation() {
        let identifier =
            json!({"type": "string", "pattern": r"^[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*$"});
        let non_blank = json!({"type": "string", "pattern": r"\S"});
        for _ in 0..2 {
            assert!(validate(&identifier, &json!("Реквизит_1"), "args").is_ok());
            assert!(validate(&identifier, &json!("1Реквизит"), "args").is_err());
            assert!(validate(&non_blank, &json!("1Реквизит"), "args").is_ok());
            assert!(validate(&non_blank, &json!(" "), "args").is_err());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OperationRef {
    op: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    args: Map<String, Value>,
}

impl OperationRef {
    pub fn new(op: impl Into<String>, args: Map<String, Value>) -> Self {
        Self {
            op: op.into(),
            args,
        }
    }

    pub fn op(&self) -> &str {
        &self.op
    }
}

pub mod input;

pub mod collections;
