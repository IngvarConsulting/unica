//! Convert the addressed property profile into a descriptor-specific intent.

use super::metadata::{json_u32, parse_property_changes};
use crate::domain::metadata::{
    MetaPropertyChanges, MetaPropertyValueKind, METADATA_PROPERTY_SPECS,
};
use crate::domain::operation_contract::{ArgumentShape, ContractError, OperationContract};
use crate::domain::properties_operation::{
    DescriptorProperty, DescriptorPropertyKind, PropertiesOperation, PropertiesSite,
};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RolePropertyChanges {
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubsystemPropertyChanges {
    pub(crate) synonym: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) explanation: Option<String>,
    pub(crate) include_help_in_contents: Option<bool>,
    pub(crate) include_in_command_interface: Option<bool>,
    pub(crate) use_one_command: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PropertiesIntent {
    Metadata(MetaPropertyChanges),
    Role(RolePropertyChanges),
    Subsystem(SubsystemPropertyChanges),
}

type DecodeResult<T> = Result<T, ContractError>;

fn invalid(path: impl Into<String>, message: impl Into<String>) -> ContractError {
    ContractError {
        path: path.into(),
        message: message.into(),
    }
}

struct PropertyArgumentReader<'a> {
    object: &'a Map<String, Value>,
    shape: ArgumentShape<'a>,
    path: String,
    consumed: HashSet<String>,
    consumed_groups: HashSet<String>,
}

impl<'a> PropertyArgumentReader<'a> {
    fn new(value: &'a Value, shape: ArgumentShape<'a>, path: &str) -> DecodeResult<Self> {
        shape.validate_type(value, path)?;
        Ok(Self {
            object: value
                .as_object()
                .ok_or_else(|| invalid(path, "expected an argument object"))?,
            shape,
            path: path.into(),
            consumed: HashSet::new(),
            consumed_groups: HashSet::new(),
        })
    }

    fn field(&mut self, name: &str, unsigned: bool) -> DecodeResult<Option<&'a Value>> {
        let path = format!("{}.{name}", self.path);
        let shape = self
            .shape
            .property(name)
            .ok_or_else(|| invalid(&path, "domain field has no declared argument shape"))?;
        self.consumed.insert(name.into());
        if let Some(value) = self.object.get(name) {
            if unsigned {
                // The existing u32 converter preserves 0/0.0/0e0. Validate
                // this one scalar's shape using the converted integer, while
                // keeping the original input for Metadata's typed converter.
                let number = json_u32(value).ok_or_else(|| {
                    invalid(&path, "property value must be an unsigned 32-bit integer")
                })?;
                shape.validate_field(&Value::from(number), &path)?;
            } else {
                shape.validate_field(value, &path)?;
            }
            Ok(Some(value))
        } else {
            if shape.has_default() {
                return Err(invalid(&path, "declared default has no domain decoder"));
            }
            Ok(None)
        }
    }

    fn alias(&mut self, field: DescriptorProperty) -> DecodeResult<Option<(&'a Value, String)>> {
        let names = field.names();
        let canonical = names[0];
        let group = self
            .shape
            .alias_groups()
            .find(|group| group.contains(&canonical));
        if group.as_deref() != Some(names) {
            return Err(invalid(
                format!("{}.{canonical}", self.path),
                "property alias group disagrees with its domain decoder",
            ));
        }
        self.consumed_groups.insert(canonical.into());
        if names
            .iter()
            .filter(|name| self.object.contains_key(**name))
            .count()
            > 1
        {
            return Err(invalid(
                format!("{}.{}", self.path, names[1]),
                format!(
                    "property `{canonical}` takes one spelling; choose either `{}` or `{}`",
                    names[0], names[1]
                ),
            ));
        }
        let mut selected = None;
        for name in names {
            if let Some(value) = self.field(name, false)? {
                selected = Some((value, format!("{}.{name}", self.path)));
            }
        }
        Ok(selected)
    }

    fn string(&mut self, field: DescriptorProperty) -> DecodeResult<Option<String>> {
        if field.kind() != DescriptorPropertyKind::String {
            return Err(invalid(
                &self.path,
                "property kind disagrees with its string decoder",
            ));
        }
        self.alias(field)?
            .map(|(value, path)| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| invalid(path, "property value must be a string"))
            })
            .transpose()
    }

    fn boolean(&mut self, field: DescriptorProperty) -> DecodeResult<Option<bool>> {
        if field.kind() != DescriptorPropertyKind::Boolean {
            return Err(invalid(
                &self.path,
                "property kind disagrees with its boolean decoder",
            ));
        }
        self.alias(field)?
            .map(|(value, path)| {
                value
                    .as_bool()
                    .ok_or_else(|| invalid(path, "property value must be a boolean"))
            })
            .transpose()
    }

    fn finish<T>(self, decoded: T) -> DecodeResult<T> {
        for name in self.object.keys() {
            if self.shape.property(name).is_none() {
                return Err(invalid(
                    format!("{}.{name}", self.path),
                    format!("unknown field `{name}`"),
                ));
            }
        }
        for name in self.shape.property_names() {
            if !self.consumed.contains(name) {
                return Err(invalid(
                    format!("{}.{name}", self.path),
                    "declared argument has no domain decoder",
                ));
            }
        }
        for name in self.shape.required_names() {
            if !self.object.contains_key(name) {
                return Err(invalid(
                    format!("{}.{name}", self.path),
                    format!("`{name}` is required"),
                ));
            }
        }
        for group in self.shape.alias_groups() {
            if group
                .first()
                .is_none_or(|name| !self.consumed_groups.contains(*name))
            {
                return Err(invalid(
                    &self.path,
                    "declared alias group has no domain decoder",
                ));
            }
        }
        if let Some(minimum) = self.shape.minimum_properties() {
            if (self.object.len() as u64) < minimum {
                return Err(invalid(&self.path, "property changes must not be empty"));
            }
        }
        Ok(decoded)
    }
}

