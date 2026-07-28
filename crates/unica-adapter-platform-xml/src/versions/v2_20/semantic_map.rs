use std::{collections::BTreeSet, sync::OnceLock};

use serde::Deserialize;

use crate::domain::{
    navigation::{
        NodeKind, PrimitiveTypeKind, RelationRole, SemanticEnumValue, SemanticObjectKind,
        SemanticPropertyId, SemanticRelationId,
    },
    source_adapters::{SourceAdapterError, SourceAdapterErrorKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObjectsVocabulary {
    None,
    ConfigurationTopLevel,
    DescriptorReferences,
    Object,
    TabularSection,
    HttpServiceUrlTemplate,
    WebServiceOperation,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataClassRole {
    Configuration,
    TopLevelObject,
    StandaloneObject,
    Attribute,
    Dimension,
    Resource,
    EnumerationValue,
    TabularSection,
    Form,
    Template,
    Command,
    Column,
    HttpServiceUrlTemplate,
    HttpServiceMethod,
    WebServiceOperation,
    WebServiceParameter,
    AccessPermission,
    AccessRestrictionTemplate,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MappingSource {
    Native,
    Derived,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetadataClassProfile {
    pub(crate) class_name: String,
    pub(crate) native_directory: Option<String>,
    pub(crate) display_name_ru: Option<String>,
    pub(crate) role: MetadataClassRole,
    pub(crate) child_objects: ChildObjectsVocabulary,
    pub(crate) kind: NodeKind,
    pub(crate) source: MappingSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeValueKind {
    Boolean,
    Integer,
    Uuid,
    String,
    LocalizedString,
    Enum,
    TypeSet,
    Polymorphic,
    StringList,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PropertyMapping {
    pub(crate) object_kinds: Vec<NodeKind>,
    pub(crate) all_object_kinds: bool,
    pub(crate) native_names: Vec<String>,
    pub(crate) semantic_id: SemanticPropertyId,
    pub(crate) value_kind: NativeValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationPropertyMapping {
    object_kinds: Vec<NodeKind>,
    native_names: Vec<String>,
    role: RelationRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildMapping {
    owner_kinds: Vec<NodeKind>,
    owner_roles: Vec<MetadataClassRole>,
    child_kinds: Vec<NodeKind>,
    child_roles: Vec<MetadataClassRole>,
    relation: RelationRole,
    partial: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnumAlias {
    semantic: SemanticEnumValue,
    native_aliases: Vec<String>,
    property_ids: Vec<SemanticPropertyId>,
    object_kinds: Vec<NodeKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DerivedEnumCase {
    SupportAbsent,
    SupportRemoved,
    SupportEditable,
    SupportLocked,
    SupportConfigurationReadOnly,
}

impl DerivedEnumCase {
    const ALL: [Self; 5] = [
        Self::SupportAbsent,
        Self::SupportRemoved,
        Self::SupportEditable,
        Self::SupportLocked,
        Self::SupportConfigurationReadOnly,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DerivedEnumValue {
    case: DerivedEnumCase,
    semantic: SemanticEnumValue,
    property_id: SemanticPropertyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeTypeNamespace {
    XmlSchema,
    DataCore,
    CurrentConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeAliasCategory {
    Primitive(PrimitiveTypeKind),
    Reference(NodeKind),
    Object(NodeKind),
    RecordSet(NodeKind),
    Manager(NodeKind),
    Key(NodeKind),
    Enumeration,
    DefinedType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeAliasMapping {
    pub(crate) namespace: NativeTypeNamespace,
    pub(crate) alias: String,
    pub(crate) category: TypeAliasCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackingKind {
    Rights,
    Form,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackingMapping {
    pub(crate) object_kinds: Vec<NodeKind>,
    pub(crate) kind: BackingKind,
    pub(crate) descriptor: bool,
    pub(crate) content: bool,
    pub(crate) opaque: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IntentionalPartialCase {
    object_kinds: Vec<NodeKind>,
    reason: IntentionalPartialReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum IntentionalPartialReason {
    OpaqueContent,
    UnknownSemantic,
    UnknownValueVariant,
}

impl IntentionalPartialReason {
    const ALL: [Self; 3] = [
        Self::OpaqueContent,
        Self::UnknownSemantic,
        Self::UnknownValueVariant,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoverageRegistry {
    objects: Vec<MetadataClassProfile>,
    properties: Vec<PropertyMapping>,
    relation_properties: Vec<RelationPropertyMapping>,
    children: Vec<ChildMapping>,
    enum_aliases: Vec<EnumAlias>,
    derived_enum_values: Vec<DerivedEnumValue>,
    type_variants: Vec<TypeAliasMapping>,
    backing_artifacts: Vec<BackingMapping>,
    intentional_partial_cases: Vec<IntentionalPartialCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCoverageRegistry {
    schema_version: u32,
    adapter_id: String,
    objects: Vec<RawObjectMapping>,
    properties: Vec<RawPropertyMapping>,
    relation_properties: Vec<RawRelationPropertyMapping>,
    children: Vec<RawChildMapping>,
    enum_aliases: Vec<RawEnumAlias>,
    derived_enum_values: Vec<RawDerivedEnumValue>,
    type_variants: Vec<RawTypeAlias>,
    backing_artifacts: Vec<RawBackingMapping>,
    intentional_partial_cases: Vec<RawPartialCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawObjectMapping {
    native_class: String,
    #[serde(default)]
    native_directory: Option<String>,
    #[serde(default)]
    display_name_ru: Option<String>,
    kind: String,
    role: String,
    child_vocabulary: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPropertyMapping {
    object_kinds: Vec<String>,
    native_names: Vec<String>,
    semantic_property: String,
    value_kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRelationPropertyMapping {
    object_kinds: Vec<String>,
    native_names: Vec<String>,
    relation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawChildMapping {
    #[serde(default)]
    owner_kinds: Vec<String>,
    #[serde(default)]
    owner_roles: Vec<String>,
    #[serde(default)]
    child_kinds: Vec<String>,
    #[serde(default)]
    child_roles: Vec<String>,
    relation: String,
    partial: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawEnumAlias {
    semantic: String,
    native_aliases: Vec<String>,
    property_ids: Vec<String>,
    object_kinds: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawDerivedEnumValue {
    case: String,
    semantic: String,
    semantic_property: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTypeAlias {
    namespace: String,
    alias: String,
    category: String,
    #[serde(default)]
    target_kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawBackingMapping {
    object_kinds: Vec<String>,
    kind: String,
    descriptor: bool,
    content: bool,
    opaque: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPartialCase {
    object_kinds: Vec<String>,
    reason: String,
}

static REGISTRY: OnceLock<CoverageRegistry> = OnceLock::new();

fn registry() -> &'static CoverageRegistry {
    REGISTRY.get_or_init(|| {
        CoverageRegistry::parse(include_str!("coverage.json"))
            .expect("the embedded Platform XML 2.20 coverage registry must be valid")
    })
}

impl CoverageRegistry {
    fn parse(raw: &str) -> Result<Self, SourceAdapterError> {
        let raw: RawCoverageRegistry = serde_json::from_str(raw)
            .map_err(|_| invalid_registry("coverage registry JSON is invalid"))?;
        if raw.schema_version != 2 || raw.adapter_id != "platform-xml-2.20" {
            return Err(invalid_registry("coverage registry identity is invalid"));
        }
        let registry = Self {
            objects: raw
                .objects
                .into_iter()
                .map(convert_object)
                .collect::<Result<_, _>>()?,
            properties: raw
                .properties
                .into_iter()
                .map(convert_property)
                .collect::<Result<_, _>>()?,
            relation_properties: raw
                .relation_properties
                .into_iter()
                .map(convert_relation_property)
                .collect::<Result<_, _>>()?,
            children: raw
                .children
                .into_iter()
                .map(convert_child)
                .collect::<Result<_, _>>()?,
            enum_aliases: raw
                .enum_aliases
                .into_iter()
                .map(convert_enum)
                .collect::<Result<_, _>>()?,
            derived_enum_values: raw
                .derived_enum_values
                .into_iter()
                .map(convert_derived_enum)
                .collect::<Result<_, _>>()?,
            type_variants: raw
                .type_variants
                .into_iter()
                .map(convert_type)
                .collect::<Result<_, _>>()?,
            backing_artifacts: raw
                .backing_artifacts
                .into_iter()
                .map(convert_backing)
                .collect::<Result<_, _>>()?,
            intentional_partial_cases: raw
                .intentional_partial_cases
                .into_iter()
                .map(convert_partial)
                .collect::<Result<_, _>>()?,
        };
        registry.validate()?;
        Ok(registry)
    }

    fn validate(&self) -> Result<(), SourceAdapterError> {
        ensure_unique(
            self.objects
                .iter()
                .filter(|entry| entry.source == MappingSource::Native)
                .map(|entry| entry.class_name.as_str()),
            "native object mapping",
        )?;
        ensure_unique(
            self.objects
                .iter()
                .filter(|entry| entry.source == MappingSource::Derived)
                .map(|entry| entry.kind),
            "derived object mapping",
        )?;
        ensure_unique(
            self.objects
                .iter()
                .filter_map(|entry| entry.native_directory.as_deref())
                .filter(|directory| !directory.is_empty()),
            "native top-level directory",
        )?;
        for object in &self.objects {
            let owns_directory = object.source == MappingSource::Native
                && matches!(
                    object.role,
                    MetadataClassRole::Configuration | MetadataClassRole::TopLevelObject
                );
            if owns_directory != object.native_directory.is_some()
                || matches!(object.role, MetadataClassRole::Configuration)
                    && object.native_directory.as_deref() != Some("")
                || matches!(object.role, MetadataClassRole::TopLevelObject)
                    && object.native_directory.as_deref().is_none_or(str::is_empty)
            {
                return Err(invalid_registry(
                    "native artifact directory ownership is inconsistent",
                ));
            }
            if matches!(object.role, MetadataClassRole::TopLevelObject)
                != object
                    .display_name_ru
                    .as_deref()
                    .is_some_and(|label| !label.trim().is_empty())
            {
                return Err(invalid_registry(
                    "top-level semantic display mapping is inconsistent",
                ));
            }
        }
        if self
            .objects
            .iter()
            .filter(|entry| entry.source == MappingSource::Unknown)
            .count()
            != 1
        {
            return Err(invalid_registry(
                "coverage registry must have exactly one unknown object mapping",
            ));
        }
        ensure_unique(
            self.properties.iter().flat_map(|entry| {
                entry.native_names.iter().flat_map(move |name| {
                    if entry.all_object_kinds {
                        vec![format!("*:{name}")]
                    } else {
                        entry
                            .object_kinds
                            .iter()
                            .map(|kind| format!("{}:{name}", kind.as_str()))
                            .collect()
                    }
                })
            }),
            "per-kind property mapping",
        )?;
        ensure_unique(
            self.relation_properties.iter().flat_map(|entry| {
                entry.object_kinds.iter().flat_map(move |kind| {
                    entry
                        .native_names
                        .iter()
                        .map(move |name| format!("{}:{name}", kind.as_str()))
                })
            }),
            "relation-property mapping",
        )?;
        ensure_unique(
            self.enum_aliases.iter().flat_map(|entry| {
                entry.native_aliases.iter().flat_map(move |native| {
                    entry.property_ids.iter().flat_map(move |property| {
                        entry.object_kinds.iter().map(move |kind| {
                            format!("{native}:{}:{}", property.as_str(), kind.as_str())
                        })
                    })
                })
            }),
            "enum alias/property/owner mapping",
        )?;
        ensure_unique(
            self.derived_enum_values.iter().map(|entry| entry.case),
            "derived enum case",
        )?;
        ensure_unique(
            self.derived_enum_values.iter().map(|entry| entry.semantic),
            "derived semantic enum mapping",
        )?;
        ensure_unique(
            self.type_variants
                .iter()
                .map(|entry| format!("{:?}:{}", entry.namespace, entry.alias)),
            "type alias",
        )?;
        ensure_unique(
            self.backing_artifacts
                .iter()
                .flat_map(|entry| entry.object_kinds.iter().map(|kind| kind.as_str())),
            "backing mapping",
        )?;
        ensure_unique(
            self.intentional_partial_cases.iter().flat_map(|entry| {
                entry
                    .object_kinds
                    .iter()
                    .map(move |kind| (*kind, entry.reason))
            }),
            "intentional partial case",
        )?;
        if self.objects.is_empty()
            || self.properties.is_empty()
            || self.relation_properties.is_empty()
            || self.children.is_empty()
            || self.enum_aliases.is_empty()
            || self.derived_enum_values.is_empty()
            || self.type_variants.is_empty()
            || self.backing_artifacts.is_empty()
            || self.intentional_partial_cases.is_empty()
        {
            return Err(invalid_registry(
                "coverage registry has an empty required section",
            ));
        }
        let covered_enums = self
            .enum_aliases
            .iter()
            .map(|entry| entry.semantic)
            .chain(self.derived_enum_values.iter().map(|entry| entry.semantic))
            .collect::<BTreeSet<_>>();
        let closed_enums = SemanticEnumValue::ALL
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if covered_enums != closed_enums {
            return Err(invalid_registry(
                "coverage registry enum inventory is not exhaustive",
            ));
        }
        let derived_cases = self
            .derived_enum_values
            .iter()
            .map(|entry| entry.case)
            .collect::<BTreeSet<_>>();
        if derived_cases
            != DerivedEnumCase::ALL
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        {
            return Err(invalid_registry(
                "coverage registry derived enum cases are not exhaustive",
            ));
        }
        for entry in &self.derived_enum_values {
            if entry.property_id != SemanticPropertyId::SUPPORT_STATE
                || crate::domain::navigation::property_definition(entry.property_id).allowed_types()
                    != [crate::domain::navigation::PropertyType::Enum]
            {
                return Err(invalid_registry(
                    "coverage registry derived enum applicability is invalid",
                ));
            }
        }
        for alias in &self.enum_aliases {
            if alias.native_aliases.is_empty()
                || alias
                    .native_aliases
                    .iter()
                    .any(|value| value.trim().is_empty())
                || alias.property_ids.is_empty()
                || alias.object_kinds.is_empty()
            {
                return Err(invalid_registry(
                    "coverage registry enum aliases and applicability must be nonempty",
                ));
            }
            for property_id in &alias.property_ids {
                if !self.properties.iter().any(|property| {
                    property.semantic_id == *property_id
                        && property.value_kind == NativeValueKind::Enum
                }) {
                    return Err(invalid_registry(
                        "coverage registry enum applicability is not an enum property",
                    ));
                }
            }
            for kind in &alias.object_kinds {
                if !self.properties.iter().any(|property| {
                    property.value_kind == NativeValueKind::Enum
                        && alias.property_ids.contains(&property.semantic_id)
                        && property_applies_to(property, *kind)
                }) {
                    return Err(invalid_registry(
                        "coverage registry enum owner has no applicable enum property",
                    ));
                }
            }
        }
        for property in self
            .properties
            .iter()
            .filter(|property| property.value_kind == NativeValueKind::Enum)
        {
            if property.all_object_kinds {
                return Err(invalid_registry(
                    "coverage registry enum properties must have exact owner applicability",
                ));
            }
            for kind in &property.object_kinds {
                if !self.enum_aliases.iter().any(|alias| {
                    alias.property_ids.contains(&property.semantic_id)
                        && alias.object_kinds.contains(kind)
                }) {
                    return Err(invalid_registry(
                        "coverage registry enum property owner has no declared aliases",
                    ));
                }
            }
        }
        let partial_reasons = self
            .intentional_partial_cases
            .iter()
            .map(|entry| entry.reason)
            .collect::<BTreeSet<_>>();
        if partial_reasons
            != IntentionalPartialReason::ALL
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        {
            return Err(invalid_registry(
                "coverage registry intentional-partial reasons are not exhaustive",
            ));
        }
        Ok(())
    }
}

fn ensure_unique<T: Ord>(
    values: impl IntoIterator<Item = T>,
    label: &str,
) -> Result<(), SourceAdapterError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid_registry(&format!(
                "coverage registry has duplicate {label}"
            )));
        }
    }
    Ok(())
}

fn property_applies_to(property: &PropertyMapping, kind: NodeKind) -> bool {
    property.all_object_kinds || property.object_kinds.contains(&kind)
}

fn parse_kind(raw: &str) -> Result<NodeKind, SourceAdapterError> {
    SemanticObjectKind::parse(raw)
        .ok_or_else(|| invalid_registry("coverage registry object kind is not closed"))
}

fn parse_kinds(raw: Vec<String>) -> Result<(Vec<NodeKind>, bool), SourceAdapterError> {
    if raw.is_empty() {
        return Err(invalid_registry(
            "coverage registry applicability must be nonempty",
        ));
    }
    if raw == ["*"] {
        return Ok((Vec::new(), true));
    }
    if raw.iter().any(|value| value == "*") {
        return Err(invalid_registry(
            "coverage registry wildcard applicability must stand alone",
        ));
    }
    Ok((
        raw.iter()
            .map(|value| parse_kind(value))
            .collect::<Result<_, _>>()?,
        false,
    ))
}

fn parse_role(raw: &str) -> Result<MetadataClassRole, SourceAdapterError> {
    match raw {
        "configuration" => Ok(MetadataClassRole::Configuration),
        "topLevelObject" => Ok(MetadataClassRole::TopLevelObject),
        "standaloneObject" => Ok(MetadataClassRole::StandaloneObject),
        "attribute" => Ok(MetadataClassRole::Attribute),
        "dimension" => Ok(MetadataClassRole::Dimension),
        "resource" => Ok(MetadataClassRole::Resource),
        "enumerationValue" => Ok(MetadataClassRole::EnumerationValue),
        "tabularSection" => Ok(MetadataClassRole::TabularSection),
        "form" => Ok(MetadataClassRole::Form),
        "template" => Ok(MetadataClassRole::Template),
        "command" => Ok(MetadataClassRole::Command),
        "column" => Ok(MetadataClassRole::Column),
        "httpServiceUrlTemplate" => Ok(MetadataClassRole::HttpServiceUrlTemplate),
        "httpServiceMethod" => Ok(MetadataClassRole::HttpServiceMethod),
        "webServiceOperation" => Ok(MetadataClassRole::WebServiceOperation),
        "webServiceParameter" => Ok(MetadataClassRole::WebServiceParameter),
        "accessPermission" => Ok(MetadataClassRole::AccessPermission),
        "accessRestrictionTemplate" => Ok(MetadataClassRole::AccessRestrictionTemplate),
        "unsupported" => Ok(MetadataClassRole::Unsupported),
        "unknown" => Ok(MetadataClassRole::Unknown),
        _ => Err(invalid_registry(
            "coverage registry metadata role is invalid",
        )),
    }
}

fn parse_child_vocabulary(raw: &str) -> Result<ChildObjectsVocabulary, SourceAdapterError> {
    match raw {
        "none" => Ok(ChildObjectsVocabulary::None),
        "configurationTopLevel" => Ok(ChildObjectsVocabulary::ConfigurationTopLevel),
        "descriptorReferences" => Ok(ChildObjectsVocabulary::DescriptorReferences),
        "object" => Ok(ChildObjectsVocabulary::Object),
        "tabularSection" => Ok(ChildObjectsVocabulary::TabularSection),
        "httpServiceUrlTemplate" => Ok(ChildObjectsVocabulary::HttpServiceUrlTemplate),
        "webServiceOperation" => Ok(ChildObjectsVocabulary::WebServiceOperation),
        "unknown" => Ok(ChildObjectsVocabulary::Unknown),
        _ => Err(invalid_registry(
            "coverage registry child vocabulary is invalid",
        )),
    }
}

fn convert_object(raw: RawObjectMapping) -> Result<MetadataClassProfile, SourceAdapterError> {
    if raw.native_class.trim().is_empty() {
        return Err(invalid_registry(
            "coverage registry native object class must be nonempty",
        ));
    }
    Ok(MetadataClassProfile {
        class_name: raw.native_class,
        native_directory: raw.native_directory,
        display_name_ru: raw.display_name_ru,
        kind: parse_kind(&raw.kind)?,
        role: parse_role(&raw.role)?,
        child_objects: parse_child_vocabulary(&raw.child_vocabulary)?,
        source: match raw.source.as_str() {
            "native" => MappingSource::Native,
            "derived" => MappingSource::Derived,
            "unknown" => MappingSource::Unknown,
            _ => {
                return Err(invalid_registry(
                    "coverage registry mapping source is invalid",
                ))
            }
        },
    })
}

fn convert_property(raw: RawPropertyMapping) -> Result<PropertyMapping, SourceAdapterError> {
    if raw.native_names.is_empty() || raw.native_names.iter().any(|value| value.trim().is_empty()) {
        return Err(invalid_registry(
            "coverage registry property aliases must be nonempty",
        ));
    }
    let (object_kinds, all_object_kinds) = parse_kinds(raw.object_kinds)?;
    Ok(PropertyMapping {
        object_kinds,
        all_object_kinds,
        native_names: raw.native_names,
        semantic_id: SemanticPropertyId::parse(&raw.semantic_property)
            .ok_or_else(|| invalid_registry("coverage registry property is not closed"))?,
        value_kind: match raw.value_kind.as_str() {
            "boolean" => NativeValueKind::Boolean,
            "integer" => NativeValueKind::Integer,
            "uuid" => NativeValueKind::Uuid,
            "string" => NativeValueKind::String,
            "localizedString" => NativeValueKind::LocalizedString,
            "enum" => NativeValueKind::Enum,
            "typeSet" => NativeValueKind::TypeSet,
            "polymorphic" => NativeValueKind::Polymorphic,
            "stringList" => NativeValueKind::StringList,
            _ => {
                return Err(invalid_registry(
                    "coverage registry native value kind is invalid",
                ))
            }
        },
    })
}

fn convert_relation_property(
    raw: RawRelationPropertyMapping,
) -> Result<RelationPropertyMapping, SourceAdapterError> {
    if raw.native_names.is_empty() || raw.native_names.iter().any(|value| value.trim().is_empty()) {
        return Err(invalid_registry(
            "coverage registry relation aliases must be nonempty",
        ));
    }
    let (object_kinds, all) = parse_kinds(raw.object_kinds)?;
    if all {
        return Err(invalid_registry(
            "relation-property mapping must be per-kind",
        ));
    }
    Ok(RelationPropertyMapping {
        object_kinds,
        native_names: raw.native_names,
        role: SemanticRelationId::parse(&raw.relation)
            .ok_or_else(|| invalid_registry("coverage registry relation is not closed"))?,
    })
}

fn convert_child(raw: RawChildMapping) -> Result<ChildMapping, SourceAdapterError> {
    if raw.owner_kinds.is_empty() && raw.owner_roles.is_empty() {
        return Err(invalid_registry(
            "coverage registry child owner applicability must be nonempty",
        ));
    }
    if raw.child_kinds.is_empty() && raw.child_roles.is_empty() && raw.owner_roles != ["unknown"] {
        return Err(invalid_registry(
            "coverage registry child applicability must be nonempty",
        ));
    }
    Ok(ChildMapping {
        owner_kinds: raw
            .owner_kinds
            .iter()
            .map(|value| parse_kind(value))
            .collect::<Result<_, _>>()?,
        owner_roles: raw
            .owner_roles
            .iter()
            .map(|value| parse_role(value))
            .collect::<Result<_, _>>()?,
        child_kinds: raw
            .child_kinds
            .iter()
            .map(|value| parse_kind(value))
            .collect::<Result<_, _>>()?,
        child_roles: raw
            .child_roles
            .iter()
            .map(|value| parse_role(value))
            .collect::<Result<_, _>>()?,
        relation: SemanticRelationId::parse(&raw.relation)
            .ok_or_else(|| invalid_registry("coverage registry child relation is not closed"))?,
        partial: raw.partial,
    })
}

fn convert_enum(raw: RawEnumAlias) -> Result<EnumAlias, SourceAdapterError> {
    Ok(EnumAlias {
        semantic: SemanticEnumValue::parse(&raw.semantic)
            .ok_or_else(|| invalid_registry("coverage registry enum is not closed"))?,
        native_aliases: raw.native_aliases,
        property_ids: raw
            .property_ids
            .iter()
            .map(|value| {
                SemanticPropertyId::parse(value).ok_or_else(|| {
                    invalid_registry("coverage registry enum property is not closed")
                })
            })
            .collect::<Result<_, _>>()?,
        object_kinds: raw
            .object_kinds
            .iter()
            .map(|value| parse_kind(value))
            .collect::<Result<_, _>>()?,
    })
}

fn convert_derived_enum(raw: RawDerivedEnumValue) -> Result<DerivedEnumValue, SourceAdapterError> {
    let case = match raw.case.as_str() {
        "supportAbsent" => DerivedEnumCase::SupportAbsent,
        "supportRemoved" => DerivedEnumCase::SupportRemoved,
        "supportEditable" => DerivedEnumCase::SupportEditable,
        "supportLocked" => DerivedEnumCase::SupportLocked,
        "supportConfigurationReadOnly" => DerivedEnumCase::SupportConfigurationReadOnly,
        _ => {
            return Err(invalid_registry(
                "coverage registry derived enum case is not closed",
            ))
        }
    };
    Ok(DerivedEnumValue {
        case,
        semantic: SemanticEnumValue::parse(&raw.semantic).ok_or_else(|| {
            invalid_registry("coverage registry derived enum value is not closed")
        })?,
        property_id: SemanticPropertyId::parse(&raw.semantic_property).ok_or_else(|| {
            invalid_registry("coverage registry derived enum property is not closed")
        })?,
    })
}

fn convert_type(raw: RawTypeAlias) -> Result<TypeAliasMapping, SourceAdapterError> {
    if raw.alias.trim().is_empty() {
        return Err(invalid_registry(
            "coverage registry type alias must be nonempty",
        ));
    }
    let namespace = match raw.namespace.as_str() {
        "xmlSchema" => NativeTypeNamespace::XmlSchema,
        "dataCore" => NativeTypeNamespace::DataCore,
        "currentConfiguration" => NativeTypeNamespace::CurrentConfiguration,
        _ => {
            return Err(invalid_registry(
                "coverage registry type namespace is invalid",
            ))
        }
    };
    let target_kind = || {
        raw.target_kind
            .as_deref()
            .ok_or_else(|| invalid_registry("coverage registry target type has no kind"))
            .and_then(parse_kind)
    };
    let category = match raw.category.as_str() {
        "boolean" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Boolean),
        "string" => TypeAliasCategory::Primitive(PrimitiveTypeKind::String),
        "number" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Number),
        "date" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Date),
        "uuid" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Uuid),
        "opaque" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Opaque),
        "table" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Table),
        "null" => TypeAliasCategory::Primitive(PrimitiveTypeKind::Null),
        "reference" => TypeAliasCategory::Reference(target_kind()?),
        "object" => TypeAliasCategory::Object(target_kind()?),
        "recordSet" => TypeAliasCategory::RecordSet(target_kind()?),
        "manager" => TypeAliasCategory::Manager(target_kind()?),
        "key" => TypeAliasCategory::Key(target_kind()?),
        "enumeration" => TypeAliasCategory::Enumeration,
        "definedType" => TypeAliasCategory::DefinedType,
        _ => {
            return Err(invalid_registry(
                "coverage registry type category is invalid",
            ))
        }
    };
    Ok(TypeAliasMapping {
        namespace,
        alias: raw.alias,
        category,
    })
}

fn convert_backing(raw: RawBackingMapping) -> Result<BackingMapping, SourceAdapterError> {
    let (object_kinds, all) = parse_kinds(raw.object_kinds)?;
    if all {
        return Err(invalid_registry("backing mapping must be per-kind"));
    }
    Ok(BackingMapping {
        object_kinds,
        kind: match raw.kind.as_str() {
            "rights" => BackingKind::Rights,
            "form" => BackingKind::Form,
            "template" => BackingKind::Template,
            _ => {
                return Err(invalid_registry(
                    "coverage registry backing kind is invalid",
                ))
            }
        },
        descriptor: raw.descriptor,
        content: raw.content,
        opaque: raw.opaque,
    })
}

fn convert_partial(raw: RawPartialCase) -> Result<IntentionalPartialCase, SourceAdapterError> {
    let (object_kinds, all) = parse_kinds(raw.object_kinds)?;
    let reason = match raw.reason.as_str() {
        "opaqueContent" => IntentionalPartialReason::OpaqueContent,
        "unknownSemantic" => IntentionalPartialReason::UnknownSemantic,
        "unknownValueVariant" => IntentionalPartialReason::UnknownValueVariant,
        _ => return Err(invalid_registry("intentional partial case is invalid")),
    };
    if all {
        return Err(invalid_registry("intentional partial case is invalid"));
    }
    Ok(IntentionalPartialCase {
        object_kinds,
        reason,
    })
}

pub(crate) fn validate_coverage_registry() -> Result<(), SourceAdapterError> {
    let registry = registry();
    for object in &registry.objects {
        match object.source {
            MappingSource::Native
                if metadata_class_profile(&object.class_name).map(|entry| entry.kind)
                    != Some(object.kind) =>
            {
                return Err(invalid_registry(
                    "native object registry lookup is not bijective",
                ));
            }
            MappingSource::Derived if derived_profile(object.kind) != object => {
                return Err(invalid_registry(
                    "derived object registry lookup is not bijective",
                ));
            }
            MappingSource::Unknown if unknown_metadata_class_profile() != object => {
                return Err(invalid_registry(
                    "unknown object registry lookup is not bijective",
                ));
            }
            _ => {}
        }
    }
    for property in &registry.properties {
        let kinds = if property.all_object_kinds {
            SemanticObjectKind::ALL
        } else {
            &property.object_kinds
        };
        for kind in kinds {
            for name in &property.native_names {
                let found = property_mapping(*kind, name).ok_or_else(|| {
                    invalid_registry("property registry lookup is not exhaustive")
                })?;
                if found.semantic_id != property.semantic_id
                    || found.value_kind != property.value_kind
                {
                    return Err(invalid_registry(
                        "property registry lookup is not bijective",
                    ));
                }
            }
        }
    }
    for relation in &registry.relation_properties {
        for kind in &relation.object_kinds {
            for name in &relation.native_names {
                if relation_property_role(*kind, name) != Some(relation.role) {
                    return Err(invalid_registry(
                        "relation-property registry lookup is not bijective",
                    ));
                }
            }
        }
    }
    let mut exercised_children = BTreeSet::new();
    for owner in &registry.objects {
        for child in &registry.objects {
            let matches = registry
                .children
                .iter()
                .enumerate()
                .filter(|(_, entry)| child_mapping_matches(entry, owner, child))
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err(invalid_registry(
                    "child registry has overlapping runtime mappings",
                ));
            }
            if let Some((index, entry)) = matches.first() {
                exercised_children.insert(*index);
                if child_relation_role(owner, child) != Some(entry.relation)
                    || child_mapping_is_partial(owner, child) != entry.partial
                {
                    return Err(invalid_registry("child registry lookup is not bijective"));
                }
            }
        }
    }
    if exercised_children.len() != registry.children.len() {
        return Err(invalid_registry(
            "child registry contains a mapping unused by runtime profiles",
        ));
    }
    for alias in &registry.enum_aliases {
        let mut exercised = false;
        for property_id in &alias.property_ids {
            for kind in &alias.object_kinds {
                if registry.properties.iter().any(|property| {
                    property.semantic_id == *property_id
                        && property.value_kind == NativeValueKind::Enum
                        && property_applies_to(property, *kind)
                }) {
                    exercised = true;
                    for native in &alias.native_aliases {
                        if enum_value(*kind, *property_id, native) != Some(alias.semantic) {
                            return Err(invalid_registry("enum registry lookup is not bijective"));
                        }
                    }
                }
            }
        }
        if !exercised {
            return Err(invalid_registry(
                "enum registry contains applicability unused by runtime properties",
            ));
        }
    }
    let native_aliases = registry
        .enum_aliases
        .iter()
        .flat_map(|entry| entry.native_aliases.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for property in registry
        .properties
        .iter()
        .filter(|property| property.value_kind == NativeValueKind::Enum)
    {
        for kind in &property.object_kinds {
            for native in &native_aliases {
                let expected = registry
                    .enum_aliases
                    .iter()
                    .find(|entry| {
                        entry.property_ids.contains(&property.semantic_id)
                            && entry.object_kinds.contains(kind)
                            && entry.native_aliases.iter().any(|alias| alias == native)
                    })
                    .map(|entry| entry.semantic);
                if enum_value(*kind, property.semantic_id, native) != expected {
                    return Err(invalid_registry(
                        "enum registry accepts an alias outside its exact property or owner context",
                    ));
                }
            }
        }
    }
    for entry in &registry.derived_enum_values {
        if derived_enum_value(entry.case) != Some(entry.semantic) {
            return Err(invalid_registry(
                "derived enum registry lookup is not bijective",
            ));
        }
    }
    for alias in &registry.type_variants {
        if type_alias(alias.namespace, &alias.alias) != Some(alias) {
            return Err(invalid_registry("type registry lookup is not bijective"));
        }
    }
    for backing in &registry.backing_artifacts {
        for kind in &backing.object_kinds {
            if backing_mapping(*kind) != Some(backing) {
                return Err(invalid_registry("backing registry lookup is not bijective"));
            }
        }
    }
    for partial in &registry.intentional_partial_cases {
        for kind in &partial.object_kinds {
            if !intentional_partial_reasons(*kind).any(|reason| reason == partial.reason) {
                return Err(invalid_registry(
                    "intentional-partial registry lookup is not bijective",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_coverage_manifest(raw: &str) -> Result<(), SourceAdapterError> {
    let candidate = CoverageRegistry::parse(raw)?;
    if candidate != *registry() {
        return Err(invalid_registry(
            "coverage manifest differs from the runtime registry",
        ));
    }
    validate_coverage_registry()
}

pub(crate) fn metadata_class_profiles() -> &'static [MetadataClassProfile] {
    &registry().objects
}

pub(crate) fn top_level_descriptor_profiles() -> impl Iterator<Item = &'static MetadataClassProfile>
{
    registry().objects.iter().filter(|entry| {
        entry.source == MappingSource::Native
            && entry.role == MetadataClassRole::TopLevelObject
            && entry.native_directory.is_some()
    })
}

pub(crate) fn native_descriptor_directory(kind: NodeKind) -> Option<&'static str> {
    registry()
        .objects
        .iter()
        .find(|entry| {
            entry.source == MappingSource::Native
                && entry.kind == kind
                && entry.role == MetadataClassRole::TopLevelObject
        })
        .and_then(|entry| entry.native_directory.as_deref())
}

pub(crate) fn metadata_class_profile_by_directory(
    directory: &str,
) -> Option<&'static MetadataClassProfile> {
    registry().objects.iter().find(|entry| {
        entry.source == MappingSource::Native
            && entry
                .native_directory
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(directory))
    })
}

pub(crate) fn writer_object_kind(value: &str) -> Option<NodeKind> {
    metadata_class_profile(value)
        .or_else(|| metadata_class_profile_by_directory(value))
        .filter(|profile| profile.role == MetadataClassRole::TopLevelObject)
        .map(|profile| profile.kind)
}

pub(crate) fn writer_object_kinds() -> Vec<NodeKind> {
    top_level_descriptor_profiles()
        .map(|profile| profile.kind)
        .collect()
}

pub(crate) fn writer_native_class(kind: NodeKind) -> Option<&'static str> {
    registry()
        .objects
        .iter()
        .find(|entry| {
            entry.source == MappingSource::Native
                && entry.kind == kind
                && entry.role == MetadataClassRole::TopLevelObject
        })
        .map(|entry| entry.class_name.as_str())
}

pub(crate) fn legacy_top_level_metadata_classes() -> &'static [&'static str] {
    static CLASSES: OnceLock<Vec<&'static str>> = OnceLock::new();
    CLASSES
        .get_or_init(|| {
            metadata_class_profiles()
                .iter()
                .filter(|entry| {
                    entry.source == MappingSource::Native
                        && entry.role == MetadataClassRole::TopLevelObject
                })
                .map(|entry| entry.class_name.as_str())
                .collect()
        })
        .as_slice()
}

pub(crate) fn metadata_class_profile(class_name: &str) -> Option<&'static MetadataClassProfile> {
    registry()
        .objects
        .iter()
        .find(|entry| entry.source == MappingSource::Native && entry.class_name == class_name)
}

pub(crate) fn unknown_metadata_class_profile() -> &'static MetadataClassProfile {
    registry()
        .objects
        .iter()
        .find(|entry| entry.source == MappingSource::Unknown)
        .expect("unknown mapping")
}

pub(crate) fn derived_profile(kind: NodeKind) -> &'static MetadataClassProfile {
    registry()
        .objects
        .iter()
        .find(|entry| entry.source == MappingSource::Derived && entry.kind == kind)
        .expect("derived mapping")
}

pub(crate) fn property_mapping(
    kind: NodeKind,
    native_name: &str,
) -> Option<&'static PropertyMapping> {
    registry()
        .properties
        .iter()
        .find(|entry| {
            !entry.all_object_kinds
                && entry.object_kinds.contains(&kind)
                && entry.native_names.iter().any(|name| name == native_name)
        })
        .or_else(|| {
            registry().properties.iter().find(|entry| {
                entry.all_object_kinds && entry.native_names.iter().any(|name| name == native_name)
            })
        })
}

pub(crate) fn relation_property_role(kind: NodeKind, native_name: &str) -> Option<RelationRole> {
    registry()
        .relation_properties
        .iter()
        .find(|entry| {
            entry.object_kinds.contains(&kind)
                && entry.native_names.iter().any(|name| name == native_name)
        })
        .map(|entry| entry.role)
}

pub(crate) fn object_kind(profile: &MetadataClassProfile) -> NodeKind {
    profile.kind
}

fn child_mapping_matches(
    entry: &ChildMapping,
    owner: &MetadataClassProfile,
    child: &MetadataClassProfile,
) -> bool {
    (entry.owner_kinds.is_empty() || entry.owner_kinds.contains(&owner.kind))
        && (entry.owner_roles.is_empty() || entry.owner_roles.contains(&owner.role))
        && (entry.child_kinds.is_empty() || entry.child_kinds.contains(&child.kind))
        && (entry.child_roles.is_empty() || entry.child_roles.contains(&child.role))
}

fn child_mapping(
    owner: &MetadataClassProfile,
    child: &MetadataClassProfile,
) -> Option<&'static ChildMapping> {
    registry()
        .children
        .iter()
        .find(|entry| child_mapping_matches(entry, owner, child))
}

pub(crate) fn child_metadata_class_profile(
    owner: &MetadataClassProfile,
    class_name: &str,
) -> Option<&'static MetadataClassProfile> {
    let child = metadata_class_profile(class_name).unwrap_or_else(unknown_metadata_class_profile);
    child_mapping(owner, child).map(|_| child)
}

pub(crate) fn child_relation_role(
    owner: &MetadataClassProfile,
    child: &MetadataClassProfile,
) -> Option<RelationRole> {
    child_mapping(owner, child).map(|entry| entry.relation)
}

pub(crate) fn child_mapping_is_partial(
    owner: &MetadataClassProfile,
    child: &MetadataClassProfile,
) -> bool {
    child_mapping(owner, child).is_some_and(|entry| entry.partial)
}

pub(crate) fn reference_kind(native_class: &str) -> Option<NodeKind> {
    metadata_class_profile(native_class)
        .map(|entry| entry.kind)
        .or_else(|| {
            type_alias(NativeTypeNamespace::CurrentConfiguration, native_class).and_then(|entry| {
                match entry.category {
                    TypeAliasCategory::Reference(kind)
                    | TypeAliasCategory::Object(kind)
                    | TypeAliasCategory::RecordSet(kind)
                    | TypeAliasCategory::Manager(kind)
                    | TypeAliasCategory::Key(kind) => Some(kind),
                    TypeAliasCategory::Enumeration => Some(NodeKind::Enumeration),
                    TypeAliasCategory::DefinedType => Some(NodeKind::DefinedType),
                    TypeAliasCategory::Primitive(_) => None,
                }
            })
        })
}

#[cfg(test)]
mod registry_authority_tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn checked_manifest_and_mutations_are_validated_only_inside_the_adapter() {
        let raw = include_str!("coverage.json");
        validate_coverage_manifest(raw).expect("checked-in registry must self-validate");

        let mutations: [fn(&mut Value); 3] = [
            |manifest: &mut Value| {
                manifest["objects"].as_array_mut().unwrap().remove(0);
            },
            |manifest: &mut Value| {
                manifest["properties"].as_array_mut().unwrap().remove(0);
            },
            |manifest: &mut Value| {
                manifest["objects"][1]["nativeDirectory"] = Value::String(String::new());
            },
        ];
        for mutate in mutations {
            let mut manifest: Value = serde_json::from_str(raw).unwrap();
            mutate(&mut manifest);
            assert!(
                validate_coverage_manifest(&serde_json::to_string(&manifest).unwrap()).is_err()
            );
        }
    }
}

pub(crate) fn enum_value(
    kind: NodeKind,
    property_id: SemanticPropertyId,
    native: &str,
) -> Option<SemanticEnumValue> {
    registry()
        .enum_aliases
        .iter()
        .find(|entry| {
            entry.property_ids.contains(&property_id)
                && entry.object_kinds.contains(&kind)
                && entry.native_aliases.iter().any(|alias| alias == native)
        })
        .map(|entry| entry.semantic)
}

pub(crate) fn derived_enum_value(case: DerivedEnumCase) -> Option<SemanticEnumValue> {
    registry()
        .derived_enum_values
        .iter()
        .find(|entry| entry.case == case)
        .map(|entry| entry.semantic)
}

pub(crate) fn type_alias(
    namespace: NativeTypeNamespace,
    alias: &str,
) -> Option<&'static TypeAliasMapping> {
    registry()
        .type_variants
        .iter()
        .find(|entry| entry.namespace == namespace && entry.alias == alias)
}

pub(crate) fn backing_mapping(kind: NodeKind) -> Option<&'static BackingMapping> {
    registry()
        .backing_artifacts
        .iter()
        .find(|entry| entry.object_kinds.contains(&kind))
}

pub(crate) fn intentional_partial_reasons(
    kind: NodeKind,
) -> impl Iterator<Item = IntentionalPartialReason> {
    registry()
        .intentional_partial_cases
        .iter()
        .filter(move |entry| entry.object_kinds.contains(&kind))
        .map(|entry| entry.reason)
}

pub(crate) fn is_field_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource
    )
}

fn invalid_registry(message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}
