//! Argument contracts shared by the operation hint and the apply boundary.
//! The closed operation registry owns names and applicability; this module
//! describes and normalizes the arguments consumed by supported planners.

use serde_json::{json, Map, Value};

pub(crate) const DCS_ROLE_FLAGS: &[&str] = &[
    "period",
    "dimension",
    "account",
    "balance",
    "ignoreNullValues",
    "required",
    "dimensionAttribute",
];
const DCS_ROLE_KEYS: &[&str] = &[
    "parentDimension",
    "accountTypeExpression",
    "balanceGroupName",
    "accountField",
    "periodNumber",
    "periodType",
    "dimension",
    "account",
    "balance",
    "ignoreNullValues",
    "required",
    "dimensionAttribute",
    "balanceType",
    "accountingBalanceType",
];

pub(crate) struct OperationContract {
    pub(crate) schema: Value,
    pub(crate) target: &'static str,
    pub(crate) effect: &'static str,
    pub(crate) notes: &'static str,
    pub(crate) example_args: Value,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ContractError {
    pub(crate) path: String,
    pub(crate) message: String,
}

fn text(nonempty: bool) -> Value {
    if nonempty {
        json!({"type":"string", "pattern":r"\S"})
    } else {
        json!({"type":"string"})
    }
}

fn object(fields: &[(&str, Value)], required: &[&str]) -> Value {
    json!({"type":"object", "additionalProperties":false,
        "properties":fields.iter().map(|(key,value)| (key.to_string(),value.clone())).collect::<Map<_,_>>(),
        "required":required})
}

fn aliases(mut schema: Value, names: &[&str]) -> Value {
    schema["x-unica-aliases"] = json!(names);
    for name in names {
        schema["properties"][*name] = text(false);
    }
    schema["anyOf"] = Value::Array(
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut choice = json!({"required":[name],"properties":{*name:text(true)}});
                if index > 0 {
                    choice["not"] = json!({"anyOf":names[..index].iter().map(|previous| json!({"required":[previous]})).collect::<Vec<_>>()});
                }
                choice
            })
            .collect(),
    );
    schema
}

fn optional_aliases(mut schema: Value, names: &[&str]) -> Value {
    schema["x-unica-aliases"] = json!(names);
    schema
}

fn items(item: Value) -> Value {
    json!({"type":"array", "items":item})
}

fn field_head(names: &[&str], required: &[&str]) -> Value {
    let mut fields: Vec<_> = names.iter().map(|name| (*name, text(true))).collect();
    fields.extend([("type", text(false)), ("title", text(false))]);
    aliases(object(&fields, required), names)
}

