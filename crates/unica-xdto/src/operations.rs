//! The current XDTO mutation profile owns its names, operands and argument grammar.

use crate::applicability::OperationApplicability;
use crate::facets::ValueFacets;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use unica_address::address::{escape_name, NodeKind, QualifiedAddress};
use unica_operation_contract::OperationSkeleton;
use unica_operation_contract::{object, text, ArgumentShape, ContractError, OperationContract};
use unica_xml_format::names::{
    prefixed_xml_qname_pattern, xml_ncname_is_valid, xml_ncname_pattern,
};

macro_rules! xdto_operations {
    ($($variant:ident => ($name:literal, $site:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum XdtoOperation { $($variant),+ }

        impl XdtoOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<XdtoOperation> = unica_operations::TypedOperationBundle::new(XdtoOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }

            const fn site(self) -> XdtoSite {
                match self { $(Self::$variant => XdtoSite::$site),+ }
            }

            pub const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$site),+ }
            }

            pub const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }
        }
    };
}

xdto_operations!(
    AddValueType => ("valueType.add", XdtoPackage, Values),
    AddObjectType => ("objectType.add", XdtoPackage, Values),
    AddProperty => ("property.add", XdtoPropertyContainer, Values),
    RemoveType => ("type.remove", XdtoType, Target),
    RemoveProperty => ("property.remove", XdtoProperty, Target),
    RenameProperty => ("property.rename", XdtoProperty, Values),
    SetType => ("type.set", XdtoType, Values),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// These names are also the applicability identities read from the family table.
// Keeping them equal avoids a second role-name mapping in metadata consumers.
#[allow(clippy::enum_variant_names)]
pub enum XdtoSite {
    XdtoPackage,
    XdtoType,
    XdtoProperty,
    XdtoPropertyContainer,
}

impl XdtoSite {
    pub fn matches(self, at: &QualifiedAddress) -> bool {
        let segments = at.segments();
        let Some(package) = segments.first() else {
            return false;
        };
        if package.kind() != NodeKind::XdtoPackage || package.name().is_none() {
            return false;
        }
        if self == Self::XdtoPackage {
            return segments.len() == 1;
        }
        let Some(ty) = segments.get(1) else {
            return false;
        };
        if ty.kind() != NodeKind::Type || ty.name().is_none() {
            return false;
        }
        let properties = &segments[2..];
        (self != Self::XdtoType || properties.is_empty())
            && (self != Self::XdtoProperty || !properties.is_empty())
            && properties
                .iter()
                .all(|property| property.kind() == NodeKind::Property && property.name().is_some())
    }

    fn resolve(
        self,
        raw: &str,
        source_set: &str,
        path: &str,
    ) -> Result<(QualifiedAddress, Vec<String>), ContractError> {
        if raw.is_empty() || raw != raw.trim() {
            return Err(invalid(path, "at must be a non-empty logical address"));
        }
        let address = QualifiedAddress::parse(raw)
            .map_err(|_| invalid(path, "at is not a valid logical XDTO address"))?;
        if address.source_set() != source_set {
            return Err(invalid(
                path,
                "at belongs to a different admitted source set",
            ));
        }
        let refused = || invalid(path, "at does not identify the required XDTO node");
        if !self.matches(&address) {
            return Err(refused());
        }
        let segments = address.segments();
        let package = &segments[0];
        if self == Self::XdtoPackage {
            return Ok((address, Vec::new()));
        }
        let ty = &segments[1];
        let properties = segments[2..]
            .iter()
            .map(|property| property.name().expect("resolved named property").to_owned())
            .collect::<Vec<_>>();
        let type_at = QualifiedAddress::parse(&format!(
            "{}:XDTOPackage.{}.Type.{}",
            address.source_set(),
            escape_name(package.name().expect("named XDTO package")),
            escape_name(ty.name().expect("named XDTO type")),
        ))
        .map_err(|_| refused())?;
        Ok((type_at, properties))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlNcName(String);

impl XmlNcName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(value: &str) -> Self {
        assert!(xml_ncname_is_valid(value));
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixedXmlQName(String);

impl PrefixedXmlQName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(value: &str) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoPropertyPath(String);

impl XdtoPropertyPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Property names of the chain with the address escape removed.
    pub fn names(&self) -> Vec<String> {
        let mut names = vec![String::new()];
        let mut characters = self.0.chars();
        while let Some(character) = characters.next() {
            match character {
                '\\' => names
                    .last_mut()
                    .expect("one name is open")
                    .extend(characters.next()),
                '.' => names.push(String::new()),
                _ => names.last_mut().expect("one name is open").push(character),
            }
        }
        names
    }

    fn from_segments(properties: &[String]) -> Option<Self> {
        (!properties.is_empty()).then(|| {
            Self(
                properties
                    .iter()
                    .map(|name| escape_name(name).into_owned())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(value: &str) -> Self {
        Self(value.into())
    }
}

/// The supported creation profile for an own property: a QName type or an
/// anonymous ValueType or ObjectType with ordered child properties. Observed properties
/// and future patch/ref inputs are distinct profiles; this is not the complete
/// platform property algebra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoPropertySpec {
    pub name: XmlNcName,
    pub definition: XdtoPropertyDefinition,
    pub min_occurs: Option<u8>,
    /// `-1` is unbounded; absent stays absent and means 1.
    pub max_occurs: Option<i64>,
}

/// How a created property defines its values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XdtoPropertyDefinition {
    /// A named type referenced by its prefixed QName.
    Named(PrefixedXmlQName),
    /// An anonymous ObjectType declared inside the property.
    Object(Vec<XdtoPropertySpec>),
    /// An anonymous primitive ValueType with optional restrictions.
    Value {
        base: PrefixedXmlQName,
        facets: ValueFacets,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoAddValueTypeArgs {
    pub at: QualifiedAddress,
    pub name: XmlNcName,
    pub base: PrefixedXmlQName,
    pub facets: ValueFacets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoAddObjectTypeArgs {
    pub at: QualifiedAddress,
    pub name: XmlNcName,
    /// A local object type the new one extends.
    pub base: Option<PrefixedXmlQName>,
    /// The object type the new one is inserted immediately before.
    pub before: Option<XmlNcName>,
    /// The object type the new one is inserted immediately after.
    pub after: Option<XmlNcName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoAddPropertyArgs {
    pub at: QualifiedAddress,
    pub property: XdtoPropertySpec,
    pub property_path: Option<XdtoPropertyPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoRemoveTypeArgs {
    pub at: QualifiedAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoRemovePropertyArgs {
    pub at: QualifiedAddress,
    pub name: XmlNcName,
    pub property_path: Option<XdtoPropertyPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoRenamePropertyArgs {
    pub at: QualifiedAddress,
    pub name: XmlNcName,
    pub property_path: Option<XdtoPropertyPath>,
    pub new_name: XmlNcName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdtoSetTypeArgs {
    pub at: QualifiedAddress,
    /// The new base of the addressed named type.
    pub base: PrefixedXmlQName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XdtoPlanOperation {
    AddValueType(XdtoAddValueTypeArgs),
    AddObjectType(XdtoAddObjectTypeArgs),
    AddProperty(XdtoAddPropertyArgs),
    RemoveType(XdtoRemoveTypeArgs),
    RemoveProperty(XdtoRemovePropertyArgs),
    RenameProperty(XdtoRenamePropertyArgs),
    SetType(XdtoSetTypeArgs),
}

/// Name of the recursive property definition in the published schema.
const PROPERTY_DEFINITION: &str = "xdtoProperty";

fn facets_schema() -> Value {
    let decimal = lexical_schema(r"^[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)$".to_string());
    object(
        &[
            ("maxLength", json!({"type":"integer","minimum":0})),
            ("totalDigits", json!({"type":"integer","minimum":1})),
            ("fractionDigits", json!({"type":"integer","minimum":0})),
            ("minInclusive", decimal.clone()),
            ("maxInclusive", decimal),
        ],
        &[],
    )
}

/// One created property has exactly one definition. Anonymous ObjectType
/// children use the same grammar through `$defs`.
fn property_definition() -> Value {
    let mut property = object(
        &[
            ("name", ncname()),
            ("type", qname()),
            (
                "valueType",
                object(&[("base", qname()), ("facets", facets_schema())], &["base"]),
            ),
            (
                "objectType",
                object(
                    &[(
                        "properties",
                        json!({
                            "type":"array",
                            "items":{"$ref":format!("#/$defs/{PROPERTY_DEFINITION}")},
                        }),
                    )],
                    &["properties"],
                ),
            ),
            ("lowerBound", json!({"type":"integer", "enum":[0,1]})),
            (
                "upperBound",
                json!({"type":"integer", "anyOf":[{"enum":[-1]}, {"minimum":1}]}),
            ),
        ],
        &["name"],
    );
    property["anyOf"] =
        json!([{"required":["type"]}, {"required":["objectType"]}, {"required":["valueType"]}]);
    property["not"] = json!({"anyOf":[{"required":["type","objectType"]},{"required":["type","valueType"]},{"required":["objectType","valueType"]}]});
    property
}

fn lexical_schema(pattern: String) -> Value {
    // ECMAScript's end anchor can match before a final line ending. The
    // explicit exclusion keeps the published grammar equal to the XML names.
    json!({"type":"string", "pattern":pattern, "not":{"pattern":r"[\r\n]"}})
}

fn ncname() -> Value {
    lexical_schema(xml_ncname_pattern())
}
fn qname() -> Value {
    lexical_schema(prefixed_xml_qname_pattern())
}

impl XdtoOperation {
    pub fn contract(self) -> OperationContract {
        let (payload, target, effect, example_args) = match self {
            Self::AddValueType => (
                Some(object(&[("name", ncname()), ("base", qname()), ("facets",facets_schema())], &["name", "base"])),
                "The XDTO package: <source-set>:XDTOPackage.<Name>.",
                "Add a named value type with the supplied prefixed base QName.",
                json!({"values":{"name":"AddedValue", "base":"xs:string"}}),
            ),
            Self::AddObjectType => (
                Some({
                    let mut values = object(
                        &[("name", ncname()), ("base", qname()), ("before", ncname()), ("after", ncname())],
                        &["name"],
                    );
                    values["not"] = json!({"required":["before", "after"]});
                    values
                }),
                "The XDTO package: <source-set>:XDTOPackage.<Name>.",
                "Add a named object type with an optional local base, immediately before or after an existing object type or appended, preserving existing bytes and order. The base and its whole chain must resolve to local object types without a cycle; an imported base is not supported. An earlier operation on the same package may create the base or the neighbour.",
                json!({"values":{"name":"AddedObject"}}),
            ),
            Self::AddProperty => (
                Some(object(&[("property", property_definition())], &["property"])),
                "An object type or a property that holds one nested anonymous ObjectType: ...Type.T[.Property.P...].",
                "Add one own property with exactly one QName type, anonymous ValueType with primitive base and optional facets, or anonymous ObjectType with ordered child properties. Omitted bounds stay absent and mean 1; explicit values stay explicit.",
                json!({"values":{"property":{"name":"AddedProperty", "type":"xs:string", "lowerBound":0}}}),
            ),
            Self::RemoveType => (
                None,
                "The existing type: <source-set>:XDTOPackage.<Name>.Type.<Type>.",
                "Remove the addressed named value or object type.",
                json!({}),
            ),
            Self::RemoveProperty => (
                None,
                "The existing property: ...Type.T.Property.P[.Property.Child...].",
                "Remove the addressed property, preserving the rest of its package.",
                json!({}),
            ),
            Self::RenameProperty => (
                Some(object(&[("newName", ncname())], &["newName"])),
                "The existing own property declared with name: ...Type.T.Property.P[.Property.Child...].",
                "Change only the name of the addressed own property, keeping its type, bounds, serialization attributes including localName, and position. The renamed property is read at the address with its last segment replaced; the old address no longer resolves.",
                json!({"values":{"newName":"RenamedProperty"}}),
            ),
            Self::SetType => (
                Some(object(&[("base", qname())], &["base"])),
                "The existing named type: <source-set>:XDTOPackage.<Name>.Type.<Type>.",
                "Change only the base of the addressed named type, keeping its name, properties, restrictions, other attributes and position. An object type takes a local object type whose chain has no cycle; a value type takes an XML Schema type or a local value type, and its restrictions must stay valid for the new base. Setting the current base writes nothing.",
                json!({"values":{"base":"tns:OtherBase"}}),
            ),
        };
        let mut schema = match payload {
            Some(payload) => object(&[("at", text(true)), ("values", payload)], &["values"]),
            None => object(&[("at", text(true))], &[]),
        };
        if self == Self::AddProperty {
            schema["$defs"] = json!({ PROPERTY_DEFINITION: property_definition() });
        }
        OperationContract {
            schema, target, effect, example_args,
            notes: "Names are XML NCNames and types are prefixed XML QNames. QName bindings and target structure are checked against the selected package. Only Package.bin is edited; exact additions, setting the current base and renaming to the current name repeat as no-op. Add, set and rename operands belong in values; remove has no payload. The common args.at address override remains available.",
            help: "skills/xdto/SKILL.md",
            format_validators: &[],
        }
    }

    pub fn decode(
        self,
        args: &Value,
        path: &str,
        source_set: &str,
    ) -> Result<XdtoPlanOperation, ContractError> {
        self.decode_with_contract(&self.contract(), args, path, source_set)
    }

    fn decode_with_contract(
        self,
        contract: &OperationContract,
        args: &Value,
        path: &str,
        source_set: &str,
    ) -> Result<XdtoPlanOperation, ContractError> {
        let mut reader = ArgumentReader::new(args, ArgumentShape::new(&contract.schema), path)?;
        let raw = reader.resolved_address()?;
        let (at, properties) = self
            .site()
            .resolve(&raw, source_set, &format!("{path}.at"))?;
        let intent = match self {
            Self::AddValueType => reader.required_object("values", |values| {
                Ok(XdtoPlanOperation::AddValueType(XdtoAddValueTypeArgs {
                    at,
                    name: values.ncname("name")?,
                    base: values.qname("base")?,
                    facets: values.facets("facets")?,
                }))
            })?,
            Self::AddObjectType => reader.required_object("values", |values| {
                if values.has("before") && values.has("after") {
                    return Err(invalid(
                        &format!("{}.after", values.path),
                        "before and after are mutually exclusive",
                    ));
                }
                Ok(XdtoPlanOperation::AddObjectType(XdtoAddObjectTypeArgs {
                    at,
                    name: values.ncname("name")?,
                    base: values.optional_qname("base")?,
                    before: values.optional_ncname("before")?,
                    after: values.optional_ncname("after")?,
                }))
            })?,
            Self::AddProperty => reader.required_object("values", |values| {
                // The top property is read by its own declared shape; nested
                // children by the recursive definition both are built from.
                let definition = ArgumentShape::new(&contract.schema["$defs"][PROPERTY_DEFINITION]);
                let property = values
                    .required_object("property", |property| property_spec(property, definition))?;
                Ok(XdtoPlanOperation::AddProperty(XdtoAddPropertyArgs {
                    at,
                    property,
                    property_path: XdtoPropertyPath::from_segments(&properties),
                }))
            })?,
            Self::RemoveType => XdtoPlanOperation::RemoveType(XdtoRemoveTypeArgs { at }),
            Self::SetType => reader.required_object("values", |values| {
                Ok(XdtoPlanOperation::SetType(XdtoSetTypeArgs {
                    at,
                    base: values.qname("base")?,
                }))
            })?,
            Self::RemoveProperty => {
                let (name, parents) = properties.split_last().expect("resolved property target");
                XdtoPlanOperation::RemoveProperty(XdtoRemovePropertyArgs {
                    at,
                    name: XmlNcName(name.clone()),
                    property_path: XdtoPropertyPath::from_segments(parents),
                })
            }
            Self::RenameProperty => reader.required_object("values", |values| {
                let (name, parents) = properties.split_last().expect("resolved property target");
                Ok(XdtoPlanOperation::RenameProperty(XdtoRenamePropertyArgs {
                    at,
                    name: XmlNcName(name.clone()),
                    property_path: XdtoPropertyPath::from_segments(parents),
                    new_name: values.ncname("newName")?,
                }))
            })?,
        };
        reader.finish(intent)
    }
}

/// Decodes one property of the recursive definition. Bounds are read first
/// to preserve the established operand diagnostic order.
fn property_spec<'a>(
    property: &mut ArgumentReader<'a>,
    definition: ArgumentShape<'a>,
) -> Result<XdtoPropertySpec, ContractError> {
    let min_occurs = property.min_occurs("lowerBound")?;
    let max_occurs = property.max_occurs("upperBound")?;
    let name = property.ncname("name")?;
    if ["type", "objectType", "valueType"]
        .iter()
        .filter(|key| property.has(key))
        .count()
        != 1
    {
        return Err(invalid(
            &property.path,
            "property requires exactly one of type, objectType or valueType",
        ));
    }
    let type_ref = property.optional_qname("type")?;
    let children = property.optional_object("objectType", |object| {
        let mut names = HashSet::new();
        object.required_items("properties", definition, |child, path| {
            let spec = property_spec(child, definition)?;
            if !names.insert(spec.name.as_str().to_string()) {
                return Err(invalid(
                    &format!("{path}.name"),
                    "duplicate anonymous ObjectType property name",
                ));
            }
            Ok(spec)
        })
    })?;
    let value = property.optional_object("valueType", |value| {
        Ok((value.qname("base")?, value.facets("facets")?))
    })?;
    let definition = match (type_ref, children, value) {
        (Some(type_ref), None, None) => XdtoPropertyDefinition::Named(type_ref),
        (None, Some(children), None) => XdtoPropertyDefinition::Object(children),
        (None, None, Some((base, facets))) => XdtoPropertyDefinition::Value { base, facets },
        _ => unreachable!("exactly one definition was checked above"),
    };
    Ok(XdtoPropertySpec {
        name,
        definition,
        min_occurs,
        max_occurs,
    })
}

fn qname_is_valid(value: &str) -> bool {
    value
        .split_once(':')
        .is_some_and(|(prefix, local)| xml_ncname_is_valid(prefix) && xml_ncname_is_valid(local))
}

fn invalid(path: &str, message: impl Into<String>) -> ContractError {
    ContractError {
        path: path.into(),
        message: message.into(),
    }
}

struct ArgumentReader<'a> {
    object: &'a Map<String, Value>,
    shape: ArgumentShape<'a>,
    path: String,
    consumed: HashSet<String>,
}

impl<'a> ArgumentReader<'a> {
    fn new(value: &'a Value, shape: ArgumentShape<'a>, path: &str) -> Result<Self, ContractError> {
        shape.validate_type(value, path)?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid(path, "expected an argument object"))?;
        for key in object.keys() {
            if shape.property(key).is_none() {
                return Err(invalid(
                    &format!("{path}.{key}"),
                    "unknown XDTO operation argument",
                ));
            }
        }
        Ok(Self {
            object,
            shape,
            path: path.into(),
            consumed: HashSet::new(),
        })
    }

    fn read<T>(
        &mut self,
        key: &str,
        convert: impl FnOnce(&'a Value, ArgumentShape<'a>, &str) -> Result<T, ContractError>,
    ) -> Result<Option<T>, ContractError> {
        let path = format!("{}.{key}", self.path);
        let shape = self
            .shape
            .property(key)
            .ok_or_else(|| invalid(&path, "decoded field has no contract"))?;
        self.consumed.insert(key.into());
        match self.object.get(key) {
            Some(value) => {
                shape.validate_type(value, &path)?;
                convert(value, shape, &path).map(Some)
            }
            None if self.shape.requires(key) => Err(invalid(&path, "required argument is missing")),
            None if shape.has_default() => {
                Err(invalid(&path, "declared default has no decoded operand"))
            }
            None => Ok(None),
        }
    }

    fn required<T>(
        &mut self,
        key: &str,
        convert: impl FnOnce(&'a Value, ArgumentShape<'a>, &str) -> Result<T, ContractError>,
    ) -> Result<T, ContractError> {
        if !self.shape.requires(key) {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "required operand is declared optional by its contract",
            ));
        }
        self.read(key, convert)?.ok_or_else(|| {
            invalid(
                &format!("{}.{key}", self.path),
                "required argument is missing",
            )
        })
    }

    // The common apply envelope resolves an inherited or explicit target
    // before family decoding; the public argument schema keeps at optional.
    fn resolved_address(&mut self) -> Result<String, ContractError> {
        self.read("at", |value, shape, path| {
            shape.validate_field(value, path)?;
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, "expected text"))
        })?
        .ok_or_else(|| {
            invalid(
                &format!("{}.at", self.path),
                "required resolved address is missing",
            )
        })
    }

    fn required_text(&mut self, key: &str) -> Result<String, ContractError> {
        self.required(key, |value, shape, path| {
            shape.validate_field(value, path)?;
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, "expected text"))
        })
    }

    fn has(&self, key: &str) -> bool {
        self.object.contains_key(key)
    }

    fn optional_text(&mut self, key: &str) -> Result<Option<String>, ContractError> {
        self.read(key, |value, shape, path| {
            shape.validate_field(value, path)?;
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, "expected text"))
        })
    }

    fn optional_ncname(&mut self, key: &str) -> Result<Option<XmlNcName>, ContractError> {
        let Some(value) = self.optional_text(key)? else {
            return Ok(None);
        };
        if !xml_ncname_is_valid(&value) {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "value must be an XML NCName",
            ));
        }
        Ok(Some(XmlNcName(value)))
    }

    fn optional_qname(&mut self, key: &str) -> Result<Option<PrefixedXmlQName>, ContractError> {
        let Some(value) = self.optional_text(key)? else {
            return Ok(None);
        };
        if !qname_is_valid(&value) {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "value must be a prefixed XML QName",
            ));
        }
        Ok(Some(PrefixedXmlQName(value)))
    }

    fn ncname(&mut self, key: &str) -> Result<XmlNcName, ContractError> {
        let value = self.required_text(key)?;
        if !xml_ncname_is_valid(&value) {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "value must be an XML NCName",
            ));
        }
        Ok(XmlNcName(value))
    }

    fn qname(&mut self, key: &str) -> Result<PrefixedXmlQName, ContractError> {
        let value = self.required_text(key)?;
        if !qname_is_valid(&value) {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "value must be a prefixed XML QName",
            ));
        }
        Ok(PrefixedXmlQName(value))
    }

    /// Facets are read field by field through their declared shapes; the
    /// restrictions between fields are checked once, by `ValueFacets`.
    fn facets(&mut self, key: &str) -> Result<ValueFacets, ContractError> {
        let facets = self.optional_object(key, |facets| {
            let max_length = facets.nonnegative("maxLength")?;
            let total_digits = facets.nonnegative("totalDigits")?;
            let fraction_digits = facets.nonnegative("fractionDigits")?;
            let min_inclusive = facets.optional_text("minInclusive")?;
            let max_inclusive = facets.optional_text("maxInclusive")?;
            ValueFacets::from_parts(
                max_length,
                total_digits,
                fraction_digits,
                min_inclusive,
                max_inclusive,
            )
            .map_err(|(field, message)| invalid(&format!("{}.{field}", facets.path), message))
        })?;
        Ok(facets.unwrap_or_default())
    }

    fn nonnegative(&mut self, key: &str) -> Result<Option<u64>, ContractError> {
        self.read(key, |value, shape, path| {
            let parsed = crate::facets::nonnegative_integer(value)
                .ok_or_else(|| invalid(path, "facet must be a nonnegative integer"))?;
            shape.validate_field(&json!(parsed), path)?;
            Ok(parsed)
        })
    }

    fn min_occurs(&mut self, key: &str) -> Result<Option<u8>, ContractError> {
        self.read(key, |value, shape, path| {
            let parsed = match value.as_f64() {
                Some(0.0) => 0,
                Some(1.0) => 1,
                _ => return Err(invalid(path, "lowerBound must be 0 or 1")),
            };
            shape.validate_field(&json!(parsed), path)?;
            Ok(parsed)
        })
    }

    fn max_occurs(&mut self, key: &str) -> Result<Option<i64>, ContractError> {
        self.read(key, |value, shape, path| {
            // Accept integral values represented exactly by the parsed JSON number;
            // the original decimal spelling is no longer available here.
            let parsed = value.as_i64().or_else(|| {
                value.as_f64().and_then(|number| {
                    // i64::MAX rounds to 2^63 as f64; a saturating cast would
                    // otherwise appear exact at this out-of-range boundary.
                    if number >= i64::MAX as f64 {
                        return None;
                    }
                    let integer = number as i64;
                    (number.fract() == 0.0 && integer as f64 == number).then_some(integer)
                })
            });
            match parsed {
                Some(parsed) if parsed == -1 || parsed >= 1 => {
                    shape.validate_field(&json!(parsed), path)?;
                    Ok(parsed)
                }
                _ => Err(invalid(path, "upperBound must be -1 or a positive integer")),
            }
        })
    }

    fn optional_object<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&mut Self) -> Result<T, ContractError>,
    ) -> Result<Option<T>, ContractError> {
        self.read(key, |value, shape, path| {
            let mut reader = Self::new(value, shape, path)?;
            let parsed = parse(&mut reader)?;
            reader.finish(parsed)
        })
    }

    /// Reads every item of an array field with the given item shape.
    fn required_items<T>(
        &mut self,
        key: &str,
        item: ArgumentShape<'a>,
        mut parse: impl FnMut(&mut Self, &str) -> Result<T, ContractError>,
    ) -> Result<Vec<T>, ContractError> {
        self.required(key, |value, _shape, path| {
            let items = value
                .as_array()
                .ok_or_else(|| invalid(path, "expected an array"))?;
            items
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let item_path = format!("{path}[{index}]");
                    let mut reader = Self::new(value, item, &item_path)?;
                    let parsed = parse(&mut reader, &item_path)?;
                    reader.finish(parsed)
                })
                .collect()
        })
    }

    fn required_object<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&mut Self) -> Result<T, ContractError>,
    ) -> Result<T, ContractError> {
        self.required(key, |value, shape, path| {
            let mut reader = Self::new(value, shape, path)?;
            let parsed = parse(&mut reader)?;
            reader.finish(parsed)
        })
    }

    fn finish<T>(self, parsed: T) -> Result<T, ContractError> {
        for key in self
            .shape
            .property_names()
            .chain(self.shape.aliases())
            .chain(self.shape.required_names())
        {
            if !self.consumed.contains(key) {
                return Err(invalid(
                    &format!("{}.{key}", self.path),
                    "declared field has no decoded operand",
                ));
            }
        }
        Ok(parsed)
    }
}

