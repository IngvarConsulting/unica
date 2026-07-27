pub(crate) use super::semantic_map::{
    ChildObjectsVocabulary, MetadataClassProfile, MetadataClassRole,
};

pub(crate) const ROOT_STRUCTURAL_CHILDREN: &[&str] =
    &["InternalInfo", "Properties", "ChildObjects"];

pub(crate) fn metadata_class_profile(class_name: &str) -> Option<&'static MetadataClassProfile> {
    super::semantic_map::metadata_class_profile(class_name)
}

pub(crate) fn legacy_top_level_metadata_classes() -> &'static [&'static str] {
    super::semantic_map::legacy_top_level_metadata_classes()
}

pub(crate) fn child_metadata_class_profile(
    owner: &MetadataClassProfile,
    child_class_name: &str,
) -> Option<&'static MetadataClassProfile> {
    super::semantic_map::child_metadata_class_profile(owner, child_class_name)
}
use std::collections::{BTreeMap, BTreeSet};

use roxmltree::Node;

use super::semantic_map::{NativeTypeNamespace, TypeAliasCategory};
use crate::domain::{
    identifiers::is_1c_identifier,
    navigation::{
        DateFractions, DateQualifiers, NumberQualifiers, NumberSign, PrimitiveTypeKind,
        PropertyValue, SemanticObjectKind, StringLength, StringQualifiers, TypeQualifiers,
        TypeSetValue, TypeVariant,
    },
    navigation_limits::MAX_NAVIGATION_TYPE_VARIANTS,
    source_adapters::{SourceAdapterError, SourceAdapterErrorKind},
};

pub(crate) const METADATA_NAMESPACE_2_20: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Parses the bounded 2.20 type-description grammar.  Only direct `Type`
/// members and declared qualifier elements are accepted; arbitrary descendant
/// text can never become a semantic type value.
const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const DATA_CORE_NAMESPACE: &str = "http://v8.1c.ru/8.1/data/core";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
const CURRENT_CONFIGURATION_NAMESPACE: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

