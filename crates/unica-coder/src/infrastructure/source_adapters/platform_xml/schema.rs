#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObjectsVocabulary {
    None,
    ConfigurationTopLevel,
    Object,
    TabularSection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataClassRole {
    Configuration,
    TopLevelObject,
    Attribute,
    TabularSection,
    Form,
    Template,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataClassProfile {
    pub(crate) class_name: &'static str,
    pub(crate) role: MetadataClassRole,
    pub(crate) child_objects: ChildObjectsVocabulary,
}

pub(crate) const ROOT_STRUCTURAL_CHILDREN: &[&str] = &["Properties", "ChildObjects"];

macro_rules! platform_xml_schema_registry {
    (top_level: [$($top_level:literal),+ $(,)?]) => {
        pub(crate) const LEGACY_TOP_LEVEL_METADATA_CLASSES: &[&str] = &[$($top_level),+];

        pub(crate) const METADATA_CLASS_PROFILES: &[MetadataClassProfile] = &[
            MetadataClassProfile {
                class_name: "Configuration",
                role: MetadataClassRole::Configuration,
                child_objects: ChildObjectsVocabulary::ConfigurationTopLevel,
            },
            $(MetadataClassProfile {
                class_name: $top_level,
                role: MetadataClassRole::TopLevelObject,
                child_objects: ChildObjectsVocabulary::Object,
            },)+
            MetadataClassProfile {
                class_name: "Form",
                role: MetadataClassRole::Form,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "Template",
                role: MetadataClassRole::Template,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "Attribute",
                role: MetadataClassRole::Attribute,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "TabularSection",
                role: MetadataClassRole::TabularSection,
                child_objects: ChildObjectsVocabulary::TabularSection,
            },
            MetadataClassProfile {
                class_name: "Command",
                role: MetadataClassRole::Command,
                child_objects: ChildObjectsVocabulary::None,
            },
        ];
    };
}

platform_xml_schema_registry! {
    top_level: [
        "Language", "Subsystem", "StyleItem", "Style", "CommonPicture", "SessionParameter",
        "Role", "CommonTemplate", "FilterCriterion", "CommonModule", "Bot", "CommonAttribute",
        "ExchangePlan", "XDTOPackage", "WebService", "HTTPService", "WSReference",
        "EventSubscription", "ScheduledJob", "SettingsStorage", "FunctionalOption",
        "FunctionalOptionsParameter", "DefinedType", "CommonCommand", "CommandGroup", "Constant",
        "CommonForm", "Catalog", "Document", "DocumentNumerator", "Sequence", "DocumentJournal",
        "Enum", "Report", "DataProcessor", "InformationRegister", "AccumulationRegister",
        "ChartOfCharacteristicTypes", "ChartOfAccounts", "AccountingRegister",
        "ChartOfCalculationTypes", "CalculationRegister", "BusinessProcess", "Task",
        "IntegrationService"
    ]
}

pub(crate) fn metadata_class_profile(class_name: &str) -> Option<&'static MetadataClassProfile> {
    METADATA_CLASS_PROFILES
        .iter()
        .find(|profile| profile.class_name == class_name)
}

pub(crate) fn child_metadata_class_profile(
    owner: &MetadataClassProfile,
    child_class_name: &str,
) -> Option<&'static MetadataClassProfile> {
    let child = metadata_class_profile(child_class_name)?;
    let allowed = match owner.child_objects {
        ChildObjectsVocabulary::None => false,
        ChildObjectsVocabulary::ConfigurationTopLevel => {
            child.role == MetadataClassRole::TopLevelObject
        }
        ChildObjectsVocabulary::Object => matches!(
            child.role,
            MetadataClassRole::Attribute
                | MetadataClassRole::TabularSection
                | MetadataClassRole::Form
                | MetadataClassRole::Template
                | MetadataClassRole::Command
        ),
        ChildObjectsVocabulary::TabularSection => child.role == MetadataClassRole::Attribute,
    };
    allowed.then_some(child)
}

#[cfg(test)]
mod scalar_tests {
    use crate::infrastructure::{
        metadata_kinds::{METADATA_KINDS, METADATA_KIND_TAGS},
        source_adapters::platform_xml::schema::LEGACY_TOP_LEVEL_METADATA_CLASSES,
    };

    #[test]
    fn legacy_metadata_kind_mapping_uses_the_shared_top_level_class_source() {
        assert_eq!(METADATA_KIND_TAGS, LEGACY_TOP_LEVEL_METADATA_CLASSES);
        assert_eq!(
            METADATA_KINDS.iter().map(|kind| kind.tag).collect::<Vec<_>>(),
            LEGACY_TOP_LEVEL_METADATA_CLASSES,
        );
    }
}
use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node};

