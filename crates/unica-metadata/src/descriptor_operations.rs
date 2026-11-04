//! The addressed descriptor owns its property vocabulary and argument shape.

use crate::applicability::OperationApplicability;
use crate::{metadata_property_schemas, MetadataKind};
use serde_json::{json, Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_operation_contract::OperationSkeleton;
use unica_operation_contract::{object, OperationContract};

macro_rules! properties_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum PropertiesOperation { $($variant),+ }

        impl PropertiesOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<PropertiesOperation> = unica_operations::TypedOperationBundle::new(PropertiesOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            pub const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$applicability),+ }
            }

            pub const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }
        }
    };
}

properties_operations!(
    Set => ("props.set", MetadataOrRoleOrSubsystem, Values),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertiesSite {
    Metadata(MetadataKind),
    CommentOnly(NodeKind),
    FunctionalOption,
    Role,
    Subsystem,
    StyleItem,
    /// The root of a source set: the configuration's own `Configuration.xml`,
    /// or an extension's, which may change only its identity properties.
    Configuration {
        extension: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorPropertyKind {
    String,
    Boolean,
    /// A closed platform enumeration; its values are the 8.3.27 set of
    /// `crate::configuration::cf_validate_enum_allowed`.
    Enum,
}

macro_rules! descriptor_properties {
    ($($field:ident => ($name:literal, $alias:literal, $kind:ident, $subsystem:literal, $role:literal, $configuration:literal, $extension:literal)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum DescriptorProperty {
            $($field),+
        }

        impl DescriptorProperty {
            const ALL: &'static [Self] = &[$(Self::$field),+];

            pub const fn names(self) -> &'static [&'static str] {
                match self { $(Self::$field => &[$name, $alias]),+ }
            }

            pub const fn kind(self) -> DescriptorPropertyKind {
                match self { $(Self::$field => DescriptorPropertyKind::$kind),+ }
            }

            fn applies(self, site: PropertiesSite) -> bool {
                match site {
                    PropertiesSite::Subsystem => match self { $(Self::$field => $subsystem),+ },
                    PropertiesSite::Role => match self { $(Self::$field => $role),+ },
                    PropertiesSite::Configuration { extension: false } => {
                        match self { $(Self::$field => $configuration),+ }
                    }
                    PropertiesSite::Configuration { extension: true } => {
                        match self { $(Self::$field => $extension),+ }
                    }
                    PropertiesSite::Metadata(_) => false,
                    PropertiesSite::CommentOnly(_) => self == Self::Comment,
                    PropertiesSite::FunctionalOption => matches!(self, Self::Location | Self::PrivilegedGetMode | Self::Comment),
                    PropertiesSite::StyleItem => self == Self::Comment,
                }
            }
        }
    };
}

// Columns: subsystem, role, configuration root, extension root. The root
// profile names scalar properties with a published 8.3.27 value set; names,
// languages, roles, storages, forms and the script variant stay outside it.
// An extension root changes only its own identity (#1644). The platform
// re-export of 8.3.27 resets a configuration's own
// `ConfigurationExtensionCompatibilityMode`, so only an extension sets it.
descriptor_properties!(
    Location => ("Location", "location", String, false, false, false, false),
    PrivilegedGetMode => ("PrivilegedGetMode", "privilegedGetMode", Boolean, false, false, false, false),
    Synonym => ("Synonym", "synonym", String, true, true, true, true),
    Comment => ("Comment", "comment", String, true, true, true, true),
    Explanation => ("Explanation", "explanation", String, true, false, false, false),
    IncludeHelpInContents => ("IncludeHelpInContents", "includeHelpInContents", Boolean, true, false, true, false),
    IncludeInCommandInterface => ("IncludeInCommandInterface", "includeInCommandInterface", Boolean, true, false, false, false),
    UseOneCommand => ("UseOneCommand", "useOneCommand", Boolean, true, false, false, false),
    Vendor => ("Vendor", "vendor", String, false, false, true, true),
    Version => ("Version", "version", String, false, false, true, true),
    UpdateCatalogAddress => ("UpdateCatalogAddress", "updateCatalogAddress", String, false, false, true, false),
    BriefInformation => ("BriefInformation", "briefInformation", String, false, false, true, true),
    DetailedInformation => ("DetailedInformation", "detailedInformation", String, false, false, true, true),
    Copyright => ("Copyright", "copyright", String, false, false, true, true),
    VendorInformationAddress => ("VendorInformationAddress", "vendorInformationAddress", String, false, false, true, true),
    ConfigurationInformationAddress => ("ConfigurationInformationAddress", "configurationInformationAddress", String, false, false, true, true),
    UseManagedFormInOrdinaryApplication => ("UseManagedFormInOrdinaryApplication", "useManagedFormInOrdinaryApplication", Boolean, false, false, true, false),
    UseOrdinaryFormInManagedApplication => ("UseOrdinaryFormInManagedApplication", "useOrdinaryFormInManagedApplication", Boolean, false, false, true, false),
    CompatibilityMode => ("CompatibilityMode", "compatibilityMode", Enum, false, false, true, false),
    ConfigurationExtensionCompatibilityMode => ("ConfigurationExtensionCompatibilityMode", "configurationExtensionCompatibilityMode", Enum, false, false, false, true),
    InterfaceCompatibilityMode => ("InterfaceCompatibilityMode", "interfaceCompatibilityMode", Enum, false, false, true, false),
    DefaultRunMode => ("DefaultRunMode", "defaultRunMode", Enum, false, false, true, false),
    DataLockControlMode => ("DataLockControlMode", "dataLockControlMode", Enum, false, false, true, false),
    ObjectAutonumerationMode => ("ObjectAutonumerationMode", "objectAutonumerationMode", Enum, false, false, true, false),
    ModalityUseMode => ("ModalityUseMode", "modalityUseMode", Enum, false, false, true, false),
    SynchronousPlatformExtensionAndAddInCallUseMode => ("SynchronousPlatformExtensionAndAddInCallUseMode", "synchronousPlatformExtensionAndAddInCallUseMode", Enum, false, false, true, false),
    MainClientApplicationWindowMode => ("MainClientApplicationWindowMode", "mainClientApplicationWindowMode", Enum, false, false, true, false),
);

impl DescriptorProperty {
    /// Localized properties keep one value per language; the others are
    /// plain element text.
    pub const fn localized(self) -> bool {
        matches!(
            self,
            Self::Synonym
                | Self::Explanation
                | Self::BriefInformation
                | Self::DetailedInformation
                | Self::Copyright
                | Self::VendorInformationAddress
                | Self::ConfigurationInformationAddress
        )
    }

    /// The closed 8.3.27 value set of an enumerated property. A compatibility
    /// mode leaves out `DontUse`: the platform reads it back as another mode.
    pub fn allowed_values(self) -> &'static [&'static str] {
        let values = crate::configuration::cf_validate_enum_allowed(self.names()[0]);
        match (self, values.split_first()) {
            (
                Self::CompatibilityMode | Self::ConfigurationExtensionCompatibilityMode,
                Some((&"DontUse", modes)),
            ) => modes,
            _ => values,
        }
    }
}

impl PropertiesSite {
    /// The profile of an operation site: a source-set root selects the root
    /// profile of its kind, any other address its descriptor owner.
    pub fn from_site(site: &unica_source_contract::context::OperationSite<'_>) -> Option<Self> {
        if site.route() == unica_source_contract::route::RouteClass::Configuration {
            return Self::root_of(site.source().kind());
        }
        Self::from_address(site.at())
    }

    /// The root profile of a source set; external processors and reports
    /// have none.
    pub fn root_of(kind: unica_source_contract::SourceSetKind) -> Option<Self> {
        match kind {
            unica_source_contract::SourceSetKind::Configuration => {
                Some(Self::Configuration { extension: false })
            }
            unica_source_contract::SourceSetKind::Extension => {
                Some(Self::Configuration { extension: true })
            }
            _ => None,
        }
    }

    /// Read routing is deliberately irrelevant here: CommonModule is read as
    /// a module and still owns a metadata descriptor. Members do not acquire
    /// the owner's write profile; a nested subsystem owns a descriptor of its
    /// own and the subsystem profile with it.
    pub fn from_address(at: &QualifiedAddress) -> Option<Self> {
        if let Some(owner) = at.form_metadata_owner() {
            return (owner.segments().len() == 1)
                .then_some(Self::CommentOnly(NodeKind::CommonForm));
        }
        if at.segments().len() > 1
            && at
                .segments()
                .iter()
                .all(|segment| segment.kind() == NodeKind::Subsystem && segment.name().is_some())
        {
            return Some(Self::Subsystem);
        }
        let [owner] = at.segments() else {
            return None;
        };
        owner.name()?;
        match owner.kind() {
            NodeKind::StyleItem => Some(Self::StyleItem),
            NodeKind::FunctionalOption => Some(Self::FunctionalOption),
            NodeKind::Role => Some(Self::Role),
            NodeKind::Subsystem => Some(Self::Subsystem),
            NodeKind::CommonAttribute
            | NodeKind::FunctionalOptionsParameter
            | NodeKind::SessionParameter
            | NodeKind::FilterCriterion
            | NodeKind::SettingsStorage
            | NodeKind::CommonCommand
            | NodeKind::CommandGroup
            | NodeKind::CommonTemplate
            | NodeKind::CommonPicture
            | NodeKind::Language
            | NodeKind::IntegrationService
            | NodeKind::DocumentNumerator
            | NodeKind::WsReference
            | NodeKind::XdtoPackage
            | NodeKind::Sequence
            | NodeKind::Style => Some(Self::CommentOnly(owner.kind())),
            kind => MetadataKind::parse(kind.as_str()).ok().map(Self::Metadata),
        }
    }

    pub fn descriptor_fields(self) -> impl Iterator<Item = DescriptorProperty> {
        DescriptorProperty::ALL
            .iter()
            .copied()
            .filter(move |field| field.applies(self))
    }

    fn values_schema(self) -> Value {
        let mut properties = Map::new();
        let mut groups = Vec::new();
        if self == Self::StyleItem {
            properties.insert(
                "Value".into(),
                crate::style_color::StyleItemColor::argument_schema(),
            );
        }
        match self {
            Self::Metadata(kind) => properties = metadata_property_schemas(Some(kind)),
            Self::StyleItem
            | Self::FunctionalOption
            | Self::Role
            | Self::Subsystem
            | Self::Configuration { .. }
            | Self::CommentOnly(_) => {
                for field in self.descriptor_fields() {
                    let schema = match field.kind() {
                        DescriptorPropertyKind::String => json!({"type":"string"}),
                        DescriptorPropertyKind::Boolean => json!({"type":"boolean"}),
                        DescriptorPropertyKind::Enum => {
                            json!({"type":"string", "enum":field.allowed_values()})
                        }
                    };
                    let schema = if field == DescriptorProperty::Location {
                        let mut schema = schema;
                        schema["description"] = json!("Existing Constant.Name, Kind.Name.Attribute.Name, Kind.Name.Resource.Name or Kind.Name.TabularSection.Name.Attribute.Name in this source set, where the owner supports that collection. Empty storage is invalid.");
                        schema
                    } else {
                        schema
                    };
                    for name in field.names() {
                        properties.insert((*name).to_string(), schema.clone());
                    }
                    groups.push(field.names());
                }
            }
        }
        let mut values = json!({"type":"object", "additionalProperties":false,
            "minProperties":1, "properties":properties});
        if !groups.is_empty() {
            values["not"] = json!({"anyOf":groups.iter()
                .map(|names| json!({"required":names})).collect::<Vec<_>>()});
            // Each property is a separate group. The boundary reads original
            // spellings before conversion; generic alias normalization must
            // neither merge these groups nor erase diagnostic provenance.
            values["x-unica-alias-groups"] = json!(groups);
        }
        values
    }
}

impl PropertiesOperation {
    pub fn contract(self, site: PropertiesSite) -> OperationContract {
        match self {
            Self::Set => OperationContract {
                schema: object(&[("values", site.values_schema())], &["values"]),
                target: match site {
                    PropertiesSite::StyleItem => "The existing named StyleItem with observed Type=Color and a supported explicit Value: `<set>:StyleItem.<name>`.",
                    PropertiesSite::Configuration { extension: false } => "The configuration root `<set>:Configuration`. The address selects the root property profile.",
                    PropertiesSite::Configuration { extension: true } => "The extension root `<set>:Configuration`. An extension changes only its own identity properties.",
                    PropertiesSite::CommentOnly(NodeKind::CommonForm) => "The existing common form descriptor at `<set>:CommonForm.Name.Metadata`.",
                    PropertiesSite::CommentOnly(_) => "The existing named root metadata descriptor at `<set>:Kind.Name`. This profile changes only Comment.",
                    _ => "The existing named top-level metadata object, role or subsystem, or a nested subsystem, at its full address. The address selects the property profile.",
                },
                effect: "Change the supplied descriptor properties, preserving identity, other properties, translations and companion files.",
                notes: match site {
                    PropertiesSite::StyleItem => "Color Value and optional string Comment/comment are accepted: #RRGGBB or an enumerated web/builtin style color. Empty Value, null and arbitrary XML are refused; empty Comment clears the comment, and simultaneous Comment/comment aliases are refused. Type, identity and bytes outside supplied Value/Comment stay. Font/Border, missing/damaged Value and unmodelled colors have no color write capability. The same semantic color is a byte-preserving no-op, including prefix aliases and RGB case. Arguments are checked before descriptor state. Preview writes nothing; execute only its saved executionToken.",
                    _ => "values is a nonempty object. Use JSON booleans for flags. An empty string clears text where the property allows it; Location requires a nonempty storage reference. null is not omission. Use one spelling of each property, never both aliases. Argument errors are checked before descriptor state; valid arguments still require an existing readable descriptor. Preview writes nothing; execute only the saved execution token.",
                },
                example_args: match site {
                    PropertiesSite::FunctionalOption => json!({"values":{"Location":"Constant.UseGoods","PrivilegedGetMode":true}}),
                    PropertiesSite::Configuration { .. } => json!({"values":{"Version":"1.0.0.2"}}),
                    PropertiesSite::StyleItem => json!({"values":{"Value":"#F0F0F0"}}),
                    _ => json!({"values":{"Comment":"Updated comment"}}),
                },
                help: match site {
                    PropertiesSite::Metadata(_) | PropertiesSite::CommentOnly(_) | PropertiesSite::FunctionalOption | PropertiesSite::StyleItem => "skills/meta-edit/SKILL.md",
                    PropertiesSite::Role => "skills/role-edit/SKILL.md",
                    PropertiesSite::Subsystem => "skills/subsystem-edit/SKILL.md",
                    PropertiesSite::Configuration { .. } => "skills/cf-edit/SKILL.md",
                },
                format_validators: &[],
            },
        }
    }
}

impl unica_operations::FamilyOperation for PropertiesOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = crate::style_color::StyleItemColor;
    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        site.and_then(PropertiesSite::from_site) == Some(PropertiesSite::StyleItem)
    }
    fn admits_observation(self, _observation: Option<&Self::Observation>) -> bool {
        // Missing color evidence still permits the independent Comment writer.
        true
    }
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Properties
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
        None
    }
    fn content_resource(self) -> Option<&'static str> {
        None
    }

    fn contract_at(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Option<unica_operation_contract::OperationContract> {
        self.contract_at_with_observation(site, None)
    }

    fn contract_at_with_observation(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
        observation: Option<&Self::Observation>,
    ) -> Option<OperationContract> {
        PropertiesSite::from_site(site).map(|profile| {
            let profile = if profile == PropertiesSite::StyleItem && observation.is_none() {
                PropertiesSite::CommentOnly(NodeKind::StyleItem)
            } else {
                profile
            };
            self.contract(profile)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_item_contract_uses_only_typed_color_evidence() {
        use unica_operations::FamilyOperation;
        use unica_source_contract::context::{OperationSite, SourceSetTraits};
        use unica_source_contract::route::RouteClass;
        use unica_source_contract::{SourceProfile, SourceSetKind};
        let at = QualifiedAddress::parse("main:StyleItem.Accent").unwrap();
        let profile = SourceProfile::platform_xml_8_3_27_format_2_20();
        let source =
            SourceSetTraits::from_profiles(SourceSetKind::Configuration, Some(profile), profile);
        let site = OperationSite::from_route(&at, source, RouteClass::Metadata);
        let operation = PropertiesOperation::Set.descriptor();
        let plain = operation.argument_contract_at(&site).unwrap();
        let absent = operation
            .argument_contract_at_with_observations(&site, &|_, _| None)
            .unwrap();
        assert_eq!(plain.schema, absent.schema);
        assert!(plain.schema["properties"]["values"]["properties"]
            .get("Value")
            .is_none());
        let color = crate::style_color::StyleItemColor::parse_argument("#123456").unwrap();
        let observed = operation
            .argument_contract_at_with_observations(&site, &|_, requested| {
                assert!(requested.is::<crate::style_color::StyleItemColor>());
                Some(Box::new(color))
            })
            .unwrap();
        assert_eq!(
            observed.schema["properties"]["values"]["properties"]
                .as_object()
                .unwrap()
                .len(),
            3
        );
        assert!(operation
            .argument_contract_at_with_observations(&site, &|_, _| Some(Box::new(String::new())))
            .is_none());
        let oracle = jsonschema::validator_for(&observed.schema).unwrap();
        for values in [
            json!({"Value":"#123456", "Comment":""}),
            json!({"comment":"text"}),
        ] {
            assert!(oracle.is_valid(&json!({"values":values})));
            assert!(crate::property_input::decode_properties(
                PropertiesOperation::Set,
                PropertiesSite::StyleItem,
                &json!({"values":values}),
                "args"
            )
            .is_ok());
        }
        // Another metadata owner ignores irrelevant Color evidence.
        let at = QualifiedAddress::parse("main:Role.Reader").unwrap();
        let site = OperationSite::from_route(&at, source, RouteClass::Metadata);
        assert_eq!(
            PropertiesOperation::Set.contract_at(&site).unwrap().schema,
            operation
                .argument_contract_at_with_observations(&site, &|_, _| Some(Box::new(color)))
                .unwrap()
                .schema
        );
    }

    #[test]
    fn additional_root_comments_have_a_closed_string_only_profile() {
        for kind in [
            "CommonAttribute",
            "FunctionalOptionsParameter",
            "SessionParameter",
            "FilterCriterion",
            "SettingsStorage",
            "CommonCommand",
            "CommandGroup",
            "CommonForm",
            "CommonTemplate",
            "CommonPicture",
            "Language",
            "IntegrationService",
            "DocumentNumerator",
            "WSReference",
            "XDTOPackage",
            "Sequence",
            "Style",
        ] {
            let at = if kind == "CommonForm" {
                format!("main:{kind}.Probe.Metadata")
            } else {
                format!("main:{kind}.Probe")
            };
            let site = PropertiesSite::from_address(&QualifiedAddress::parse(&at).unwrap())
                .unwrap_or_else(|| panic!("missing comment profile for {at}"));
            let schema = PropertiesOperation::Set.contract(site).schema;
            let properties = schema["properties"]["values"]["properties"]
                .as_object()
                .unwrap();
            assert_eq!(properties.len(), 2, "{at}: {properties:?}");
            for alias in ["Comment", "comment"] {
                assert_eq!(properties[alias], json!({"type":"string"}));
                assert!(
                    crate::property_input::decode_properties(
                        PropertiesOperation::Set,
                        site,
                        &json!({"values":{alias:"Текст & < >\n"}}),
                        "args"
                    )
                    .is_ok(),
                    "{at}"
                );
            }
            for values in [
                json!({"Name":"Renamed"}),
                json!({"Synonym":"Label"}),
                json!({"Comment":null}),
                json!({"Comment":1}),
                json!({"Comment":"A","comment":"A"}),
                json!({"Unknown":"x"}),
                json!({}),
            ] {
                assert!(
                    crate::property_input::decode_properties(
                        PropertiesOperation::Set,
                        site,
                        &json!({"values":values}),
                        "args"
                    )
                    .is_err(),
                    "{at}: {values}"
                );
            }
        }
        for at in [
            "main:ExternalDataSource.Probe",
            "main:Interface.Probe",
            "main:CommonForm.Probe",
            "main:CommonForm.Probe.Metadata.Body",
            "main:CommonAttribute",
            "main:CommonAttribute.Probe.Attribute.Child",
        ] {
            assert!(
                QualifiedAddress::parse(at)
                    .ok()
                    .and_then(|at| PropertiesSite::from_address(&at))
                    .is_none(),
                "{at}"
            );
        }
    }

    #[test]
    fn functional_option_storage_properties_have_an_addressed_contract() {
        let at = QualifiedAddress::parse("main:FunctionalOption.UseGoods").unwrap();
        let site = PropertiesSite::from_address(&at).expect("FunctionalOption property profile");
        let contract = PropertiesOperation::Set.contract(site);
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        assert!(oracle.is_valid(
            &json!({"values":{"Location":"Constant.UseGoods","PrivilegedGetMode":true}})
        ));
        assert_eq!(
            contract.schema["properties"]["values"]["properties"]
                .as_object()
                .unwrap()
                .len(),
            6
        );
        for values in [
            json!({"Comment":"Текст"}),
            json!({"comment":"Текст", "Location":"Constant.UseGoods", "privilegedGetMode":false}),
        ] {
            assert!(oracle.is_valid(&json!({"values":values})), "{values}");
            assert!(crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":values}),
                "args"
            )
            .is_ok());
        }
        for values in [
            json!({"Comment":false}),
            json!({"Unknown":"x"}),
            json!({"Synonym":"x"}),
            json!({"Name":"x"}),
            json!({"Comment":"A", "comment":"B", "Location":"Constant.UseGoods"}),
        ] {
            assert!(crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":values}),
                "args"
            )
            .is_err());
        }
        assert!(!oracle.is_valid(&json!({"values":{"PrivilegedGetMode":"true"}})));
        assert!(!oracle.is_valid(&json!({"values":{"Content":[]}})));
    }

    #[test]
    fn registration_and_context_cover_only_current_descriptor_owners() {
        let operation = PropertiesOperation::Set;
        assert_eq!(PropertiesOperation::ALL, &[operation]);
        assert_eq!(operation.name(), "props.set");
        assert_eq!(
            operation.applicability(),
            OperationApplicability::MetadataOrRoleOrSubsystem
        );
        assert_eq!(operation.skeleton(), OperationSkeleton::Values);
        for kind in MetadataKind::ALL {
            let at = QualifiedAddress::parse(&format!("main:{}.Example", kind.as_str())).unwrap();
            assert_eq!(
                PropertiesSite::from_address(&at),
                Some(PropertiesSite::Metadata(*kind))
            );
        }
        assert_eq!(
            PropertiesSite::root_of(unica_source_contract::SourceSetKind::ExternalReport),
            None
        );
        for property in [
            DescriptorProperty::CompatibilityMode,
            DescriptorProperty::ConfigurationExtensionCompatibilityMode,
        ] {
            assert!(!property.allowed_values().contains(&"DontUse"));
            assert!(property.allowed_values().contains(&"Version8_3_27"));
        }
        for (at, expected) in [
            ("main:Role.Reader", Some(PropertiesSite::Role)),
            ("main:Subsystem.Sales", Some(PropertiesSite::Subsystem)),
            ("main:Configuration", None),
            ("main:Catalog", None),
            (
                "main:CommonForm.Card.Metadata",
                Some(PropertiesSite::CommentOnly(NodeKind::CommonForm)),
            ),
            ("main:Catalog.Items.Attribute.Code", None),
            (
                "main:Subsystem.Sales.Subsystem.Child",
                Some(PropertiesSite::Subsystem),
            ),
            ("main:Subsystem.Sales.Interface", None),
        ] {
            assert_eq!(
                PropertiesSite::from_address(&QualifiedAddress::parse(at).unwrap()),
                expected,
                "{at}"
            );
        }
    }

    #[test]
    fn contextual_schema_preserves_metadata_types_and_owner_specific_enums() {
        for (kind, values, expected) in [
            (
                MetadataKind::Task,
                json!({"DefaultPresentation":"AsNumber"}),
                false,
            ),
            (
                MetadataKind::Task,
                json!({"DefaultPresentation":"AsCode"}),
                false,
            ),
            (
                MetadataKind::Catalog,
                json!({"DefaultPresentation":"AsCode"}),
                true,
            ),
            (
                MetadataKind::Catalog,
                json!({"DefaultPresentation":"AsNumber"}),
                false,
            ),
            (
                MetadataKind::InformationRegister,
                json!({"Periodicity":"Second"}),
                true,
            ),
            (
                MetadataKind::CalculationRegister,
                json!({"Periodicity":"Second"}),
                false,
            ),
            (
                MetadataKind::CalculationRegister,
                json!({"Periodicity":"Day"}),
                true,
            ),
            (
                MetadataKind::Constant,
                json!({"Type":{"variants":[{"kind":"boolean"}]}}),
                true,
            ),
            (
                MetadataKind::DefinedType,
                json!({"Type":{"variants":[{"kind":"boolean"}]}}),
                false,
            ),
            (
                MetadataKind::CommonModule,
                json!({"ClientOrdinaryApplication":false}),
                true,
            ),
            (
                MetadataKind::Document,
                json!({"ClientOrdinaryApplication":false}),
                false,
            ),
        ] {
            let contract = PropertiesOperation::Set.contract(PropertiesSite::Metadata(kind));
            let oracle = jsonschema::validator_for(&contract.schema).unwrap();
            assert_eq!(
                oracle.is_valid(&json!({"values":values})),
                expected,
                "{kind:?}: {values}"
            );
        }
    }

    #[test]
    fn normalization_keeps_independent_alias_groups_and_rejects_collisions() {
        for site in [PropertiesSite::Role, PropertiesSite::Subsystem] {
            let contract = PropertiesOperation::Set.contract(site);
            let args = json!({"values":{"synonym":"S","Comment":"C"}});
            assert_eq!(contract.normalize(&args, "args").unwrap(), args);
            for values in [
                json!({"Synonym":"S","synonym":"S"}),
                json!({"Comment":"C","comment":"different"}),
                json!({}),
            ] {
                assert!(contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err());
            }
        }
    }
}