pub(crate) fn parse_type_description_2_20(
    root: Node<'_, '_>,
) -> Result<TypeSetValue, SourceAdapterError> {
    if root.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(projection_error(
            "unsupported Platform XML type description root",
        ));
    }
    let mut variants = Vec::new();
    let mut unknown_ordinal = 0u32;
    let mut qualifiers = BTreeMap::new();
    let mut qualifier_groups = BTreeSet::new();
    let mut children = root.children().filter(Node::is_element).peekable();
    if children.peek().is_none() {
        variants.push(parse_type_variant(
            &text_only(root)?,
            root,
            &mut unknown_ordinal,
        )?);
    } else {
        for child in children {
            if child.tag_name().namespace() != Some(DATA_CORE_NAMESPACE) {
                return Err(projection_error(
                    "Platform XML type-description member has an unsupported namespace",
                ));
            }
            match child.tag_name().name() {
                "Type" | "TypeSet" => {
                    if variants.len() >= MAX_NAVIGATION_TYPE_VARIANTS {
                        return Err(resource_limit(
                            "Platform XML type description has too many variants",
                        ));
                    }
                    variants.push(parse_type_variant(
                        &text_only(child)?,
                        child,
                        &mut unknown_ordinal,
                    )?);
                }
                "StringQualifiers" => parse_qualifier_group(
                    QualifierGroup::String,
                    child,
                    &mut qualifier_groups,
                    &mut qualifiers,
                    &[
                        ("Length", QualifierKind::Integer),
                        ("AllowedLength", QualifierKind::AllowedLength),
                    ],
                )?,
                "NumberQualifiers" => parse_qualifier_group(
                    QualifierGroup::Number,
                    child,
                    &mut qualifier_groups,
                    &mut qualifiers,
                    &[
                        ("Digits", QualifierKind::Integer),
                        ("FractionDigits", QualifierKind::Integer),
                        ("AllowedSign", QualifierKind::AllowedSign),
                    ],
                )?,
                "DateQualifiers" => parse_qualifier_group(
                    QualifierGroup::Date,
                    child,
                    &mut qualifier_groups,
                    &mut qualifiers,
                    &[("DateFractions", QualifierKind::DateFractions)],
                )?,
                _ => {
                    return Err(projection_error(
                        "unsupported Platform XML type-description member",
                    ))
                }
            }
        }
    }
    if variants.is_empty() {
        return Err(projection_error(
            "Platform XML type description has no variants",
        ));
    }
    for group in qualifier_groups {
        let primitive_indexes = variants
            .iter()
            .enumerate()
            .filter_map(|(index, variant)| {
                variant
                    .primitive_kind()
                    .filter(|kind| group.is_compatible_with(kind))
                    .map(|_| index)
            })
            .collect::<Vec<_>>();
        if primitive_indexes.len() != 1 {
            return Err(projection_error(
                "type qualifier group requires one compatible primitive variant",
            ));
        }
        let primitive_index = primitive_indexes[0];
        let kind = variants[primitive_index]
            .primitive_kind()
            .expect("compatible primitive index was selected above");
        let qualifiers = match kind {
            PrimitiveTypeKind::String => TypeQualifiers::String(
                StringQualifiers::new(
                    qualifier_integer(&qualifiers, "length")?,
                    qualifier_string(&qualifiers, "allowedLength")
                        .map(|value| match value {
                            "Fixed" => Ok(StringLength::Fixed),
                            "Variable" => Ok(StringLength::Variable),
                            _ => Err(projection_error("invalid string length qualifier")),
                        })
                        .transpose()?,
                )
                .map_err(|_| projection_error("invalid semantic string qualifiers"))?,
            ),
            PrimitiveTypeKind::Number => TypeQualifiers::Number(
                NumberQualifiers::new(
                    qualifier_integer(&qualifiers, "digits")?,
                    qualifier_integer(&qualifiers, "fractionDigits")?,
                    qualifier_string(&qualifiers, "allowedSign")
                        .map(|value| match value {
                            "Any" => Ok(NumberSign::Any),
                            "Nonnegative" => Ok(NumberSign::Nonnegative),
                            _ => Err(projection_error("invalid number sign qualifier")),
                        })
                        .transpose()?,
                )
                .map_err(|_| projection_error("invalid semantic number qualifiers"))?,
            ),
            PrimitiveTypeKind::Date => TypeQualifiers::Date(
                DateQualifiers::new(
                    qualifier_string(&qualifiers, "dateFractions")
                        .map(|value| match value {
                            "Date" => Ok(DateFractions::Date),
                            "DateTime" => Ok(DateFractions::DateTime),
                            "Time" => Ok(DateFractions::Time),
                            _ => Err(projection_error("invalid date fractions qualifier")),
                        })
                        .transpose()?,
                )
                .map_err(|_| projection_error("invalid semantic date qualifiers"))?,
            ),
            PrimitiveTypeKind::Boolean
            | PrimitiveTypeKind::Uuid
            | PrimitiveTypeKind::Opaque
            | PrimitiveTypeKind::Table
            | PrimitiveTypeKind::Null => {
                return Err(projection_error(
                    "primitive type cannot carry Platform XML qualifiers",
                ))
            }
        };
        variants[primitive_index] = TypeVariant::primitive(kind, Some(qualifiers))
            .map_err(|_| projection_error("invalid qualified semantic primitive"))?;
    }
    variants.sort_by_key(type_variant_key);
    variants.dedup();
    TypeSetValue::new(variants)
        .map_err(|_| projection_error("invalid semantic type-description value"))
}