use crate::domain::{
    navigation::{PropertyValue, TypeSetValue, TypeVariant},
    source_adapters::{SourceAdapterError, SourceAdapterErrorKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScalarPropertyKind {
    Boolean,
    Integer,
    Uuid,
    String,
    PolymorphicFillValue,
}

/// The scalar subset certified for the exact Platform XML 2.20 projector.
/// Unknown scalar properties remain typed unknown rather than being inferred
/// from their lexical representation.
pub(crate) fn scalar_property_kind_2_20(id: &str) -> Option<ScalarPropertyKind> {
    match id {
        "UseStandardCommands" | "AutoNumbering" | "IncludeHelpInContents" => {
            Some(ScalarPropertyKind::Boolean)
        }
        "NumberLength" | "Length" | "Digits" | "FractionDigits" => {
            Some(ScalarPropertyKind::Integer)
        }
        "Uuid" | "UUID" => Some(ScalarPropertyKind::Uuid),
        "Name" | "Code" | "Description" | "Comment" | "TemplateType" => {
            Some(ScalarPropertyKind::String)
        }
        "FillValue" => Some(ScalarPropertyKind::PolymorphicFillValue),
        _ => None,
    }
}

pub(crate) fn is_type_property_2_20(id: &str) -> bool {
    matches!(id, "Type" | "TypeDescription" | "DataType")
}

/// Parses the bounded 2.20 type-description grammar.  Only direct `Type`
/// members and declared qualifier elements are accepted; arbitrary descendant
/// text can never become a semantic type value.
pub(crate) fn parse_type_description_2_20(raw_xml: &str) -> Result<TypeSetValue, SourceAdapterError> {
    let document = Document::parse(raw_xml).map_err(|_| projection_error("malformed Platform XML type description"))?;
    let root = document.root_element();
    if !matches!(root.tag_name().name(), "TypeDescription" | "DataType" | "Type") {
        return Err(projection_error("unsupported Platform XML type description root"));
    }
    let mut variants = Vec::new();
    let mut qualifiers = BTreeMap::new();
    let mut qualifier_groups = BTreeSet::new();
    if root.tag_name().name() == "Type" {
        variants.push(parse_type_variant(&text_only(root)?)?);
    } else {
        for child in root.children().filter(Node::is_element) {
            match child.tag_name().name() {
                "Type" => variants.push(parse_type_variant(&text_only(child)?)?),
                "StringQualifiers" => parse_qualifier_group(
                    QualifierGroup::String, child, &mut qualifier_groups, &mut qualifiers,
                    &[("Length", QualifierKind::Integer), ("AllowedLength", QualifierKind::AllowedLength)],
                )?,
                "NumberQualifiers" => parse_qualifier_group(
                    QualifierGroup::Number, child, &mut qualifier_groups, &mut qualifiers,
                    &[
                        ("Digits", QualifierKind::Integer),
                        ("FractionDigits", QualifierKind::Integer),
                        ("AllowedSign", QualifierKind::AllowedSign),
                    ],
                )?,
                "DateQualifiers" => parse_qualifier_group(
                    QualifierGroup::Date, child, &mut qualifier_groups, &mut qualifiers,
                    &[("DateFractions", QualifierKind::DateFractions)],
                )?,
                _ => return Err(projection_error("unsupported Platform XML type-description member")),
            }
        }
    }
    if variants.is_empty() {
        return Err(projection_error("Platform XML type description has no variants"));
    }
    if !qualifier_groups.is_empty() {
        let primitive_indexes = variants
            .iter()
            .enumerate()
            .filter_map(|(index, variant)| matches!(variant, TypeVariant::Primitive { .. }).then_some(index))
            .collect::<Vec<_>>();
        if primitive_indexes.len() != 1 {
            return Err(projection_error("type qualifiers require one primitive variant"));
        }
        let TypeVariant::Primitive { kind, .. } = &variants[primitive_indexes[0]] else { unreachable!() };
        if !qualifier_groups.iter().all(|group| group.is_compatible_with(kind)) {
            return Err(projection_error("type qualifier group is incompatible with primitive variant"));
        }
        let TypeVariant::Primitive { qualifiers: destination, .. } = &mut variants[primitive_indexes[0]] else { unreachable!() };
        *destination = qualifiers;
    }
    variants.sort_by_key(type_variant_key);
    variants.dedup();
    Ok(TypeSetValue { variants })
}

#[derive(Clone, Copy)]
enum QualifierKind { Integer, AllowedLength, AllowedSign, DateFractions }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum QualifierGroup { String, Number, Date }

impl QualifierGroup {
    fn is_compatible_with(self, primitive_kind: &str) -> bool {
        matches!(
            (self, primitive_kind),
            (Self::String, "String") | (Self::Number, "Number") | (Self::Date, "Date")
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
        return Err(projection_error("duplicate Platform XML type qualifier group"));
    }
    parse_qualifiers(node, qualifiers, allowed)
}

fn parse_qualifiers(
    node: Node<'_, '_>,
    qualifiers: &mut BTreeMap<String, PropertyValue>,
    allowed: &[(&str, QualifierKind)],
) -> Result<(), SourceAdapterError> {
    for child in node.children().filter(Node::is_element) {
        let Some((_, kind)) = allowed.iter().find(|(name, _)| *name == child.tag_name().name()) else {
            return Err(projection_error("unsupported Platform XML type qualifier"));
        };
        let key = lower_camel(child.tag_name().name());
        if qualifiers.contains_key(&key) {
            return Err(projection_error("duplicate Platform XML type qualifier"));
        }
        let text = text_only(child)?;
        let value = match kind {
            QualifierKind::Integer => PropertyValue::Integer(text.parse().map_err(|_| projection_error("invalid numeric type qualifier"))?),
            QualifierKind::AllowedLength if matches!(text.as_str(), "Fixed" | "Variable") => PropertyValue::EnumSymbol(text.to_string()),
            QualifierKind::AllowedSign if matches!(text.as_str(), "Any" | "Nonnegative") => PropertyValue::EnumSymbol(text.to_string()),
            QualifierKind::DateFractions if matches!(text.as_str(), "Date" | "DateTime" | "Time") => PropertyValue::EnumSymbol(text.to_string()),
            _ => return Err(projection_error("invalid Platform XML type qualifier")),
        };
        qualifiers.insert(key, value);
    }
    Ok(())
}

fn parse_type_variant(value: &str) -> Result<TypeVariant, SourceAdapterError> {
    let value = value.strip_prefix("cfg:").unwrap_or(value);
    let primitive = match value {
        "xs:string" | "String" => Some("String"),
        "xs:boolean" | "Boolean" => Some("Boolean"),
        "xs:decimal" | "Number" => Some("Number"),
        "xs:date" | "xs:dateTime" | "Date" => Some("Date"),
        _ => None,
    };
    if let Some(kind) = primitive {
        return Ok(TypeVariant::Primitive { kind: kind.to_string(), qualifiers: BTreeMap::new() });
    }
    let Some((class, name)) = value.split_once('.') else {
        return Err(projection_error("unsupported Platform XML type variant"));
    };
    if value.split('.').count() != 2 || !is_identifier(name) {
        return Err(projection_error("invalid Platform XML metadata type target"));
    }
    let target = match class {
        "CatalogRef" => format!("Catalog.{name}"),
        "DocumentRef" => format!("Document.{name}"),
        "EnumRef" => return Ok(TypeVariant::Enumeration { target: format!("Enum.{name}") }),
        "ChartOfAccountsRef" => format!("ChartOfAccounts.{name}"),
        "ChartOfCharacteristicTypesRef" => format!("ChartOfCharacteristicTypes.{name}"),
        _ => return Err(projection_error("unsupported Platform XML metadata type class")),
    };
    Ok(TypeVariant::Reference { target })
}

fn text_only(node: Node<'_, '_>) -> Result<String, SourceAdapterError> {
    if node.children().any(|child| child.is_element()) {
        return Err(projection_error("nested Platform XML type value is unsupported"));
    }
    let value = node.text().map(str::trim).filter(|value| !value.is_empty())
        .ok_or_else(|| projection_error("empty Platform XML type value"))?;
    if value.contains(['/', '\\']) || value.contains("..") || value.chars().any(char::is_control) {
        return Err(projection_error("invalid Platform XML type value"));
    }
    Ok(value.to_string())
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_alphabetic() || first == '_')
        && chars.all(|character| character.is_alphanumeric() || character == '_')
}

fn lower_camel(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map(|first| first.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

fn type_variant_key(value: &TypeVariant) -> String {
    match value {
        TypeVariant::Primitive { kind, .. } => format!("primitive:{kind}"),
        TypeVariant::Reference { target } => format!("reference:{target}"),
        TypeVariant::Enumeration { target } => format!("enumeration:{target}"),
    }
}

fn projection_error(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}

#[cfg(test)]
mod tests {
    use super::parse_type_description_2_20;

    #[test]
    fn rejects_empty_alien_qualifier_group() {
        assert!(parse_type_description_2_20(
            "<DataType><Type>xs:string</Type><NumberQualifiers/></DataType>",
        )
        .is_err());
    }

    #[test]
    fn rejects_duplicate_qualifier_group_even_when_one_is_empty() {
        assert!(parse_type_description_2_20(
            "<DataType><Type>xs:string</Type><StringQualifiers/><StringQualifiers><Length>12</Length></StringQualifiers></DataType>",
        )
        .is_err());
    }

    #[test]
    fn rejects_unknown_qualifier_child() {
        assert!(parse_type_description_2_20(
            "<DataType><Type>xs:decimal</Type><NumberQualifiers><Scale>2</Scale></NumberQualifiers></DataType>",
        )
        .is_err());
    }
}
