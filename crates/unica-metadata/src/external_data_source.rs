//! Closed address and property profile of an ordinary external data source.

use serde_json::{json, Map, Value};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_operation_contract::ContractError;

use crate::{LocalizedItem, MetadataType, MetadataTypeVariant};

pub fn kind(segments: &[AddressSegment]) -> Option<NodeKind> {
    let [root, children @ ..] = segments else {
        return None;
    };
    if root.kind() != NodeKind::ExternalDataSource
        || segments.iter().any(|segment| segment.name().is_none())
    {
        return None;
    }
    let mut parent = root.kind();
    for segment in children {
        let child = segment.kind();
        let order = unica_address::kinds::declared(parent.as_str())
            .and_then(|(_, declaration)| declaration.child_order)?;
        if !order.contains(&child.as_str()) {
            return None;
        }
        parent = child;
    }
    Some(parent)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Synonym,
    Comment,
    SourceName,
    Expression,
    ReturnValue,
    Type,
    ReadOnly,
    AllowNull,
}

impl Field {
    pub const fn names(self) -> [&'static str; 2] {
        match self {
            Self::Synonym => ["Synonym", "synonym"],
            Self::Comment => ["Comment", "comment"],
            Self::SourceName => ["NameInDataSource", "nameInDataSource"],
            Self::Expression => ["ExpressionInDataSource", "expressionInDataSource"],
            Self::ReturnValue => ["ReturnValue", "returnValue"],
            Self::Type => ["Type", "type"],
            Self::ReadOnly => ["ReadOnly", "readOnly"],
            Self::AllowNull => ["AllowNull", "allowNull"],
        }
    }

    fn schema(self, kind: NodeKind) -> Value {
        match self {
            Self::Synonym => crate::localized_value_schema(),
            Self::Type if kind == NodeKind::Dimension => json!({
                "type":"string", "minLength":1,
                "description": "Qualified logical address of an existing DimensionTable of the same cube."
            }),
            Self::Type => single_type_schema(),
            Self::ReturnValue | Self::ReadOnly | Self::AllowNull => json!({"type":"boolean"}),
            Self::SourceName => json!({"type":"string", "minLength":1}),
            Self::Expression | Self::Comment => json!({"type":"string"}),
        }
    }
}

fn fields(kind: NodeKind) -> &'static [Field] {
    use Field::*;
    match kind {
        NodeKind::ExternalDataSource => &[Synonym, Comment],
        NodeKind::Table => &[Synonym, Comment, SourceName, Expression, ReadOnly],
        NodeKind::Cube | NodeKind::DimensionTable => &[Synonym, Comment, SourceName],
        NodeKind::Field => &[Synonym, Comment, Type, SourceName, ReadOnly, AllowNull],
        NodeKind::Function => &[Synonym, Comment, ReturnValue, Type, Expression],
        NodeKind::Dimension => &[Synonym, Comment, Type],
        NodeKind::Resource => &[Synonym, Comment, Type, SourceName],
        _ => &[],
    }
}

pub fn values_schema(kind: NodeKind) -> Value {
    let mut properties = Map::new();
    let mut aliases = Vec::new();
    for field in fields(kind) {
        let names = field.names();
        let schema = field.schema(kind);
        for name in names {
            properties.insert(name.into(), schema.clone());
        }
        aliases.push(names);
    }
    let duplicate = aliases
        .iter()
        .map(|pair| json!({"required":pair}))
        .collect::<Vec<_>>();
    json!({
        "type":"object",
        "minProperties":1,
        "additionalProperties":false,
        "properties": properties,
        "not": {"anyOf":duplicate},
        "x-unica-alias-groups": aliases,
    })
}

pub fn single_type_schema() -> Value {
    let mut schema = crate::metadata_type_schema();
    let variants = &mut schema["properties"]["variants"];
    variants["maxItems"] = json!(1);
    variants["description"] = json!("Exactly one string, number or boolean platform type.");
    variants["items"]["oneOf"]
        .as_array_mut()
        .expect("metadata type variant schema")
        .retain(|variant| {
            matches!(
                variant["properties"]["kind"]["enum"][0].as_str(),
                Some("string" | "number" | "boolean")
            )
        });
    schema
}