impl OperationContract {
    pub(crate) fn dcs(op: &str) -> Option<Self> {
        let data = "Existing Template.DataSet.<name> (Query.<name> alias also selects the dataset). Read DataSet and Field names from view; the Template itself is not a DCS apply target.";
        let setting = "Existing Template.Setting.<name>. Read Setting and its Item collection; do not assume the variant is named Основной.";
        let parameter = "A DCS DataSet or Setting; parameter.remove ends with Parameter.<name>.";
        let head_example = json!({"items":[{"dataPath":"Added","title":"Added"}]});
        let (key, inner, target, effect, notes, example_args) = match op {
            "field.add" => ("items", items(field_head(&["dataPath","name"], &[])), data,
                "Add fields; duplicate dataPath is skipped by the editor. Inspect the post-image to establish the effect.",
                "The first supplied alias wins: dataPath, then name; it must not be empty. Optional empty type/title are ignored. Values are typed fields, not legacy shorthand flags.", head_example),
            "field.set" => ("values", field_head(&["field","dataPath","name"], &[]), data,
                "Update an existing field's supplied title; explicit type is written only for a non-query dataset. Query fields derive their type from the query, so an old valueType is removed.",
                "Field alias precedence: first supplied field, dataPath, name; the selected alias must not be empty. Read the existing Field collection before choosing the name. Other field properties are preserved.", json!({"values":{"field":"Amount","title":"Amount"}})),
            "fieldRole.set" => {
                let role = json!({"type":"string","anyOf":[{"enum":DCS_ROLE_FLAGS},
                    {"pattern":format!(r"^({0})=\S+(?:\s+({0})=\S+)*$",DCS_ROLE_KEYS.join("|"))}]});
                let inner = aliases(object(&[("field",text(true)),("dataPath",text(true)),
                    ("name",text(true)),("role",role)], &["role"]), &["field","dataPath","name"]);
                ("values",inner,data,"Replace the existing field role with only the supplied role entries; previous entries are removed.",
                    "The field uses alias precedence field, dataPath, name. role accepts one bare supported flag or whitespace-separated key=value entries. Copy any existing role entries that must remain. The editor validates values against the platform XSD; flags in the field name are not accepted.",
                    json!({"values":{"field":"Amount","role":"dimension"}}))
            }
            "parameter.add" | "parameter.set" => {
                let inner = object(&[("name",text(true)),("type",text(false)),("title",text(false)),("value",text(false))], &["name"]);
                let (key, inner, example) = if op == "parameter.add" {
                    ("items",items(inner),json!({"items":[{"name":"Period","type":"date"}]}))
                } else {
                    ("values",inner,json!({"values":{"name":"Period","title":"Period"}}))
                };
                (key,inner,parameter,"Add or update a schema parameter; inspect the Parameter collection after execution.",
                    "value is a string, not arbitrary JSON; empty optional strings are ignored. Hidden/autoDates/availableValues flags of the old DSL are not supported by these typed arguments.",example)
            }
            "filter.add" => {
                let comparison = json!({"type":"string","default":"Equal","enum":[
                    "Equal","NotEqual","Greater","GreaterOrEqual","Less","LessOrEqual",
                    "Contains","NotContains","BeginsWith","NotBeginsWith","InList","NotInList",
                    "InHierarchy","InListByHierarchy","Filled","NotFilled"]});
                ("items",items(aliases(object(&[("field",text(true)),("dataPath",text(true)),
                    ("comparison",comparison),("value",json!({}))], &[]), &["field","dataPath"])),setting,
                    "Add variant filters, preserving existing filters.",
                    "The first supplied alias wins: field, then dataPath; it must not be empty. Omitted comparison is Equal. Omitted value is empty text; a JSON value is serialized by the editor. This operation does not expose the legacy group/user-setting DSL.",
                    json!({"items":[{"field":"Amount","comparison":"Greater","value":0}]}))
            }
            "selection.add" => ("items",items(aliases(object(&[("field",text(true)),("dataPath",text(true))], &[]), &["field","dataPath"])),setting,
                "Add selected fields to the variant.","field precedes dataPath; read dataset fields before selecting.",json!({"items":[{"field":"Amount"}]})),
            "calculatedField.add" => {
                let mut inner = field_head(&["name","dataPath"], &[]);
                inner["properties"]["expression"] = text(true);
                inner["required"] = json!(["expression"]);
                ("items",items(inner),data,"Add schema calculated fields.",
                    "The first supplied alias wins: name, then dataPath; it must not be empty. expression is literal expression text; legacy restriction flags are not typed fields.",
                    json!({"items":[{"name":"DoubleAmount","expression":"Amount * 2"}]}))
            }
            "total.add" => ("items",items(aliases(object(&[("field",text(true)),("dataPath",text(true)),("expression",text(false))], &[]), &["field","dataPath"])),setting,
                "Add schema totals.","The first supplied alias wins: field, then dataPath; it must not be empty. Omitted or empty expression is Сумма(<field>); grouping associations are not exposed by this operation.",json!({"items":[{"field":"Amount","expression":"Сумма(Amount)"}]})),
            "variant.add" => ("items",items(optional_aliases(object(&[("name",text(true)),("title",text(false)),("presentation",text(false))], &["name"]), &["title","presentation"])),setting,
                "Add a settings variant.","title precedes presentation; optional empty titles are ignored.",json!({"items":[{"name":"Additional","title":"Additional"}]})),
            "query.set" => ("values",aliases(object(&[("dataSet",text(false)),("query",text(true)),("text",text(true))], &[]), &["query","text"]),data,
                "Replace the existing dataset query with supplied text.",
                "The first supplied alias wins: query, then text; it must not be empty. dataSet overrides the dataset selected by the address. Supply text itself; @file is refused. A new template has no DataSet and cannot be populated by query.set.",
                json!({"values":{"query":"ВЫБРАТЬ 1 КАК Amount"}})),
            "query.patch" => ("values",object(&[("dataSet",text(false)),("find",text(true)),
                ("replace",json!({"type":"string","default":""})),("once",json!({"type":"boolean","default":false}))], &["find"]),data,
                "Replace matching query text; no matches fail. once=true requires exactly one match.",
                "Empty replace deletes the matching text. dataSet overrides the address dataset. find/replace are text; editor control tokens ' => ' and @once are refused.",
                json!({"values":{"find":"1 КАК Amount","replace":"2 КАК Amount","once":true}})),
            "structure.set" | "structure.patch" => ("values",object(&[("variant",text(false)),
                ("groupBy",json!({"type":"array","items":{"type":"string","pattern":r"\S"},"default":[]})),
                ("details",json!({"type":"boolean","default":true}))], &[]),setting,
                if op == "structure.set" {"Replace variant grouping structure."} else {"Change the grouping fields of existing named groups; an unknown or ambiguous group must fail without publication."},
                "variant overrides the address variant; if neither supplies a name, the editor uses Основной (not the first variant). groupBy=[] creates details even with details=false. For patch, read Setting.Item and use a real unique group name in 'Amount @name=ExistingGroup'; warning/skip is not evidence of a change. Patch preserves other groups.",
                if op == "structure.set" {json!({"values":{"groupBy":["Amount"],"details":true}})} else {json!({"values":{"groupBy":["Amount @name=ExistingGroup"],"details":false}})}),
            "field.remove" | "parameter.remove" => ("",json!({}),
                if op == "field.remove" {"A named existing DataSet.Field.<name>."} else {"A named existing DCS Parameter.<name>."},
                "Remove the addressed field or parameter.","The terminal address supplies the name; no values/items payload is accepted.",json!({})),
            "filter.clear" | "selection.clear" | "order.clear" | "conditionalAppearance.clear" => ("",json!({}),setting,
                "Clear the entire corresponding variant collection.",
                "This removes all items, not one chosen item. No values/items payload is accepted; read the result after saved-plan execution.",json!({})),
            _ => return None,
        };
        let mut fields = vec![("at", text(true))];
        let required = if key.is_empty() {
            vec![]
        } else {
            fields.push((key, inner));
            vec![key]
        };
        Some(Self {
            schema: object(&fields, &required),
            target,
            effect,
            notes,
            example_args,
        })
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
            "exampleUse":"Choose existing names from view; this is an argument example, not a claim that its example fields or group already exist in this workspace.",
            "help":{"scope":"plugin","path":"references/use-cases/reports-printing.md"},
            "execution":"Preview with at/ops; execute only data.executionToken from the successful plan, then read the post-image and check. Observe the same Task to terminal state; never repeat a mutation to wait for it."})
    }
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
        None => true,
        other => unreachable!("unsupported internal schema type: {other:?}"),
    };
    if !valid_type {
        return Err(error(path, format!("expected {}", schema["type"])));
    }
    if let Some(pattern) = schema["pattern"].as_str() {
        if !value.as_str().is_some_and(|text| {
            regex::Regex::new(pattern)
                .expect("owned contract pattern")
                .is_match(text)
        }) {
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
        assert_eq!(count, 20);
    }

    #[test]
    fn dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary() {
        let query = OperationContract::dcs("query.patch").unwrap();
        assert_eq!(
            query
                .normalize(&json!({"values":{"find":"old"}}), "args")
                .unwrap(),
            json!({"values":{"find":"old","replace":"","once":false}})
        );
        let structure = OperationContract::dcs("structure.patch").unwrap();
        assert_eq!(
            structure.normalize(&json!({"values":{}}), "args").unwrap(),
            json!({"values":{"groupBy":[],"details":true}})
        );
        let field = OperationContract::dcs("field.add").unwrap();
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
        let role = OperationContract::dcs("fieldRole.set").unwrap();
        for value in [
            "dimension",
            "period",
            "dimension=false",
            "periodType=Main",
            "balanceType=OpeningBalance",
            "parentDimension=Parent",
            "accountTypeExpression=Type",
            "balanceGroupName=Amount",
            "accountField=Account",
            "dimension=true parentDimension=Parent accountField=Account",
        ] {
            assert!(
                role.normalize(&json!({"values":{"field":"Amount","role":value}}), "args")
                    .is_ok(),
                "{value}"
            );
        }
        for value in [
            "@dimension",
            "dimension autoOrder",
            "unknown",
            "unknown=true",
            "dimension=",
            "dimension=true extra",
        ] {
            assert!(
                role.normalize(&json!({"values":{"field":"Amount","role":value}}), "args")
                    .is_err(),
                "{value}"
            );
        }
    }
}