#[derive(Clone, Copy)]
enum QualifierKind {
    Integer,
    AllowedLength,
    AllowedSign,
    DateFractions,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum QualifierGroup {
    String,
    Number,
    Date,
}

impl QualifierGroup {
    fn is_compatible_with(self, primitive_kind: &PrimitiveTypeKind) -> bool {
        matches!(
            (self, primitive_kind),
            (Self::String, PrimitiveTypeKind::String)
                | (Self::Number, PrimitiveTypeKind::Number)
                | (Self::Date, PrimitiveTypeKind::Date)
        )
    }
}

fn parse_qualifier_group(
    group: QualifierGroup,
    node: Node<'_, '_>,
    groups: &mut BTreeSet<QualifierGroup>,
    qualifiers: &mut BTreeMap<String, PropertyValue>,
    allowed: &[(&str, QualifierKind)],
) -> Result<(), SourceAdapterError> {
    if !groups.insert(group) {
        return Err(projection_error(
            "duplicate Platform XML type qualifier group",
        ));
    }
    let validated_children = qualifiers.len();
    parse_qualifiers(node, qualifiers, allowed)?;
    if qualifiers.len() == validated_children {
        return Err(projection_error("empty Platform XML type qualifier group"));
    }
    Ok(())
}

fn parse_qualifiers(
    node: Node<'_, '_>,
    qualifiers: &mut BTreeMap<String, PropertyValue>,
    allowed: &[(&str, QualifierKind)],
) -> Result<(), SourceAdapterError> {
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(DATA_CORE_NAMESPACE) {
            return Err(projection_error(
                "Platform XML type qualifier has an unsupported namespace",
            ));
        }
        let Some((_, kind)) = allowed
            .iter()
            .find(|(name, _)| *name == child.tag_name().name())
        else {
            return Err(projection_error("unsupported Platform XML type qualifier"));
        };
        let key = lower_camel(child.tag_name().name());
        if qualifiers.contains_key(&key) {
            return Err(projection_error("duplicate Platform XML type qualifier"));
        }
        let text = text_only(child)?;
        let value = match kind {
            QualifierKind::Integer => PropertyValue::Integer(
                text.parse()
                    .map_err(|_| projection_error("invalid numeric type qualifier"))?,
            ),
            QualifierKind::AllowedLength if matches!(text.as_str(), "Fixed" | "Variable") => {
                PropertyValue::String(text.to_string())
            }
            QualifierKind::AllowedSign if matches!(text.as_str(), "Any" | "Nonnegative") => {
                PropertyValue::String(text.to_string())
            }
            QualifierKind::DateFractions
                if matches!(text.as_str(), "Date" | "DateTime" | "Time") =>
            {
                PropertyValue::String(text.to_string())
            }
            _ => return Err(projection_error("invalid Platform XML type qualifier")),
        };
        qualifiers.insert(key, value);
    }
    Ok(())
}