pub fn single_type(value: &Value, path: &str) -> Result<MetadataType, ContractError> {
    let value = crate::request::parse_metadata_type(value, path)
        .map_err(|error| bad(error.field.as_deref().unwrap_or(path), error.message))?;
    if !matches!(
        value.variants.as_slice(),
        [MetadataTypeVariant::String { .. }
            | MetadataTypeVariant::Number { .. }
            | MetadataTypeVariant::Boolean]
    ) {
        return Err(bad(
            path,
            "external data source type must be one string, number or boolean",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    Text(String),
    Bool(bool),
    Synonym(Vec<LocalizedItem>),
    Type(MetadataType),
    DimensionTable(QualifiedAddress),
}

pub type Changes = Vec<(Field, PropertyValue)>;

fn bad(path: &str, message: impl Into<String>) -> ContractError {
    ContractError {
        enum_value: false,
        path: path.into(),
        message: message.into(),
    }
}

pub fn decode_values(
    kind: NodeKind,
    values: &Map<String, Value>,
    path: &str,
) -> Result<Changes, ContractError> {
    let mut changes = Vec::new();
    for name in values.keys() {
        if !fields(kind)
            .iter()
            .any(|field| field.names().contains(&name.as_str()))
        {
            return Err(bad(
                &format!("{path}.{name}"),
                format!("this external data source node cannot set {name}"),
            ));
        }
    }
    for field in fields(kind) {
        let [canonical, alias] = field.names();
        let (name, value) = match (values.get(canonical), values.get(alias)) {
            (Some(_), Some(_)) => {
                return Err(bad(
                    &format!("{path}.{alias}"),
                    "supply one spelling of the property",
                ))
            }
            (Some(value), None) => (canonical, value),
            (None, Some(value)) => (alias, value),
            (None, None) => continue,
        };
        let path = format!("{path}.{name}");
        let text = || {
            value
                .as_str()
                .filter(|text| text.chars().all(crate::properties::is_xml_text_character))
                .ok_or_else(|| bad(&path, "property must be valid XML text"))
        };
        let value = match field {
            Field::Synonym => PropertyValue::Synonym(
                crate::parse_nstr(text()?).map_err(|error| bad(&path, error))?,
            ),
            Field::ReturnValue | Field::ReadOnly | Field::AllowNull => PropertyValue::Bool(
                value
                    .as_bool()
                    .ok_or_else(|| bad(&path, "property must be a boolean"))?,
            ),
            Field::Type if kind == NodeKind::Dimension => PropertyValue::DimensionTable(
                QualifiedAddress::parse(text()?).map_err(|error| bad(&path, error.to_string()))?,
            ),
            Field::Type => PropertyValue::Type(single_type(value, &path)?),
            Field::SourceName => {
                let text = text()?;
                if text.trim().is_empty() {
                    return Err(bad(&path, "name in data source must be nonempty"));
                }
                PropertyValue::Text(text.to_string())
            }
            Field::Comment | Field::Expression => PropertyValue::Text(text()?.to_string()),
        };
        changes.push((*field, value));
    }
    if changes.is_empty() {
        return Err(bad(path, "supply at least one property"));
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_existing_external_source_address_shapes_offer_properties() {
        for at in [
            "main:ExternalDataSource.Store",
            "main:ExternalDataSource.Store.Table.Items.Field.Code",
            "main:ExternalDataSource.Store.Cube.Sales.DimensionTable.Products.Field.Code",
            "main:ExternalDataSource.Store.Function.Total",
        ] {
            assert!(
                kind(QualifiedAddress::parse(at).unwrap().segments()).is_some(),
                "{at}"
            );
        }
        for at in [
            "main:ExternalDataSource.Store.Function.Total.Field.Code",
            "main:ExternalDataSource.Store.Function.Total.Parameter.Value",
            "main:ExternalDataSource.Store.Cube.Sales.Field.Code",
        ] {
            assert!(
                QualifiedAddress::parse(at).is_err()
                    || kind(QualifiedAddress::parse(at).unwrap().segments()).is_none(),
                "{at}"
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddContent {
    Table {
        source: TableSource,
        data_type: TableDataType,
    },
    Cube(String),
    DimensionTable(String),
    Field {
        source_name: String,
        field_type: MetadataType,
    },
    Function {
        expression: String,
        return_value: bool,
        return_type: MetadataType,
    },
    Dimension(QualifiedAddress),
    Resource {
        source_name: String,
        field_type: MetadataType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableSource {
    Table(String),
    Expression(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableDataType {
    NonobjectData,
    ObjectData,
}

impl TableDataType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonobjectData => "NonobjectData",
            Self::ObjectData => "ObjectData",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddItem {
    pub name: String,
    pub content: AddContent,
}

pub fn add_contract(kind: NodeKind) -> unica_operation_contract::OperationContract {
    use unica_operation_contract::{object, text, OperationContract};
    let name = text(true);
    let source_name = text(true);
    let field_type = single_type_schema();
    let named = {
        object(
            &[
                ("name", name.clone()),
                ("nameInDataSource", source_name.clone()),
            ],
            &["name", "nameInDataSource"],
        )
    };
    let mut item = match kind {
        NodeKind::Table => object(
            &[
                ("name", name),
                (
                    "tableType",
                    json!({"type":"string","enum":["Table","Expression"]}),
                ),
                ("nameInDataSource", source_name),
                ("expressionInDataSource", text(true)),
                (
                    "tableDataType",
                    json!({"type":"string","enum":["NonobjectData","ObjectData"]}),
                ),
            ],
            &["name", "tableType"],
        ),
        NodeKind::Function => object(
            &[
                ("name", name),
                ("returnValue", json!({"type":"boolean"})),
                ("type", field_type),
                ("expressionInDataSource", text(false)),
            ],
            &["name", "returnValue", "type", "expressionInDataSource"],
        ),
        NodeKind::Field | NodeKind::Resource => object(
            &[
                ("name", name),
                ("nameInDataSource", source_name),
                ("type", field_type),
            ],
            &["name", "nameInDataSource", "type"],
        ),
        NodeKind::Dimension => object(&[("name", name), ("type", text(true))], &["name", "type"]),
        _ => named,
    };
    if kind == NodeKind::Table {
        item["if"] = json!({"properties":{"tableType":{"enum":["Table"]}}});
        item["then"] =
            json!({"required":["nameInDataSource"],"not":{"required":["expressionInDataSource"]}});
        item["else"] =
            json!({"required":["expressionInDataSource"],"not":{"required":["nameInDataSource"]}});
    }
    let (target, example) = match kind {
        NodeKind::Table => (
            "An existing or staged ExternalDataSource in an ordinary configuration.",
            json!({"name":"Items","tableType":"Table","nameInDataSource":"items"}),
        ),
        NodeKind::Cube => (
            "An existing or staged ExternalDataSource in an ordinary configuration.",
            json!({"name":"Sales","nameInDataSource":"sales"}),
        ),
        NodeKind::Function => (
            "An existing or staged ExternalDataSource in an ordinary configuration.",
            json!({"name":"Total","returnValue":true,"type":{"variants":[{"kind":"boolean"}]},"expressionInDataSource":"dbo.Total(&1)"}),
        ),
        NodeKind::DimensionTable => (
            "An existing or staged Cube of an external data source.",
            json!({"name":"Products","nameInDataSource":"products"}),
        ),
        NodeKind::Dimension => (
            "An existing or staged Cube of an external data source.",
            json!({"name":"Product","type":"main:ExternalDataSource.Store.Cube.Sales.DimensionTable.Products"}),
        ),
        NodeKind::Resource => (
            "An existing or staged Cube of an external data source.",
            json!({"name":"Amount","type":{"variants":[{"kind":"boolean"}]},"nameInDataSource":"amount"}),
        ),
        _ => (
            "An existing or staged Table or DimensionTable of an external data source.",
            json!({"name":"Code","type":{"variants":[{"kind":"boolean"}]},"nameInDataSource":"code"}),
        ),
    };
    OperationContract {
        schema: object(&[("items", json!({"type":"array","minItems":1,"items":item}))], &["items"]),
        target,
        effect: "Create every item in its registered owner and preserve other objects and byte ranges.",
        notes: "Identifiers are unique within this owner and kind. Table requires NameInDataSource. Expression table requires ExpressionInDataSource. TableDataType defaults to NonobjectData. A dimension type is the qualified address of a dimension table of the same cube. A function's formal parameters are part of its expression. Preview writes nothing; execute only the saved token.",
        example_args: json!({"items":[example]}),
        help: "skills/meta-edit/SKILL.md",
        format_validators: &[],
    }
}

pub fn decode_add(
    kind: NodeKind,
    items: &Value,
    path: &str,
) -> Result<Vec<AddItem>, ContractError> {
    let items = items
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| bad(path, "items must be a nonempty array"))?;
    let contract = add_contract(kind);
    let properties = contract.schema["properties"]["items"]["items"]["properties"]
        .as_object()
        .expect("external item schema");
    let mut result = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let item = item
            .as_object()
            .ok_or_else(|| bad(&item_path, "item must be an object"))?;
        for name in item.keys() {
            if !properties.contains_key(name) {
                return Err(bad(
                    &format!("{item_path}.{name}"),
                    format!("unknown item argument {name}"),
                ));
            }
        }
        let text = |field, nonempty| atom(item, field, &item_path, nonempty);
        let name = text("name", true)?;
        if !crate::identifier::is_1c_identifier(name) {
            return Err(bad(
                &format!("{item_path}.name"),
                "name must be a 1C identifier",
            ));
        }
        let source_name = || text("nameInDataSource", true).map(str::to_string);
        let type_path = format!("{item_path}.type");
        let field_type = || single_type(item.get("type").unwrap_or(&Value::Null), &type_path);
        let content = match kind {
            NodeKind::Table => {
                let source = match text("tableType", true)? {
                    "Table" if !item.contains_key("expressionInDataSource") => {
                        TableSource::Table(source_name()?)
                    }
                    "Expression" if !item.contains_key("nameInDataSource") => {
                        TableSource::Expression(text("expressionInDataSource", true)?.to_string())
                    }
                    _ => {
                        return Err(bad(
                            &format!("{item_path}.tableType"),
                            "Table takes nameInDataSource; Expression takes expressionInDataSource",
                        ))
                    }
                };
                let data_type = match item.get("tableDataType") {
                    None => TableDataType::NonobjectData,
                    Some(value) => match value.as_str() {
                        Some("NonobjectData") => TableDataType::NonobjectData,
                        Some("ObjectData") => TableDataType::ObjectData,
                        _ => {
                            return Err(bad(
                                &format!("{item_path}.tableDataType"),
                                "expected NonobjectData or ObjectData",
                            ))
                        }
                    },
                };
                AddContent::Table { source, data_type }
            }
            NodeKind::Cube => AddContent::Cube(source_name()?),
            NodeKind::DimensionTable => AddContent::DimensionTable(source_name()?),
            NodeKind::Field => AddContent::Field {
                source_name: source_name()?,
                field_type: field_type()?,
            },
            NodeKind::Function => AddContent::Function {
                expression: text("expressionInDataSource", false)?.to_string(),
                return_value: item
                    .get("returnValue")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        bad(
                            &format!("{item_path}.returnValue"),
                            "returnValue must be a boolean",
                        )
                    })?,
                return_type: field_type()?,
            },
            NodeKind::Dimension => AddContent::Dimension(
                QualifiedAddress::parse(text("type", true)?)
                    .map_err(|error| bad(&type_path, error.to_string()))?,
            ),
            NodeKind::Resource => AddContent::Resource {
                source_name: source_name()?,
                field_type: field_type()?,
            },
            _ => return Err(bad(path, "unknown external child profile")),
        };
        if result
            .iter()
            .any(|item: &AddItem| crate::metadata_name_eq(&item.name, name))
        {
            return Err(bad(
                &format!("{item_path}.name"),
                "the batch contains a duplicate item name",
            ));
        }
        result.push(AddItem {
            name: name.to_string(),
            content,
        });
    }
    Ok(result)
}

fn atom<'a>(
    item: &'a Map<String, Value>,
    field: &str,
    path: &str,
    nonempty: bool,
) -> Result<&'a str, ContractError> {
    item.get(field)
        .and_then(Value::as_str)
        .filter(|text| !nonempty || !text.trim().is_empty())
        .filter(|text| text.chars().all(crate::properties::is_xml_text_character))
        .ok_or_else(|| {
            bad(
                &format!("{path}.{field}"),
                "expected the field's text value",
            )
        })
}

pub fn inline_read_facts(kind: NodeKind) -> &'static [&'static str] {
    match kind {
        NodeKind::Function => &["ReturnValue", "ExpressionInDataSource"],
        NodeKind::Field => &["NameInDataSource", "ReadOnly", "AllowNull"],
        NodeKind::Resource => &["NameInDataSource"],
        _ => &[],
    }
}
