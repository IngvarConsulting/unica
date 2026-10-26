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
    Role,
    Subsystem,
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
    /// a module and still owns a metadata descriptor. Members and nested
    /// subsystems do not acquire the owner's write profile.
    pub fn from_address(at: &QualifiedAddress) -> Option<Self> {
        let [owner] = at.segments() else {
            return None;
        };
        owner.name()?;
        match owner.kind() {
            NodeKind::Role => Some(Self::Role),
            NodeKind::Subsystem => Some(Self::Subsystem),
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
        match self {
            Self::Metadata(kind) => properties = metadata_property_schemas(Some(kind)),
            Self::Role | Self::Subsystem | Self::Configuration { .. } => {
                for field in self.descriptor_fields() {
                    let schema = match field.kind() {
                        DescriptorPropertyKind::String if field.localized() => {
                            crate::localized_value_schema()
                        }
                        DescriptorPropertyKind::String => json!({"type":"string"}),
                        DescriptorPropertyKind::Boolean => json!({"type":"boolean"}),
                        DescriptorPropertyKind::Enum => {
                            json!({"type":"string", "enum":field.allowed_values()})
                        }
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
                    PropertiesSite::Configuration { extension: false } => "The configuration root `<set>:Configuration`. The address selects the root property profile.",
                    PropertiesSite::Configuration { extension: true } => "The extension root `<set>:Configuration`. An extension changes only its own identity properties.",
                    _ => "The existing named top-level metadata object, role or subsystem at its full address. The address selects the property profile.",
                },
                effect: "Change the supplied descriptor properties, preserving identity, other properties, translations and companion files.",
                notes: "values is a nonempty object. Use JSON booleans for flags. An empty string clears text; null is not omission. Use one spelling of each property, never both aliases. Argument errors are checked before descriptor state; valid arguments still require an existing readable descriptor. Preview writes nothing; execute only the saved execution token.",
                example_args: match site {
                    PropertiesSite::Configuration { .. } => json!({"values":{"Version":"1.0.0.2"}}),
                    _ => json!({"values":{"Comment":"Updated comment"}}),
                },
                help: match site {
                    PropertiesSite::Metadata(_) => "skills/meta-edit/SKILL.md",
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
    type Observation = ();
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
        PropertiesSite::from_site(site).map(|profile| self.contract(profile))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ("main:CommonForm.Card.Metadata", None),
            ("main:Catalog.Items.Attribute.Code", None),
            ("main:Subsystem.Sales.Subsystem.Child", None),
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
            let args = json!({"values":{"synonym":"ru = 'S'","Comment":"C"}});
            assert_eq!(contract.normalize(&args, "args").unwrap(), args);
            for values in [
                json!({"Synonym":"ru = 'S'","synonym":"ru = 'S'"}),
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