fn parse_type_variant(
    value: &str,
    node: Node<'_, '_>,
    unknown_ordinal: &mut u32,
) -> Result<TypeVariant, SourceAdapterError> {
    let (prefix, local) = value
        .split_once(':')
        .filter(|(prefix, local)| !prefix.is_empty() && !local.is_empty() && !local.contains(':'))
        .ok_or_else(|| projection_error("Platform XML type value must be a qualified name"))?;
    let namespace = node
        .lookup_namespace_uri(Some(prefix))
        .ok_or_else(|| projection_error("Platform XML type value has an undeclared namespace"))?;
    let native_namespace = match namespace {
        XML_SCHEMA_NAMESPACE => NativeTypeNamespace::XmlSchema,
        DATA_CORE_NAMESPACE => NativeTypeNamespace::DataCore,
        CURRENT_CONFIGURATION_NAMESPACE => NativeTypeNamespace::CurrentConfiguration,
        _ => {
            return Err(projection_error(
                "Platform XML type value has an unsupported namespace",
            ))
        }
    };
    let (alias, target) = if native_namespace == NativeTypeNamespace::CurrentConfiguration {
        let Some((alias, target)) = local.split_once('.') else {
            return unknown_type_variant(unknown_ordinal);
        };
        if local.split('.').count() != 2 || !is_1c_identifier(target) {
            return Err(projection_error(
                "invalid Platform XML metadata type target",
            ));
        }
        (alias, Some(target))
    } else {
        (local, None)
    };
    let Some(mapping) = super::semantic_map::type_alias(native_namespace, alias) else {
        return unknown_type_variant(unknown_ordinal);
    };
    match (mapping.category, target) {
        (TypeAliasCategory::Primitive(kind), None) => TypeVariant::primitive(kind, None)
            .map_err(|_| projection_error("invalid semantic primitive type")),
        (TypeAliasCategory::Reference(kind), Some(target)) => TypeVariant::reference(kind, target)
            .map_err(|_| projection_error("invalid semantic reference target")),
        (TypeAliasCategory::Object(kind), Some(target)) => TypeVariant::object(kind, target)
            .map_err(|_| projection_error("invalid semantic object target")),
        (TypeAliasCategory::RecordSet(kind), Some(target)) => TypeVariant::record_set(kind, target)
            .map_err(|_| projection_error("invalid semantic record-set target")),
        (TypeAliasCategory::Manager(kind), Some(target)) => TypeVariant::manager(kind, target)
            .map_err(|_| projection_error("invalid semantic manager target")),
        (TypeAliasCategory::Key(kind), Some(target)) => TypeVariant::key(kind, target)
            .map_err(|_| projection_error("invalid semantic key target")),
        (TypeAliasCategory::Enumeration, Some(target)) => TypeVariant::enumeration(target)
            .map_err(|_| projection_error("invalid semantic enumeration target")),
        (TypeAliasCategory::DefinedType, Some(target)) => TypeVariant::defined_type(target)
            .map_err(|_| projection_error("invalid semantic defined-type target")),
        _ => unknown_type_variant(unknown_ordinal),
    }
}

fn unknown_type_variant(ordinal: &mut u32) -> Result<TypeVariant, SourceAdapterError> {
    *ordinal = ordinal
        .checked_add(1)
        .ok_or_else(|| resource_limit("too many unknown Platform XML type variants"))?;
    TypeVariant::unknown_with_ordinal(*ordinal)
        .map_err(|_| projection_error("invalid semantic unknown type ordinal"))
}

fn text_only(node: Node<'_, '_>) -> Result<String, SourceAdapterError> {
    if node.children().any(|child| child.is_element()) {
        return Err(projection_error(
            "nested Platform XML type value is unsupported",
        ));
    }
    let value = node
        .text()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| projection_error("empty Platform XML type value"))?;
    if value.contains(['/', '\\']) || value.contains("..") || value.chars().any(char::is_control) {
        return Err(projection_error("invalid Platform XML type value"));
    }
    Ok(value.to_string())
}

fn lower_camel(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

fn qualifier_integer(
    qualifiers: &BTreeMap<String, PropertyValue>,
    name: &str,
) -> Result<Option<u32>, SourceAdapterError> {
    qualifiers
        .get(name)
        .map(|value| match value {
            PropertyValue::Integer(value) => u32::try_from(*value)
                .map_err(|_| projection_error("numeric type qualifier is out of range")),
            _ => Err(projection_error("numeric type qualifier has invalid type")),
        })
        .transpose()
}

fn qualifier_string<'a>(
    qualifiers: &'a BTreeMap<String, PropertyValue>,
    name: &str,
) -> Option<&'a str> {
    qualifiers.get(name).and_then(|value| match value {
        PropertyValue::String(value) => Some(value.as_str()),
        _ => None,
    })
}

fn type_variant_key(value: &TypeVariant) -> String {
    if let Some(kind) = value.primitive_kind() {
        return format!("primitive:{}", kind.as_str());
    }
    if value.is_unknown() {
        return "unknown".to_string();
    }
    let target = value
        .target()
        .expect("non-primitive semantic type has a target");
    match target.kind() {
        SemanticObjectKind::Enumeration => format!("enumeration:{}", target.name()),
        SemanticObjectKind::DefinedType => format!("definedType:{}", target.name()),
        kind => format!("reference:{kind}:{}", target.name()),
    }
}

fn projection_error(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}