impl unica_operations::FamilyOperation for XdtoOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Xdto
    }
    fn name(self) -> &'static str {
        self.name()
    }
    fn skeleton(self) -> unica_operation_contract::OperationSkeleton {
        self.skeleton()
    }
    fn address_shape(self) -> &'static str {
        self.applicability().shape()
    }
    fn matches(self, site: &unica_source_contract::context::OperationSite<'_>) -> bool {
        self.applicability().matches(site)
    }
    fn contract(self) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract())
    }
    fn content_resource(self) -> Option<&'static str> {
        Some("Package.bin")
    }
}

/// One parsed element of the `operations` array: the operation kind plus the
/// element's own fields as that operation's writer argument map.
pub type ParsedXdtoOperation = (crate::results::XdtoEditOperation, Map<String, Value>);
/// Split the typed `operations` array into per-element writer inputs. The
/// element's own fields are exactly the argument names the writer has always
/// read, so each element minus its `op` tag is that operation's argument map.
pub fn parse_operations(args: &Map<String, Value>) -> Result<Vec<ParsedXdtoOperation>, String> {
    let operations = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "operations must be a non-empty array".to_string())?;
    if operations.is_empty() {
        return Err("operations must be a non-empty array".to_string());
    }
    operations
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let item = item
                .as_object()
                .ok_or_else(|| format!("operations[{index}]: must be an object"))?;
            let op = item
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("operations[{index}]: requires `op`"))?;
            let operation = crate::results::XdtoEditOperation::parse(op)
                .map_err(|error| format!("operations[{index}]: {error}"))?;
            let mut operation_args = item.clone();
            operation_args.remove("op");
            Ok((operation, operation_args))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(operation: XdtoOperation) -> Value {
        let mut args = operation.contract().example_args;
        args["at"] = json!(match operation {
            XdtoOperation::AddValueType | XdtoOperation::AddObjectType => "main:XDTOPackage.Sample",
            XdtoOperation::AddProperty | XdtoOperation::RemoveType | XdtoOperation::SetType =>
                "main:XDTOPackage.Sample.Type.Target",
            XdtoOperation::RemoveProperty | XdtoOperation::RenameProperty => {
                "main:XDTOPackage.Sample.Type.Target.Property.Child"
            }
        });
        args
    }

    #[test]
    fn published_xdto_contracts_decode_their_operands_and_keep_addressed_removals_empty() {
        for &operation in XdtoOperation::ALL {
            let contract = operation.contract();
            let oracle = jsonschema::validator_for(&contract.schema).unwrap();
            assert!(oracle.is_valid(&contract.example_args), "{operation:?}");
            assert!(contract.example_args.get("at").is_none());
            if matches!(
                operation,
                XdtoOperation::RemoveType | XdtoOperation::RemoveProperty
            ) {
                assert_eq!(contract.example_args, json!({}));
            }
            let args = arguments(operation);
            assert!(oracle.is_valid(&args), "{operation:?}");
            operation.decode(&args, "ops[7].args", "main").unwrap();
            let mut unknown = args;
            unknown["ignored"] = json!(true);
            assert!(!oracle.is_valid(&unknown));
            assert_eq!(
                operation
                    .decode(&unknown, "ops[7].args", "main")
                    .unwrap_err()
                    .path,
                "ops[7].args.ignored"
            );
        }
    }

    #[test]
    fn xdto_object_contract_publishes_base_and_one_exclusive_position() {
        let operation = XdtoOperation::AddObjectType;
        let contract = operation.contract();
        let schema = jsonschema::validator_for(&contract.schema).unwrap();
        for value in [
            json!({"at":"main:XDTOPackage.Sample","values":{"name":"Child","base":"tns:Base","before":"Next"}}),
            json!({"at":"main:XDTOPackage.Sample","values":{"name":"Child","after":"Previous"}}),
            json!({"at":"main:XDTOPackage.Sample","values":{"name":"Child"}}),
        ] {
            assert!(schema.is_valid(&value), "{value}");
            operation.decode(&value, "ops[0].args", "main").unwrap();
        }
        let XdtoPlanOperation::AddObjectType(args) = operation
            .decode(
                &json!({"at":"main:XDTOPackage.Sample","values":{"name":"Child","base":"tns:Base","after":"Previous"}}),
                "ops[0].args",
                "main",
            )
            .unwrap()
        else {
            panic!("objectType.add keeps its typed intent");
        };
        assert_eq!(args.base, Some(PrefixedXmlQName("tns:Base".into())));
        assert_eq!(args.before, None);
        assert_eq!(args.after, Some(XmlNcName("Previous".into())));
        for (value, path) in [
            (
                json!({"values":{"name":"Child","before":"A","after":"B"}}),
                "ops[0].args.values.after",
            ),
            (
                json!({"values":{"name":"Child","base":null}}),
                "ops[0].args.values.base",
            ),
            (
                json!({"values":{"name":"Child","base":"Base"}}),
                "ops[0].args.values.base",
            ),
            (
                json!({"values":{"name":"Child","unknown":true}}),
                "ops[0].args.values.unknown",
            ),
            (json!({"name":"Child"}), "ops[0].args.name"),
        ] {
            let mut value = value;
            value["at"] = json!("main:XDTOPackage.Sample");
            assert!(!schema.is_valid(&value), "{value}");
            assert_eq!(
                operation
                    .decode(&value, "ops[0].args", "main")
                    .unwrap_err()
                    .path,
                path,
                "{value}"
            );
        }
    }

    #[test]
    fn value_facets_are_decoded_through_their_declared_shapes() {
        let operation = XdtoOperation::AddValueType;
        let args = |facets: Value| json!({"at":"main:XDTOPackage.Sample","values":{"name":"V","base":"xs:decimal","facets":facets}});
        let path = |facets: Value| {
            operation
                .decode(&args(facets), "ops[0].args", "main")
                .unwrap_err()
                .path
        };
        assert_eq!(
            path(json!({"maxLength":-1})),
            "ops[0].args.values.facets.maxLength"
        );
        assert_eq!(
            path(json!({"pattern":"x"})),
            "ops[0].args.values.facets.pattern"
        );
        assert_eq!(
            path(json!({"totalDigits":2,"fractionDigits":3})),
            "ops[0].args.values.facets.fractionDigits"
        );
        assert_eq!(
            path(json!({"minInclusive":"2","maxInclusive":"1"})),
            "ops[0].args.values.facets.minInclusive"
        );
        // A narrowed declaration of a facet is honoured by the decoder.
        let mut narrowed = operation.contract();
        narrowed.schema["properties"]["values"]["properties"]["facets"]["properties"]
            ["totalDigits"]["minimum"] = json!(7);
        let error = operation
            .decode_with_contract(
                &narrowed,
                &args(json!({"totalDigits":6})),
                "ops[0].args",
                "main",
            )
            .unwrap_err();
        assert_eq!(error.path, "ops[0].args.values.facets.totalDigits");
        // Every declared facet field is consumed by the decoder.
        let mut extended = operation.contract();
        extended.schema["properties"]["values"]["properties"]["facets"]["properties"]["unwired"] =
            json!({"type":"boolean"});
        let error = operation
            .decode_with_contract(&extended, &args(json!({})), "ops[0].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[0].args.values.facets.unwired");
    }

    #[test]
    fn restricted_values_keep_schema_and_decoder_in_agreement() {
        for operation in [XdtoOperation::AddProperty, XdtoOperation::AddValueType] {
            let contract = operation.contract();
            let schema = jsonschema::validator_for(&contract.schema).unwrap();
            let args = |facets: Value| match operation {
                XdtoOperation::AddProperty => {
                    json!({"at":"main:XDTOPackage.Sample.Type.Target","values":{"property":{"name":"V","valueType":{"base":"xs:decimal","facets":facets}}}})
                }
                _ => {
                    json!({"at":"main:XDTOPackage.Sample","values":{"name":"V","base":"xs:decimal","facets":facets}})
                }
            };
            for facets in [
                json!({}),
                json!({"totalDigits":5,"fractionDigits":2,"minInclusive":"-0.01","maxInclusive":"999.99"}),
                json!({"fractionDigits":0.0,"totalDigits":5e0}),
            ] {
                let value = args(facets);
                assert!(schema.is_valid(&value), "{value}");
                operation.decode(&value, "ops[0].args", "main").unwrap();
            }
            for facets in [
                json!({"minInclusive":"1\n"}),
                json!({"maxInclusive":"1\r\n"}),
                json!({"minInclusive":"1e2"}),
                json!({"minInclusive":1}),
                json!({"totalDigits":0}),
                json!({"fractionDigits":-1}),
                json!({"fractionDigits":0.5}),
                json!({"unknown":1}),
            ] {
                let value = args(facets);
                assert!(!schema.is_valid(&value), "{value}");
                assert!(
                    operation.decode(&value, "ops[0].args", "main").is_err(),
                    "{value}"
                );
            }
        }
        let operation = XdtoOperation::AddProperty;
        let contract = operation.contract();
        let schema = jsonschema::validator_for(&contract.schema).unwrap();
        for definitions in [
            json!({"type":"xs:string","objectType":{"properties":[]}}),
            json!({"type":"xs:string","valueType":{"base":"xs:string"}}),
            json!({"objectType":{"properties":[]},"valueType":{"base":"xs:string"}}),
            json!({"type":"xs:string","objectType":{"properties":[]},"valueType":{"base":"xs:string"}}),
        ] {
            let mut property = definitions;
            property["name"] = json!("V");
            let value =
                json!({"at":"main:XDTOPackage.Sample.Type.Target","values":{"property":property}});
            assert!(!schema.is_valid(&value), "{value}");
            assert!(
                operation.decode(&value, "ops[0].args", "main").is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn xdto_property_contract_validates_nested_definitions() {
        let operation = XdtoOperation::AddProperty;
        let contract = operation.contract();
        let validator = jsonschema::validator_for(&contract.schema).unwrap();
        let valid = json!({"at":"main:XDTOPackage.Sample.Type.Target","values":{"property":{
        "name":"Rows","lowerBound":0,"upperBound":-1,"objectType":{"properties":[
            {"name":"Recipient","type":"tns:Recipient","upperBound":1},
            {"name":"Nested","objectType":{"properties":[{"name":"Code","type":"xs:string"}]}}
        ]}}}});
        assert!(validator.is_valid(&valid));
        let XdtoPlanOperation::AddProperty(args) =
            operation.decode(&valid, "ops[0].args", "main").unwrap()
        else {
            panic!("property.add keeps its typed intent");
        };
        assert_eq!(args.property.max_occurs, Some(-1));
        let XdtoPropertyDefinition::Object(children) = &args.property.definition else {
            panic!("an anonymous ObjectType is decoded");
        };
        assert_eq!(children[0].max_occurs, Some(1));
        assert_eq!(
            children[0].definition,
            XdtoPropertyDefinition::Named(PrefixedXmlQName("tns:Recipient".into()))
        );
        assert!(matches!(
            &children[1].definition,
            XdtoPropertyDefinition::Object(grandchildren) if grandchildren.len() == 1
        ));
        let child = "/values/property/objectType/properties/0";
        for (field, value, path) in [
            (
                "upperBound",
                json!(0),
                "ops[0].args.values.property.objectType.properties[0].upperBound",
            ),
            (
                "upperBound",
                json!(-2),
                "ops[0].args.values.property.objectType.properties[0].upperBound",
            ),
            (
                "unknown",
                json!(true),
                "ops[0].args.values.property.objectType.properties[0].unknown",
            ),
            (
                "objectType",
                json!({"properties":[]}),
                "ops[0].args.values.property.objectType.properties[0]",
            ),
            (
                "type",
                json!("unprefixed"),
                "ops[0].args.values.property.objectType.properties[0].type",
            ),
        ] {
            let mut invalid = valid.clone();
            invalid
                .pointer_mut(child)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), value);
            assert!(!validator.is_valid(&invalid), "{invalid}");
            assert_eq!(
                operation
                    .decode(&invalid, "ops[0].args", "main")
                    .unwrap_err()
                    .path,
                path,
                "{invalid}"
            );
        }
        let mut duplicate = valid.clone();
        duplicate["values"]["property"]["objectType"]["properties"][1] =
            json!({"name":"Recipient","type":"xs:string"});
        duplicate["values"]["property"]["objectType"]["properties"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"Broken","type":"unprefixed"}));
        assert_eq!(
            operation
                .decode(&duplicate, "ops[0].args", "main")
                .unwrap_err()
                .path,
            "ops[0].args.values.property.objectType.properties[1].name"
        );

        // Bounds and the name keep the established diagnostic order even
        // when the definition is missing.
        let at = "main:XDTOPackage.Sample.Type.Target";
        for (property, path) in [
            (
                json!({"name":"X","lowerBound":2}),
                "ops[0].args.values.property.lowerBound",
            ),
            (
                json!({"name":"bad:name"}),
                "ops[0].args.values.property.name",
            ),
            (json!({"name":"X"}), "ops[0].args.values.property"),
        ] {
            let args = json!({"at":at,"values":{"property":property}});
            assert_eq!(
                operation
                    .decode(&args, "ops[0].args", "main")
                    .unwrap_err()
                    .path,
                path,
                "{args}"
            );
        }

        // An integral spelling is exact; a value beyond the integer range is refused.
        let bound = |value: Value| {
            let args = json!({"at":at,"values":{"property":{"name":"X","type":"xs:string","upperBound":value}}});
            match operation.decode(&args, "ops[0].args", "main") {
                Ok(XdtoPlanOperation::AddProperty(args)) => Ok(args.property.max_occurs),
                Ok(other) => panic!("{other:?}"),
                Err(error) => Err(error.path),
            }
        };
        assert_eq!(
            bound(json!(9007199254740993_i64)),
            Ok(Some(9007199254740993))
        );
        assert_eq!(bound(json!(i64::MAX)), Ok(Some(i64::MAX)));
        assert_eq!(
            bound(json!(9223372036854774784.0)),
            Ok(Some(9223372036854774784))
        );
        for value in [
            json!(9223372036854775808_u64),
            json!(9223372036854775808.0),
            json!(u64::MAX),
        ] {
            assert_eq!(
                bound(value),
                Err("ops[0].args.values.property.upperBound".to_string())
            );
        }
        // The transport has already rounded both decimal spellings to 2^63.
        // Refuse that lossy value; an exact integer spelling keeps i64::MAX.
        assert_eq!(
            bound(serde_json::from_str("9223372036854775807").unwrap()),
            Ok(Some(i64::MAX))
        );
        for raw in ["9223372036854775807.0", "9223372036854775808.0"] {
            assert_eq!(
                bound(serde_json::from_str(raw).unwrap()),
                Err("ops[0].args.values.property.upperBound".to_string())
            );
        }
        assert_eq!(bound(json!(2.0)), Ok(Some(2)));
        assert_eq!(bound(json!(-1.0)), Ok(Some(-1)));
        assert_eq!(
            bound(json!(1e19)),
            Err("ops[0].args.values.property.upperBound".to_string())
        );

        // A narrowed declaration of upperBound is honoured by the decoder.
        let mut narrowed = operation.contract();
        narrowed.schema["properties"]["values"]["properties"]["property"]["properties"]
            ["upperBound"]["enum"] = json!([-1]);
        let args =
            json!({"at":at,"values":{"property":{"name":"X","type":"xs:string","upperBound":5}}});
        assert_eq!(
            operation
                .decode_with_contract(&narrowed, &args, "ops[0].args", "main")
                .unwrap_err()
                .path,
            "ops[0].args.values.property.upperBound"
        );
    }

    #[test]
    fn declared_xdto_fields_aliases_defaults_and_types_cannot_escape_consumption() {
        let operation = XdtoOperation::AddProperty;
        let args = arguments(operation);
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["unwired"] =
            json!({"type":"boolean"});
        let error = operation
            .decode_with_contract(&contract, &args, "ops[3].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.unwired");

        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["x-unica-aliases"] = json!(["unusedAlias"]);
        let error = operation
            .decode_with_contract(&contract, &args, "ops[3].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.unusedAlias");

        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["properties"]["type"]
            ["type"] = json!("boolean");
        assert!(operation
            .decode_with_contract(&contract, &args, "ops[3].args", "main")
            .is_err());

        let mut absent = args.clone();
        absent["values"]["property"]
            .as_object_mut()
            .unwrap()
            .remove("lowerBound");
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["properties"]
            ["lowerBound"]["default"] = json!(1);
        let error = operation
            .decode_with_contract(&contract, &absent, "ops[3].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.property.lowerBound");

        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["properties"]
            ["lowerBound"]["enum"] = json!([1]);
        assert!(operation
            .decode_with_contract(&contract, &args, "ops[3].args", "main")
            .is_err());
    }

    #[test]
    fn xdto_requiredness_mutations_are_consumed_without_reordering_operand_errors() {
        let operation = XdtoOperation::AddValueType;
        let args = arguments(operation);
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["required"] = json!(["base"]);
        assert!(jsonschema::validator_for(&contract.schema)
            .unwrap()
            .is_valid(&args));
        let error = operation
            .decode_with_contract(&contract, &args, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.name");

        let operation = XdtoOperation::AddProperty;
        let mut absent = arguments(operation);
        absent["values"]["property"]
            .as_object_mut()
            .unwrap()
            .remove("lowerBound");
        assert!(operation.decode(&absent, "ops[6].args", "main").is_ok());
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["required"] =
            json!(["name", "type", "lowerBound"]);
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        assert!(!oracle.is_valid(&absent));
        let error = operation
            .decode_with_contract(&contract, &absent, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.property.lowerBound");
        absent["values"]["property"]["lowerBound"] = json!(0);
        assert!(oracle.is_valid(&absent));
        assert!(operation
            .decode_with_contract(&contract, &absent, "ops[6].args", "main")
            .is_ok());

        absent["values"]["property"] = json!({"lowerBound":2,"name":"bad:name","type":"bad"});
        let error = operation
            .decode(&absent, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.property.lowerBound");
        let args =
            json!({"at":"main:XDTOPackage.Sample","values":{"name":"bad:name","base":"bad"}});
        let error = XdtoOperation::AddValueType
            .decode(&args, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.name");
    }

    #[test]
    fn xdto_ncname_qname_oracles_and_decoder_share_exact_xml_unicode_ranges() {
        let operation = XdtoOperation::AddValueType;
        let contract = operation.contract();
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        let name_shape = ArgumentShape::new(&contract.schema)
            .property("values")
            .unwrap()
            .property("name")
            .unwrap();
        let names = [
            ("A", true),
            ("_", true),
            ("A.B", true),
            ("\u{00C0}", true),
            ("\u{0370}", true),
            ("\u{200C}", true),
            ("\u{3001}", true),
            ("\u{10000}", true),
            ("\u{EFFFF}", true),
            ("A\u{00B7}", true),
            ("A\u{0300}", true),
            ("A\u{203F}", true),
            ("", false),
            ("1A", false),
            (":A", false),
            ("A:B", false),
            ("A\n", false),
            ("A\r\n", false),
            ("\u{00A0}", false),
            ("A\u{00A0}", false),
            ("\u{00B7}A", false),
            ("\u{0300}A", false),
            ("\u{037E}", false),
            ("\u{F0000}", false),
        ];
        for (name, expected) in names {
            assert_eq!(xml_ncname_is_valid(name), expected, "XML name {name:?}");
            let mut args = arguments(operation);
            args["values"]["name"] = json!(name);
            assert_eq!(oracle.is_valid(&args), expected, "name {name:?}");
            assert_eq!(
                name_shape.validate_field(&json!(name), "name").is_ok(),
                expected,
                "field {name:?}"
            );
            assert_eq!(
                operation.decode(&args, "args", "main").is_ok(),
                expected,
                "decode {name:?}"
            );
            args["values"]["name"] = json!("Valid");
            args["values"]["base"] = json!(format!("{name}:Local"));
            assert_eq!(oracle.is_valid(&args), expected, "QName prefix {name:?}");
            assert_eq!(
                operation.decode(&args, "args", "main").is_ok(),
                expected,
                "QName decode {name:?}"
            );
            args["values"]["base"] = json!(format!("p:{name}"));
            assert_eq!(oracle.is_valid(&args), expected, "QName local {name:?}");
            assert_eq!(
                operation.decode(&args, "args", "main").is_ok(),
                expected,
                "QName local decode {name:?}"
            );
        }
    }

    #[test]
    fn xdto_integer_values_share_schema_and_decoder_without_losing_absence() {
        let operation = XdtoOperation::AddProperty;
        let contract = operation.contract();
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        for literal in [
            "0", "1", "0.0", "0e0", "1.0", "1e0", "-0.0", "0.5", "1.0001", "2", "null", "true",
            "\"1\"",
        ] {
            let mut args = arguments(operation);
            args["values"]["property"]["lowerBound"] = serde_json::from_str(literal).unwrap();
            let decoded = operation.decode(&args, "args", "main");
            assert_eq!(oracle.is_valid(&args), decoded.is_ok(), "{literal}");
        }
        let mut absent = arguments(operation);
        absent["values"]["property"]
            .as_object_mut()
            .unwrap()
            .remove("lowerBound");
        let XdtoPlanOperation::AddProperty(args) =
            operation.decode(&absent, "args", "main").unwrap()
        else {
            panic!("property expected");
        };
        assert_eq!(args.property.min_occurs, None);
        absent["values"]["property"]["lowerBound"] = json!(1);
        let XdtoPlanOperation::AddProperty(args) =
            operation.decode(&absent, "args", "main").unwrap()
        else {
            panic!("property expected");
        };
        assert_eq!(args.property.min_occurs, Some(1));
    }

    #[test]
    fn property_rename_takes_its_target_from_the_address_and_only_the_new_name_from_values() {
        let operation = XdtoOperation::RenameProperty;
        let oracle = jsonschema::validator_for(&operation.contract().schema).unwrap();
        let args = json!({
            "at": r"main:XDTOPackage.Sample.Type.Target.Property.A\.B.Property.Код",
            "values": {"newName": "Внешний.Код"},
        });
        assert!(oracle.is_valid(&args));
        let XdtoPlanOperation::RenameProperty(decoded) =
            operation.decode(&args, "ops[2].args", "main").unwrap()
        else {
            panic!("property.rename keeps its typed intent");
        };
        assert_eq!(decoded.name, XmlNcName("Код".into()));
        assert_eq!(
            decoded.property_path,
            Some(XdtoPropertyPath(r"A\.B".into()))
        );
        assert_eq!(decoded.property_path.unwrap().names(), ["A.B"]);
        assert_eq!(decoded.new_name, XmlNcName("Внешний.Код".into()));
        for (args, path) in [
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код", "values": {"newName": ""}}),
                "ops[2].args.values.newName",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код", "values": {"newName": "1Код"}}),
                "ops[2].args.values.newName",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код", "values": {"newName": "tns:Код"}}),
                "ops[2].args.values.newName",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код", "values": {}}),
                "ops[2].args.values.newName",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код"}),
                "ops[2].args.values",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target", "values": {"newName": "Б"}}),
                "ops[2].args.at",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.Код", "values": {"newName": "Б", "name": "Код"}}),
                "ops[2].args.values.name",
            ),
        ] {
            let error = operation.decode(&args, "ops[2].args", "main").unwrap_err();
            assert_eq!(error.path, path, "{args}");
        }
    }

    #[test]
    fn type_set_takes_the_type_from_the_address_and_only_the_base_from_values() {
        let operation = XdtoOperation::SetType;
        let oracle = jsonschema::validator_for(&operation.contract().schema).unwrap();
        let args =
            json!({"at": "main:XDTOPackage.Sample.Type.Target", "values": {"base": "tns:ОсноваБ"}});
        assert!(oracle.is_valid(&args));
        let XdtoPlanOperation::SetType(decoded) =
            operation.decode(&args, "ops[1].args", "main").unwrap()
        else {
            panic!("type.set keeps its typed intent");
        };
        assert_eq!(decoded.base, PrefixedXmlQName("tns:ОсноваБ".into()));
        for (args, path) in [
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target", "values": {"base": "ОсноваБ"}}),
                "ops[1].args.values.base",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target", "values": {}}),
                "ops[1].args.values.base",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target"}),
                "ops[1].args.values",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target", "values": {"base": "tns:Б", "name": "Б"}}),
                "ops[1].args.values.name",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample", "values": {"base": "tns:Б"}}),
                "ops[1].args.at",
            ),
            (
                json!({"at": "main:XDTOPackage.Sample.Type.Target.Property.P", "values": {"base": "tns:Б"}}),
                "ops[1].args.at",
            ),
        ] {
            assert_eq!(
                operation
                    .decode(&args, "ops[1].args", "main")
                    .unwrap_err()
                    .path,
                path,
                "{args}"
            );
        }
    }
}