pub(crate) fn decode_properties(
    operation: PropertiesOperation,
    site: PropertiesSite,
    args: &Value,
    path: &str,
) -> DecodeResult<PropertiesIntent> {
    let contract = operation.contract(site);
    decode_with_contract(operation, site, args, path, &contract)
}

fn decode_with_contract(
    operation: PropertiesOperation,
    site: PropertiesSite,
    args: &Value,
    path: &str,
    contract: &OperationContract,
) -> DecodeResult<PropertiesIntent> {
    match operation {
        PropertiesOperation::Set => {
            let mut reader =
                PropertyArgumentReader::new(args, ArgumentShape::new(&contract.schema), path)?;
            let values_path = format!("{path}.values");
            let values = reader
                .field("values", false)?
                .ok_or_else(|| invalid(&values_path, "values must be an object"))?;
            let shape = reader
                .shape
                .property("values")
                .expect("declared values field");
            let mut values_reader = PropertyArgumentReader::new(values, shape, &values_path)?;
            if values_reader.object.is_empty() {
                return Err(invalid(&values_path, "property changes must not be empty"));
            }
            let intent = match site {
                PropertiesSite::Metadata(kind) => {
                    // Preserve Metadata's existing conversion diagnostics and
                    // structured Type model, then verify the shared shape.
                    let changes =
                        parse_property_changes(values_reader.object, kind).map_err(|error| {
                            invalid(
                                format!("{path}.{}", error.field.as_deref().unwrap_or("values")),
                                error.message,
                            )
                        })?;
                    for spec in METADATA_PROPERTY_SPECS
                        .iter()
                        .filter(|spec| spec.allowed_kinds.contains(&kind))
                    {
                        values_reader.field(
                            spec.public_name,
                            spec.value_kind == MetaPropertyValueKind::UnsignedInteger,
                        )?;
                    }
                    PropertiesIntent::Metadata(changes)
                }
                PropertiesSite::Role => PropertiesIntent::Role(RolePropertyChanges {
                    synonym: values_reader.string(DescriptorProperty::Synonym)?,
                    comment: values_reader.string(DescriptorProperty::Comment)?,
                }),
                PropertiesSite::Subsystem => {
                    PropertiesIntent::Subsystem(SubsystemPropertyChanges {
                        synonym: values_reader.string(DescriptorProperty::Synonym)?,
                        comment: values_reader.string(DescriptorProperty::Comment)?,
                        explanation: values_reader.string(DescriptorProperty::Explanation)?,
                        include_help_in_contents: values_reader
                            .boolean(DescriptorProperty::IncludeHelpInContents)?,
                        include_in_command_interface: values_reader
                            .boolean(DescriptorProperty::IncludeInCommandInterface)?,
                        use_one_command: values_reader
                            .boolean(DescriptorProperty::UseOneCommand)?,
                    })
                }
            };
            reader.finish(values_reader.finish(intent)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::{
        MetaPropertyKey, MetaPropertyValue, MetadataKind, MetadataTypeVariant,
    };
    use serde_json::json;

    fn decode(site: PropertiesSite, args: Value) -> DecodeResult<PropertiesIntent> {
        decode_properties(PropertiesOperation::Set, site, &args, "ops[7].args")
    }

    #[test]
    fn every_current_metadata_property_uses_its_owner_schema_and_typed_value() {
        for kind in MetadataKind::ALL {
            let site = PropertiesSite::Metadata(*kind);
            let contract = PropertiesOperation::Set.contract(site);
            let oracle = jsonschema::validator_for(&contract.schema).unwrap();
            for spec in METADATA_PROPERTY_SPECS
                .iter()
                .filter(|spec| spec.allowed_kinds.contains(kind))
            {
                let (input, expected) = match spec.value_kind {
                    MetaPropertyValueKind::Type => (
                        json!({"variants":[{"kind":"boolean"}]}),
                        MetaPropertyValue::Type(crate::domain::metadata::MetadataType {
                            variants: vec![MetadataTypeVariant::Boolean],
                        }),
                    ),
                    MetaPropertyValueKind::Boolean => {
                        (json!(false), MetaPropertyValue::Boolean(false))
                    }
                    MetaPropertyValueKind::UnsignedInteger => {
                        (json!(0.0), MetaPropertyValue::UnsignedInteger(0))
                    }
                    MetaPropertyValueKind::String => {
                        let text = spec.enum_values.first().copied().unwrap_or(match spec.key {
                            MetaPropertyKey::Event => "OnWrite",
                            MetaPropertyKey::Handler => "CommonModule.Events.OnWrite",
                            _ => "property text",
                        });
                        (json!(text), MetaPropertyValue::String(text.into()))
                    }
                };
                let args = json!({"values":{spec.public_name:input}});
                assert!(
                    oracle.is_valid(&args),
                    "{kind:?} {}: {args}",
                    spec.public_name
                );
                let PropertiesIntent::Metadata(changes) = decode(site, args).unwrap() else {
                    panic!("metadata property must retain its typed Metadata intent");
                };
                assert_eq!(
                    changes.entries(),
                    &[(spec.key, expected)],
                    "{kind:?} {}",
                    spec.public_name
                );
            }
        }
    }

    #[test]
    fn descriptor_aliases_keep_independent_values_false_and_empty_strings() {
        assert_eq!(
            decode(
                PropertiesSite::Role,
                json!({"values":{"synonym":"S","Comment":""}})
            )
            .unwrap(),
            PropertiesIntent::Role(RolePropertyChanges {
                synonym: Some("S".into()),
                comment: Some(String::new())
            })
        );
        assert_eq!(
            decode(PropertiesSite::Subsystem, json!({"values":{
                "Synonym":"S", "comment":"C", "explanation":"",
                "includeHelpInContents":false, "IncludeInCommandInterface":true, "useOneCommand":false
            }})).unwrap(),
            PropertiesIntent::Subsystem(SubsystemPropertyChanges {
                synonym: Some("S".into()), comment: Some("C".into()), explanation: Some(String::new()),
                include_help_in_contents: Some(false), include_in_command_interface: Some(true), use_one_command: Some(false),
            })
        );
        for site in [PropertiesSite::Role, PropertiesSite::Subsystem] {
            for field in site.descriptor_fields() {
                for spelling in field.names() {
                    let value = match field.kind() {
                        DescriptorPropertyKind::String => json!(""),
                        DescriptorPropertyKind::Boolean => json!(false),
                    };
                    assert!(
                        decode(site, json!({"values":{*spelling:value}})).is_ok(),
                        "{site:?}: {spelling}"
                    );
                }
            }
        }
    }

    #[test]
    fn collisions_types_and_closed_fields_report_the_original_caller_path() {
        for site in [PropertiesSite::Role, PropertiesSite::Subsystem] {
            for field in site.descriptor_fields() {
                let names = field.names();
                for equal in [true, false] {
                    let value = match field.kind() {
                        DescriptorPropertyKind::String => json!("value"),
                        DescriptorPropertyKind::Boolean => json!(true),
                    };
                    let other = if equal { value.clone() } else { json!(false) };
                    let error = decode(site, json!({"values":{names[0]:value,names[1]:other}}))
                        .unwrap_err();
                    assert_eq!(error.path, format!("ops[7].args.values.{}", names[1]));
                    assert!(error.message.contains("one spelling"));
                }
                for spelling in names {
                    let invalid = match field.kind() {
                        DescriptorPropertyKind::String => json!(false),
                        DescriptorPropertyKind::Boolean => json!("false"),
                    };
                    assert_eq!(
                        decode(site, json!({"values":{*spelling:invalid}}))
                            .unwrap_err()
                            .path,
                        format!("ops[7].args.values.{spelling}")
                    );
                }
            }
            for args in [
                json!({}),
                json!({"values":null}),
                json!({"values":[]}),
                json!({"values":{}}),
            ] {
                assert_eq!(decode(site, args).unwrap_err().path, "ops[7].args.values");
            }
            assert_eq!(
                decode(site, json!({"values":{"Comment":"x"},"unknown":true}))
                    .unwrap_err()
                    .path,
                "ops[7].args.unknown"
            );
        }
        assert_eq!(
            decode(
                PropertiesSite::Role,
                json!({"values":{"UseOneCommand":false}})
            )
            .unwrap_err()
            .path,
            "ops[7].args.values.UseOneCommand"
        );
    }

    #[test]
    fn metadata_numeric_spellings_and_structured_reference_types_survive_conversion() {
        let site = PropertiesSite::Metadata(MetadataKind::Catalog);
        for spelling in ["0", "0.0", "0e0", "4294967295", "4294967295.0"] {
            let number: Value = serde_json::from_str(spelling).unwrap();
            let PropertiesIntent::Metadata(changes) =
                decode(site, json!({"values":{"CodeLength":number}})).unwrap()
            else {
                unreachable!()
            };
            assert_eq!(
                changes.entries(),
                &[(
                    MetaPropertyKey::CodeLength,
                    MetaPropertyValue::UnsignedInteger(if spelling.starts_with('0') {
                        0
                    } else {
                        u32::MAX
                    })
                )]
            );
        }
        for spelling in ["-1", "0.5", "4294967296", "1e100"] {
            let number: Value = serde_json::from_str(spelling).unwrap();
            assert_eq!(
                decode(site, json!({"values":{"CodeLength":number}}))
                    .unwrap_err()
                    .path,
                "ops[7].args.values.CodeLength"
            );
        }
        let args = json!({"values":{"Type":{"variants":[{"kind":"reference","metadataPath":"Catalog.Items"}]}}});
        let PropertiesIntent::Metadata(changes) =
            decode(PropertiesSite::Metadata(MetadataKind::Constant), args).unwrap()
        else {
            unreachable!()
        };
        let MetaPropertyValue::Type(ty) = &changes.entries()[0].1 else {
            panic!("Type must stay structured")
        };
        assert!(
            matches!(&ty.variants[0], MetadataTypeVariant::Reference { metadata_path } if metadata_path.as_str() == "Catalog.Items")
        );
        assert_eq!(
            decode(
                PropertiesSite::Metadata(MetadataKind::Constant),
                json!({"values":{"Type":{"variants":[{"kind":"boolean","extra":true}]}}})
            )
            .unwrap_err()
            .path,
            "ops[7].args.values.Type.variants[0].extra"
        );
    }

    #[test]
    fn declared_optional_fields_defaults_and_alias_groups_cannot_escape_consumption() {
        let site = PropertiesSite::Role;
        let args = json!({"values":{"Comment":"x"}});
        assert!(decode(site, args.clone()).is_ok());
        let baseline = PropertiesOperation::Set.contract(site);
        let mut mutations = Vec::new();
        let mut schema = baseline.schema.clone();
        schema["properties"]["forgotten"] = json!({"type":"boolean"});
        mutations.push((schema, "ops[7].args.forgotten"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["properties"]["Forgotten"] = json!({"type":"boolean"});
        mutations.push((schema, "ops[7].args.values.Forgotten"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["properties"]["Comment"]["type"] = json!("boolean");
        mutations.push((schema, "ops[7].args.values.Comment"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["properties"]["Comment"]["enum"] = json!(["different"]);
        mutations.push((schema, "ops[7].args.values.Comment"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["required"] = json!(["Synonym"]);
        mutations.push((schema, "ops[7].args.values.Synonym"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["properties"]["Synonym"]["default"] =
            json!("ignored default");
        mutations.push((schema, "ops[7].args.values.Synonym"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["x-unica-alias-groups"] =
            json!([["Synonym", "synonym", "Comment", "comment"]]);
        mutations.push((schema, "ops[7].args.values.Synonym"));
        let mut schema = baseline.schema.clone();
        schema["properties"]["values"]["x-unica-alias-groups"][0] =
            json!(["Synonym", "synonym", "newAlias"]);
        mutations.push((schema, "ops[7].args.values.Synonym"));
        for (schema, expected_path) in mutations {
            let contract = OperationContract {
                schema,
                ..PropertiesOperation::Set.contract(site)
            };
            let error = decode_with_contract(
                PropertiesOperation::Set,
                site,
                &args,
                "ops[7].args",
                &contract,
            )
            .unwrap_err();
            assert_eq!(error.path, expected_path, "{}", error.message);
        }
    }
}
