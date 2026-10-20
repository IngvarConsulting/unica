//! The current XDTO mutation profile owns its names, operands and argument grammar.

use super::address::{escape_name, NodeKind, QualifiedAddress};
use super::apply::{OperationApplicability, OperationSkeleton};
use super::operation_contract::{object, text, ArgumentShape, ContractError, OperationContract};
use super::source_target::{prefixed_xml_qname_pattern, xml_ncname_is_valid, xml_ncname_pattern};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

macro_rules! xdto_operations {
    ($($variant:ident => ($name:literal, $site:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum XdtoOperation { $($variant),+ }

        impl XdtoOperation {
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            #[cfg(test)]
            pub(crate) fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }

            const fn site(self) -> XdtoSite {
                match self { $(Self::$variant => XdtoSite::$site),+ }
            }

            pub(super) const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$site),+ }
            }

            pub(super) const fn skeleton(self) -> OperationSkeleton {
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
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// These names are also the applicability identities read from the family table.
// Keeping them equal avoids a second role-name mapping in metadata consumers.
#[allow(clippy::enum_variant_names)]
pub(super) enum XdtoSite {
    XdtoPackage,
    XdtoType,
    XdtoProperty,
    XdtoPropertyContainer,
}

impl XdtoSite {
    pub(super) fn matches(self, at: &QualifiedAddress) -> bool {
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
pub(crate) struct XmlNcName(String);

impl XmlNcName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: &str) -> Self {
        assert!(xml_ncname_is_valid(value));
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrefixedXmlQName(String);

impl PrefixedXmlQName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: &str) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoPropertyPath(String);

impl XdtoPropertyPath {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
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

    #[cfg(test)]
    pub(crate) fn for_test(value: &str) -> Self {
        Self(value.into())
    }
}

/// The supported creation profile for an own property with a QName type.
/// Observed properties and future patch/ref/anonymous-type inputs are distinct
/// profiles; this is not the complete platform property algebra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoPropertySpec {
    pub(crate) name: XmlNcName,
    pub(crate) type_ref: PrefixedXmlQName,
    pub(crate) min_occurs: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoAddValueTypeArgs {
    pub(crate) at: QualifiedAddress,
    pub(crate) name: XmlNcName,
    pub(crate) base: PrefixedXmlQName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoAddObjectTypeArgs {
    pub(crate) at: QualifiedAddress,
    pub(crate) name: XmlNcName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoAddPropertyArgs {
    pub(crate) at: QualifiedAddress,
    pub(crate) property: XdtoPropertySpec,
    pub(crate) property_path: Option<XdtoPropertyPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoRemoveTypeArgs {
    pub(crate) at: QualifiedAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XdtoRemovePropertyArgs {
    pub(crate) at: QualifiedAddress,
    pub(crate) name: XmlNcName,
    pub(crate) property_path: Option<XdtoPropertyPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum XdtoPlanOperation {
    AddValueType(XdtoAddValueTypeArgs),
    AddObjectType(XdtoAddObjectTypeArgs),
    AddProperty(XdtoAddPropertyArgs),
    RemoveType(XdtoRemoveTypeArgs),
    RemoveProperty(XdtoRemovePropertyArgs),
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
    pub(crate) fn contract(self) -> OperationContract {
        let (payload, target, effect, example_args) = match self {
            Self::AddValueType => (
                Some(object(&[("name", ncname()), ("base", qname())], &["name", "base"])),
                "The XDTO package: <source-set>:XDTOPackage.<Name>.",
                "Add a named value type with the supplied prefixed base QName.",
                json!({"values":{"name":"AddedValue", "base":"xs:string"}}),
            ),
            Self::AddObjectType => (
                Some(object(&[("name", ncname())], &["name"])),
                "The XDTO package: <source-set>:XDTOPackage.<Name>.",
                "Add a named empty object type.",
                json!({"values":{"name":"AddedObject"}}),
            ),
            Self::AddProperty => (
                Some(object(&[("property", object(&[
                    ("name", ncname()), ("type", qname()),
                    ("minOccurs", json!({"type":"integer", "enum":[0,1]})),
                ], &["name", "type"]))], &["property"])),
                "An object type or a property that holds one nested anonymous ObjectType: ...Type.T[.Property.P...].",
                "Add one own property with a QName type. Omitted minOccurs stays absent; explicit 0 or 1 stays explicit.",
                json!({"values":{"property":{"name":"AddedProperty", "type":"xs:string", "minOccurs":0}}}),
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
        };
        let schema = match payload {
            Some(payload) => object(&[("at", text(true)), ("values", payload)], &["values"]),
            None => object(&[("at", text(true))], &[]),
        };
        OperationContract {
            schema, target, effect, example_args,
            notes: "Names are XML NCNames and types are prefixed XML QNames. QName bindings and target structure are checked against the selected package. Only Package.bin is edited; exact additions repeat as no-op. Add operands belong in values; remove has no payload. The common args.at address override remains available.",
            help: "skills/xdto/SKILL.md",
        }
    }

    pub(crate) fn decode(
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
                }))
            })?,
            Self::AddObjectType => reader.required_object("values", |values| {
                Ok(XdtoPlanOperation::AddObjectType(XdtoAddObjectTypeArgs {
                    at,
                    name: values.ncname("name")?,
                }))
            })?,
            Self::AddProperty => reader.required_object("values", |values| {
                let property = values.required_object("property", |property| {
                    // Preserve the established operand diagnostic order.
                    let min_occurs = property.min_occurs("minOccurs")?;
                    Ok(XdtoPropertySpec {
                        name: property.ncname("name")?,
                        type_ref: property.qname("type")?,
                        min_occurs,
                    })
                })?;
                Ok(XdtoPlanOperation::AddProperty(XdtoAddPropertyArgs {
                    at,
                    property,
                    property_path: XdtoPropertyPath::from_segments(&properties),
                }))
            })?,
            Self::RemoveType => XdtoPlanOperation::RemoveType(XdtoRemoveTypeArgs { at }),
            Self::RemoveProperty => {
                let (name, parents) = properties.split_last().expect("resolved property target");
                XdtoPlanOperation::RemoveProperty(XdtoRemovePropertyArgs {
                    at,
                    name: XmlNcName(name.clone()),
                    property_path: XdtoPropertyPath::from_segments(parents),
                })
            }
        };
        reader.finish(intent)
    }
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
        let valid = value.split_once(':').is_some_and(|(prefix, local)| {
            xml_ncname_is_valid(prefix) && xml_ncname_is_valid(local)
        });
        if !valid {
            return Err(invalid(
                &format!("{}.{key}", self.path),
                "value must be a prefixed XML QName",
            ));
        }
        Ok(PrefixedXmlQName(value))
    }

    fn min_occurs(&mut self, key: &str) -> Result<Option<u8>, ContractError> {
        self.read(key, |value, shape, path| {
            let parsed = match value.as_f64() {
                Some(0.0) => 0,
                Some(1.0) => 1,
                _ => return Err(invalid(path, "minOccurs must be 0 or 1")),
            };
            shape.validate_field(&json!(parsed), path)?;
            Ok(parsed)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(operation: XdtoOperation) -> Value {
        let mut args = operation.contract().example_args;
        args["at"] = json!(match operation {
            XdtoOperation::AddValueType | XdtoOperation::AddObjectType => "main:XDTOPackage.Sample",
            XdtoOperation::AddProperty | XdtoOperation::RemoveType =>
                "main:XDTOPackage.Sample.Type.Target",
            XdtoOperation::RemoveProperty => "main:XDTOPackage.Sample.Type.Target.Property.Child",
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
            .remove("minOccurs");
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["properties"]
            ["minOccurs"]["default"] = json!(1);
        let error = operation
            .decode_with_contract(&contract, &absent, "ops[3].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.property.minOccurs");

        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["properties"]
            ["minOccurs"]["enum"] = json!([1]);
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
            .remove("minOccurs");
        assert!(operation.decode(&absent, "ops[6].args", "main").is_ok());
        let mut contract = operation.contract();
        contract.schema["properties"]["values"]["properties"]["property"]["required"] =
            json!(["name", "type", "minOccurs"]);
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        assert!(!oracle.is_valid(&absent));
        let error = operation
            .decode_with_contract(&contract, &absent, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.property.minOccurs");
        absent["values"]["property"]["minOccurs"] = json!(0);
        assert!(oracle.is_valid(&absent));
        assert!(operation
            .decode_with_contract(&contract, &absent, "ops[6].args", "main")
            .is_ok());

        absent["values"]["property"] = json!({"minOccurs":2,"name":"bad:name","type":"bad"});
        let error = operation
            .decode(&absent, "ops[6].args", "main")
            .unwrap_err();
        assert_eq!(error.path, "ops[6].args.values.property.minOccurs");
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
            args["values"]["property"]["minOccurs"] = serde_json::from_str(literal).unwrap();
            let decoded = operation.decode(&args, "args", "main");
            assert_eq!(oracle.is_valid(&args), decoded.is_ok(), "{literal}");
        }
        let mut absent = arguments(operation);
        absent["values"]["property"]
            .as_object_mut()
            .unwrap()
            .remove("minOccurs");
        let XdtoPlanOperation::AddProperty(args) =
            operation.decode(&absent, "args", "main").unwrap()
        else {
            panic!("property expected");
        };
        assert_eq!(args.property.min_occurs, None);
        absent["values"]["property"]["minOccurs"] = json!(1);
        let XdtoPlanOperation::AddProperty(args) =
            operation.decode(&absent, "args", "main").unwrap()
        else {
            panic!("property expected");
        };
        assert_eq!(args.property.min_occurs, Some(1));
    }
}