fn resource_limit(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

#[cfg(test)]
mod tests {
    use roxmltree::Document;

    use super::parse_type_description_2_20;

    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    const V8: &str = "http://v8.1c.ru/8.1/data/core";
    const XS: &str = "http://www.w3.org/2001/XMLSchema";
    const CFG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

    fn parse(
        xml: &str,
    ) -> Result<
        crate::domain::navigation::TypeSetValue,
        crate::domain::source_adapters::SourceAdapterError,
    > {
        let document = Document::parse(xml).expect("test XML");
        parse_type_description_2_20(document.root_element())
    }

    #[test]
    fn rejects_empty_alien_qualifier_group() {
        assert!(parse(&format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:string</v8:Type><v8:NumberQualifiers/></DataType>"))
        .is_err());
    }

    #[test]
    fn rejects_duplicate_qualifier_group_even_when_one_is_empty() {
        assert!(parse(&format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:string</v8:Type><v8:StringQualifiers/><v8:StringQualifiers><v8:Length>12</v8:Length></v8:StringQualifiers></DataType>"))
        .is_err());
    }

    #[test]
    fn rejects_empty_compatible_qualifier_groups_without_raw_error_content() {
        for input in [
            &format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:string</v8:Type><v8:StringQualifiers/></DataType>"),
            &format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers/></DataType>"),
            &format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:date</v8:Type><v8:DateQualifiers/></DataType>"),
        ] {
            let error = parse(input).unwrap_err();
            assert!(!error.message.contains("Qualifiers"));
            assert!(!error.message.contains("xs:"));
        }
    }

    #[test]
    fn rejects_unknown_qualifier_child() {
        assert!(parse(&format!("<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\"><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Scale>2</v8:Scale></v8:NumberQualifiers></DataType>"))
        .is_err());
    }

    #[test]
    fn inherited_official_namespaces_and_aliases_are_normalized() {
        let inherited = format!(
            "<MetaDataObject xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\" xmlns:cfg=\"{CFG}\"><DataType><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>20</v8:Length></v8:StringQualifiers><v8:Type>cfg:CatalogRef.Items</v8:Type></DataType></MetaDataObject>"
        );
        let document = Document::parse(&inherited).unwrap();
        let root = document.root_element().first_element_child().unwrap();
        assert_eq!(
            parse_type_description_2_20(root).unwrap().variants().len(),
            2
        );

        let alias = format!(
            "<DataType xmlns=\"{MD}\" xmlns:core=\"{V8}\" xmlns:schema=\"{XS}\"><core:Type>schema:boolean</core:Type></DataType>"
        );
        assert_eq!(parse(&alias).unwrap().variants().len(), 1);
    }

    #[test]
    fn alien_element_namespace_and_qname_value_fail_closed() {
        let alien_element = format!(
            "<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:xs=\"{XS}\" xmlns:alien=\"urn:alien\"><alien:Type>xs:string</alien:Type></DataType>"
        );
        assert!(parse(&alien_element).is_err());
        let alien_value = format!(
            "<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:alien=\"urn:alien\"><v8:Type>alien:string</v8:Type></DataType>"
        );
        assert!(parse(&alien_value).is_err());
    }

    #[test]
    fn type_reference_and_enumeration_targets_use_the_shared_identifier_grammar() {
        for class in ["CatalogRef", "EnumRef"] {
            let valid = format!(
                "<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:cfg=\"{CFG}\"><v8:Type>cfg:{class}.Ёж_2</v8:Type></DataType>"
            );
            assert!(
                parse(&valid).is_ok(),
                "{class} must accept Russian Cyrillic"
            );
            let invalid = format!(
                "<DataType xmlns=\"{MD}\" xmlns:v8=\"{V8}\" xmlns:cfg=\"{CFG}\"><v8:Type>cfg:{class}.Δelta</v8:Type></DataType>"
            );
            assert!(
                parse(&invalid).is_err(),
                "{class} must reject unsupported scripts"
            );
        }
    }
}
