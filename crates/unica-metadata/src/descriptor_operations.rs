//! The addressed descriptor owns its property vocabulary and argument shape.

use crate::applicability::OperationApplicability;
use crate::{metadata_property_schemas, MetadataKind};
use serde_json::{json, Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_operation_contract::OperationSkeleton;
use unica_operation_contract::{object, OperationContract};
use unica_source_contract::format_identity::{configuration_root_additions_after, PlatformLine};
use unica_source_contract::SourceSetKind;

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
    Metadata {
        kind: MetadataKind,
        source_kind: SourceSetKind,
    },
    CommentOnly(NodeKind),
    DocumentNumerator,
    FunctionalOption,
    Role,
    WSReference,
    Subsystem,
    Language,
    StyleItem,
    /// The root of a source set: the configuration's own `Configuration.xml`,
    /// or an extension's, which may change only its identity properties.
    /// `line` is the platform line of the set's format; it decides which
    /// root properties and values exist. A set without a known line has only
    /// those every described line has.
    Configuration {
        extension: bool,
        line: Option<PlatformLine>,
    },
}

/// A root property, or a value of one, that a later platform line than the
/// set's adds: `can` leaves it out and an edit is refused with this reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaterLineCapability {
    pub property: &'static str,
    pub value: Option<&'static str>,
    pub since: PlatformLine,
}

impl LaterLineCapability {
    /// The reason `can` and the refusal give, naming the set's format and line.
    pub fn reason(self, source_set: &str, line: Option<PlatformLine>) -> String {
        let subject = match self.value {
            Some(value) => format!("Value `{value}` of `{}`", self.property),
            None => format!("Property `{}`", self.property),
        };
        later_line_reason(&subject, self.since, source_set, line)
    }

    /// The `can` mark: what is missing, from which line and format.
    fn mark(self) -> Value {
        let mut mark = json!({
            "property": self.property,
            "since": self.since.to_string(),
            "format": line_format(self.since),
        });
        if let Some(value) = self.value {
            mark["value"] = json!(value);
        }
        mark
    }
}

/// Whether the profile of platform `line` lacks a configuration root
/// `property`, or its `value`: a later line adds it.
fn line_lacks(line: Option<PlatformLine>, property: &str, value: Option<&str>) -> bool {
    configuration_root_additions_after(line)
        .iter()
        .any(|addition| addition.property == property && addition.value == value)
}

/// Why `subject` is missing in a set of platform `line`: it exists from
/// line `since`. `can` and the refusal of an edit give this one reason.
pub fn later_line_reason(
    subject: &str,
    since: PlatformLine,
    source_set: &str,
    line: Option<PlatformLine>,
) -> String {
    let set = match line {
        Some(line) => format!(
            "source set `{source_set}` is in format {} (platform {line})",
            line_format(line)
        ),
        None => format!("source set `{source_set}` names no platform line"),
    };
    format!(
        "{subject} exists from platform {since} (format {}); {set}.",
        line_format(since)
    )
}

/// The Designer export format platform `line` writes.
pub(crate) fn line_format(line: PlatformLine) -> &'static str {
    unica_source_contract::SourceProfile::platform_xml_of_line(line)
        .and_then(unica_source_contract::SourceProfile::serialization_format)
        .unwrap_or("unknown")
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

            /// Whether a configuration root of some line has the property.
            const fn on_configuration_root(self) -> bool {
                match self { $(Self::$field => $configuration),+ }
            }

            /// Whether a root of the given kind, of some line, has the property.
            const fn on_root(self, extension: bool) -> bool {
                if extension {
                    match self { $(Self::$field => $extension),+ }
                } else {
                    self.on_configuration_root()
                }
            }

            pub const fn kind(self) -> DescriptorPropertyKind {
                match self { $(Self::$field => DescriptorPropertyKind::$kind),+ }
            }

            fn applies(self, site: PropertiesSite) -> bool {
                match site {
                    PropertiesSite::Subsystem => match self { $(Self::$field => $subsystem),+ },
                    PropertiesSite::Role => match self { $(Self::$field => $role),+ },
                    PropertiesSite::Configuration { extension, line } => {
                        self.on_root(extension) && !line_lacks(line, self.names()[0], None)
                    }
                    PropertiesSite::Metadata { .. } => false,
                    PropertiesSite::DocumentNumerator => self == Self::Comment,
                    PropertiesSite::CommentOnly(_) | PropertiesSite::Language => self == Self::Comment,
                    PropertiesSite::FunctionalOption => matches!(self, Self::Location | Self::PrivilegedGetMode | Self::Comment),
                    PropertiesSite::StyleItem => self == Self::Comment,
                    PropertiesSite::WSReference => matches!(self, Self::LocationUrl | Self::Comment),
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
    LocationUrl => ("LocationURL", "locationURL", String, false, false, false, false),
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
    Caption => ("Caption", "caption", String, false, false, true, false),
    ShortCaption => ("ShortCaption", "shortCaption", String, false, false, true, false),
    MainClientApplicationWindowInterfaceVariant => ("MainClientApplicationWindowInterfaceVariant", "mainClientApplicationWindowInterfaceVariant", Enum, false, false, true, false),
    ClientApplicationTheme => ("ClientApplicationTheme", "clientApplicationTheme", Enum, false, false, true, false),
    ClientApplicationWindowsOpenVariant => ("ClientApplicationWindowsOpenVariant", "clientApplicationWindowsOpenVariant", Enum, false, false, true, false),
    Version85InterfaceMigrationMode => ("Version85InterfaceMigrationMode", "version85InterfaceMigrationMode", Enum, false, false, true, false),
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
                | Self::Caption
                | Self::ShortCaption
        )
    }

    /// The closed value set of an enumerated property in the line it first
    /// exists in. A compatibility mode leaves out `DontUse`: the platform
    /// reads it back as another mode.
    pub fn allowed_values(self) -> &'static [&'static str] {
        let values = match self {
            Self::MainClientApplicationWindowInterfaceVariant => {
                &["NavigationLeft", "NavigationTop"][..]
            }
            Self::ClientApplicationTheme => &["Auto", "Light", "Dark"],
            Self::ClientApplicationWindowsOpenVariant => &["OpenDataInTabs", "OpenDataInDialogs"],
            Self::Version85InterfaceMigrationMode => &["Use", "DontUse"],
            _ => crate::configuration::cf_validate_enum_allowed(self.names()[0]),
        };
        match (self, values.split_first()) {
            (
                Self::CompatibilityMode | Self::ConfigurationExtensionCompatibilityMode,
                Some((&"DontUse", modes)),
            ) => modes,
            _ => values,
        }
    }

    /// The values of an enumerated property in a set of platform `line`.
    pub fn allowed_values_in(self, line: Option<PlatformLine>) -> Vec<&'static str> {
        let mut values = self.allowed_values().to_vec();
        let property = self.names()[0];
        values.extend(
            configuration_root_additions_after(None)
                .into_iter()
                .filter(|addition| addition.property == property)
                .filter_map(|addition| addition.value)
                .filter(|value| !line_lacks(line, property, Some(value))),
        );
        values
    }
}

impl PropertiesSite {
    /// The profile of an operation site: a source-set root selects the root
    /// profile of its kind, any other address its descriptor owner.
    pub fn from_site(site: &unica_source_contract::context::OperationSite<'_>) -> Option<Self> {
        if site.route() == unica_source_contract::route::RouteClass::Configuration {
            let source = site.source();
            return Self::root_of(
                source.kind(),
                source.classification_profile().semantic_line(),
            );
        }
        let profile = Self::from_address(site.at(), site.source().kind())?;
        if site.source().kind() != unica_source_contract::SourceSetKind::Configuration {
            // An extension adopts WS references and languages; only their own
            // Comment belongs to the extension.
            match profile {
                Self::WSReference => return Some(Self::CommentOnly(NodeKind::WsReference)),
                Self::Language => return Some(Self::CommentOnly(NodeKind::Language)),
                _ => {}
            }
        }
        Some(profile)
    }

    /// The root profile of a source set of platform `line`; external
    /// processors and reports have none.
    pub fn root_of(
        kind: unica_source_contract::SourceSetKind,
        line: Option<PlatformLine>,
    ) -> Option<Self> {
        match kind {
            unica_source_contract::SourceSetKind::Configuration => Some(Self::Configuration {
                extension: false,
                line,
            }),
            unica_source_contract::SourceSetKind::Extension => Some(Self::Configuration {
                extension: true,
                line,
            }),
            _ => None,
        }
    }

    /// The properties and values of a configuration root that the line of
    /// its set does not have, each with the line it exists from
    /// (INV.SOURCE.PROFILE-CAPABILITIES).
    pub fn later_line_capabilities(self) -> Vec<LaterLineCapability> {
        let Self::Configuration { extension, line } = self else {
            return Vec::new();
        };
        configuration_root_additions_after(line)
            .into_iter()
            .filter(|addition| {
                DescriptorProperty::ALL
                    .iter()
                    .any(|field| field.on_root(extension) && field.names()[0] == addition.property)
            })
            .map(|addition| LaterLineCapability {
                property: addition.property,
                value: addition.value,
                since: addition.since,
            })
            .collect()
    }

    /// Read routing is deliberately irrelevant here: CommonModule is read as
    /// a module and still owns a metadata descriptor. Members do not acquire
    /// the owner's write profile; a nested subsystem owns a descriptor of its
    /// own and the subsystem profile with it.
    pub fn from_address(at: &QualifiedAddress, source_kind: SourceSetKind) -> Option<Self> {
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
            NodeKind::DocumentNumerator => Some(Self::DocumentNumerator),
            NodeKind::StyleItem => Some(Self::StyleItem),
            NodeKind::FunctionalOption => Some(Self::FunctionalOption),
            NodeKind::Role => Some(Self::Role),
            NodeKind::Language => Some(Self::Language),
            NodeKind::WsReference => Some(Self::WSReference),
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
            | NodeKind::IntegrationService
            | NodeKind::XdtoPackage
            | NodeKind::Sequence
            | NodeKind::Style => Some(Self::CommentOnly(owner.kind())),
            kind => MetadataKind::parse(kind.as_str())
                .ok()
                .map(|kind| Self::Metadata { kind, source_kind }),
        }
    }

    pub fn descriptor_fields(self) -> impl Iterator<Item = DescriptorProperty> {
        DescriptorProperty::ALL
            .iter()
            .copied()
            .filter(move |field| field.applies(self))
    }

    fn values_schema(self) -> Value {
        self.values_schema_with_style_capabilities(true, true)
    }

    fn values_schema_with_style_capabilities(self, comment: bool, color: bool) -> Value {
        let mut properties = Map::new();
        let mut groups = Vec::new();
        if self == Self::StyleItem && color {
            properties.insert(
                "Value".into(),
                crate::style_color::StyleItemColor::argument_schema(),
            );
        }
        match self {
            Self::DocumentNumerator => {
                // The numerator's closed scalar profile plus the descriptor
                // Comment that the Designer property palette also edits.
                let mut values = crate::document_numerator::values_schema();
                let names = DescriptorProperty::Comment.names();
                for name in names {
                    values["properties"][*name] = json!({"type":"string"});
                }
                values["not"] = json!({"anyOf":[{"required":names}]});
                values["x-unica-alias-groups"] = json!([names]);
                return values;
            }
            Self::Metadata { kind, source_kind } => {
                properties = metadata_property_schemas(Some(kind));
                // EditType is written only in a configuration's own objects.
                if source_kind != SourceSetKind::Configuration {
                    properties.remove("EditType");
                }
            }
            Self::StyleItem
            | Self::WSReference
            | Self::FunctionalOption
            | Self::Language
            | Self::Role
            | Self::Subsystem
            | Self::Configuration { .. }
            | Self::CommentOnly(_) => {
                for field in self.descriptor_fields() {
                    if self == Self::StyleItem && field == DescriptorProperty::Comment && !comment {
                        continue;
                    }
                    let schema = match field.kind() {
                        DescriptorPropertyKind::String => json!({"type":"string"}),
                        DescriptorPropertyKind::Boolean => json!({"type":"boolean"}),
                        DescriptorPropertyKind::Enum => {
                            json!({"type":"string", "enum":self.enum_values(field)})
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
        if self == Self::Language {
            properties.insert(
                "LanguageCode".to_string(),
                json!({
                    "type":"string", "minLength":1,
                    "pattern":crate::language_code::LANGUAGE_CODE_PATTERN
                }),
            );
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
        let later = self.later_line_capabilities();
        if let (Self::Configuration { line, .. }, false) = (self, later.is_empty()) {
            // `can` names what a later line adds, with the reason the edit
            // is refused for (INV.SOURCE.PROFILE-CAPABILITIES).
            values["x-unica-later-lines"] = json!({
                "line": line.map(|line| line.to_string()),
                "format": line.map(line_format),
                "items": later.iter().map(|item| item.mark()).collect::<Vec<_>>(),
            });
        }
        values
    }

    /// The values of an enumerated `field` this site offers.
    fn enum_values(self, field: DescriptorProperty) -> Vec<&'static str> {
        match self {
            Self::Configuration { line, .. } => field.allowed_values_in(line),
            _ => field.allowed_values().to_vec(),
        }
    }

    /// The refusal of a configuration root edit that names a property or
    /// value the line of `source_set` does not have; it is the reason `can`
    /// marks the same property or value with.
    pub fn later_line_refusal(
        self,
        source_set: &str,
        values: &Map<String, Value>,
    ) -> Option<(String, String)> {
        let Self::Configuration { extension, line } = self else {
            return None;
        };
        let later = self.later_line_capabilities();
        for (name, value) in values {
            let Some(field) = DescriptorProperty::ALL
                .iter()
                .copied()
                .find(|field| field.on_root(extension) && field.names().contains(&name.as_str()))
            else {
                continue;
            };
            let canonical = field.names()[0];
            let missing = later.iter().find(|item| {
                item.property == canonical
                    && item.value.is_none_or(|later| value.as_str() == Some(later))
            });
            if let Some(item) = missing {
                return Some((name.clone(), item.reason(source_set, line)));
            }
        }
        None
    }
}

impl PropertiesOperation {
    pub fn contract(self, site: PropertiesSite) -> OperationContract {
        match self {
            Self::Set => OperationContract {
                schema: object(&[("values", site.values_schema())], &["values"]),
                target: match site {
                    PropertiesSite::Language => "An existing registered Language of an ordinary configuration. Its Comment can change; LanguageCode changes only for an unused additional language.",
                    PropertiesSite::WSReference => "An existing registered WSReference in a configuration source set, platform XML 2.20. Its Comment and existing LocationURL properties can change.",
                    PropertiesSite::StyleItem => "The existing named StyleItem with observed Type=Color and a supported explicit Value: `<set>:StyleItem.<name>`.",
                    PropertiesSite::Configuration { extension: false, .. } => "The configuration root `<set>:Configuration`. The address selects the root property profile.",
                    PropertiesSite::Configuration { extension: true, .. } => "The extension root `<set>:Configuration`. An extension changes only its own identity properties.",
                    PropertiesSite::CommentOnly(NodeKind::CommonForm) => "The existing common form descriptor at `<set>:CommonForm.Name.Metadata`.",
                    PropertiesSite::CommentOnly(_) => "The existing named root metadata descriptor at `<set>:Kind.Name`. This profile changes only Comment.",
                    _ => "The existing named top-level metadata object, role or subsystem, or a nested subsystem, at its full address. The address selects the property profile.",
                },
                effect: "Change the supplied descriptor properties, preserving identity, other properties, translations and companion files.",
                notes: match site {
                    PropertiesSite::Language => "Comment/comment is an exact string; an empty string clears it. LanguageCode is a nonempty string without the platform's reserved characters; custom codes are allowed. A LanguageCode change requires an unused additional language, no code collision, and no texts or embedded help bound to either the old or new code. XML and literal NStr/НСтр dependencies are checked in original and final sources; an unprovable dependency refuses the change. No translation migration is performed. An equivalent value preserves bytes. In an extension a language is adopted: its code is set in the main configuration and only Comment changes. Preview writes nothing; execute only the saved execution token.",
                    PropertiesSite::WSReference => "LocationURL and optional Comment/comment are exact strings; an empty string clears the supplied field. Comment-only edits do not validate LocationURL; mixed edits validate both fields before publication. Missing or damaged fields are refused. Editing preserves identity, registration and the local WSDL; it does not download WSDL/imports or generate a client. Preview writes nothing; publish only its execution token. Changes to the descriptor or configuration registration invalidate the saved plan.",
                    PropertiesSite::StyleItem => "Color Value and optional string Comment/comment are accepted: #RRGGBB or an enumerated web/builtin style color. Empty Value, null and arbitrary XML are refused; empty Comment clears the comment, and simultaneous Comment/comment aliases are refused. Type, identity and bytes outside supplied Value/Comment stay. Font/Border, missing/damaged Value and unmodelled colors have no color write capability. The same semantic color is a byte-preserving no-op, including prefix aliases and RGB case. Arguments are checked before descriptor state. Preview writes nothing; execute only its saved executionToken.",
                    _ => "values is a nonempty object. Use JSON booleans for flags. An empty string clears text where the property allows it; Location requires a nonempty storage reference. null is not omission. Use one spelling of each property, never both aliases. Argument errors are checked before descriptor state; valid arguments still require an existing readable descriptor. Preview writes nothing; execute only the saved execution token.",
                },
                example_args: match site {
                    PropertiesSite::DocumentNumerator => json!({"values":{"NumberLength":12,"CheckUnique":true}}),
                    PropertiesSite::WSReference => json!({"values":{"LocationURL":"https://example.invalid/service?wsdl"}}),
                    PropertiesSite::FunctionalOption => json!({"values":{"Location":"Constant.UseGoods","PrivilegedGetMode":true}}),
                    PropertiesSite::Configuration { .. } => json!({"values":{"Version":"1.0.0.2"}}),
                    PropertiesSite::StyleItem => json!({"values":{"Value":"#F0F0F0"}}),
                    PropertiesSite::Language => json!({"values":{"LanguageCode":"de"}}),
                    _ => json!({"values":{"Comment":"Updated comment"}}),
                },
                help: match site {
                    PropertiesSite::WSReference | PropertiesSite::Metadata { .. } | PropertiesSite::DocumentNumerator | PropertiesSite::CommentOnly(_) | PropertiesSite::FunctionalOption | PropertiesSite::StyleItem | PropertiesSite::Language => "skills/meta-edit/SKILL.md",
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
    type Observation = crate::style_color::StyleItemPropertiesObservation;
    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        site.and_then(PropertiesSite::from_site) == Some(PropertiesSite::StyleItem)
    }
    fn admits_observation(self, observation: Option<&Self::Observation>) -> bool {
        observation.is_some_and(|observed| observed.comment || observed.color.is_some())
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
        PropertiesSite::from_site(site).map(|profile| self.contract(profile))
    }

    fn contract_at_with_observation(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
        observation: Option<&Self::Observation>,
    ) -> Option<OperationContract> {
        let profile = PropertiesSite::from_site(site)?;
        if profile != PropertiesSite::StyleItem {
            return Some(self.contract(profile));
        }
        let observed = observation?;
        if !observed.comment && observed.color.is_none() {
            return None;
        }
        if observed.color.is_none() {
            return Some(self.contract(PropertiesSite::CommentOnly(NodeKind::StyleItem)));
        }
        let mut contract = self.contract(profile);
        contract.schema = object(
            &[(
                "values",
                profile.values_schema_with_style_capabilities(observed.comment, true),
            )],
            &["values"],
        );
        if !observed.comment {
            contract.notes = "Only Value is writable on this descriptor: #RRGGBB or an enumerated web/builtin style color. Comment has no confirmed write capability. Type, identity and bytes outside Value stay. The same semantic color is a byte-preserving no-op. Preview writes nothing; execute only its saved executionToken.";
        }
        Some(contract)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn can_offers_the_root_properties_and_values_of_the_set_line() {
        use unica_source_contract::context::{OperationSite, SourceSetTraits};
        use unica_source_contract::route::RouteClass;
        use unica_source_contract::SourceProfile;
        let at = QualifiedAddress::parse("main:Configuration").unwrap();
        let contract = |line: PlatformLine| {
            let profile = SourceProfile::platform_xml_of_line(line).unwrap();
            let source = SourceSetTraits::from_profiles(
                SourceSetKind::Configuration,
                Some(profile),
                profile,
            );
            let site = OperationSite::from_route(&at, source, RouteClass::Configuration);
            PropertiesOperation::Set
                .descriptor()
                .argument_contract_at(&site)
                .unwrap()
        };

        let old = contract(PlatformLine::new(8, 3, 27));
        let values = &old.schema["properties"]["values"];
        for name in [
            "ClientApplicationTheme",
            "clientApplicationTheme",
            "Caption",
            "Version85InterfaceMigrationMode",
        ] {
            assert!(values["properties"].get(name).is_none(), "{name}");
        }
        assert!(!values["properties"]["InterfaceCompatibilityMode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("Version8_5")));
        let later = &values["x-unica-later-lines"];
        assert_eq!(later["line"], "8.3.27");
        assert_eq!(later["format"], "2.20");
        let items = later["items"].as_array().unwrap();
        assert!(items.contains(&json!({
            "property":"ClientApplicationTheme","since":"8.5.1","format":"2.21"
        })));
        assert!(items.contains(&json!({
            "property":"InterfaceCompatibilityMode","value":"Version8_5",
            "since":"8.5.1","format":"2.21"
        })));
        let oracle = jsonschema::validator_for(&old.schema).unwrap();
        assert!(!oracle.is_valid(&json!({"values":{"ClientApplicationTheme":"Dark"}})));

        for line in [PlatformLine::new(8, 5, 1), PlatformLine::new(8, 5, 4)] {
            let new = contract(line);
            let values = &new.schema["properties"]["values"];
            assert_eq!(
                values["properties"]["ClientApplicationTheme"]["enum"],
                json!(["Auto", "Light", "Dark"])
            );
            assert!(values["properties"]["InterfaceCompatibilityMode"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("Version8_5")));
            // 8.5.1 lacks only the modes of 8.5.4; 8.5.4 lacks nothing.
            let later = values["x-unica-later-lines"]["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| item["value"].as_str().unwrap_or_default())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let expected: &[&str] = if line == PlatformLine::new(8, 5, 1) {
                &["Version8_5_4"]
            } else {
                &[]
            };
            assert_eq!(later, expected, "{line}");
            let oracle = jsonschema::validator_for(&new.schema).unwrap();
            assert!(oracle.is_valid(&json!({"values":{
                "ClientApplicationTheme":"Dark","InterfaceCompatibilityMode":"Version8_5",
                "caption":"Учёт"
            }})));
        }

        // The refusal of an edit gives the reason `can` marks it with.
        let site = PropertiesSite::root_of(
            SourceSetKind::Configuration,
            Some(PlatformLine::new(8, 3, 27)),
        )
        .unwrap();
        let values = json!({"Comment":"x","clientApplicationTheme":"Dark"});
        assert_eq!(
            site.later_line_refusal("main", values.as_object().unwrap()),
            Some((
                "clientApplicationTheme".to_string(),
                "Property `ClientApplicationTheme` exists from platform 8.5.1 (format 2.21); source set `main` is in format 2.20 (platform 8.3.27).".to_string()
            ))
        );
        let values = json!({"InterfaceCompatibilityMode":"Taxi"});
        assert_eq!(
            site.later_line_refusal("main", values.as_object().unwrap()),
            None
        );
        // An extension root has none of the 8.5 interface properties; of the
        // later lines it has only its own compatibility modes.
        let extension =
            PropertiesSite::root_of(SourceSetKind::Extension, Some(PlatformLine::new(8, 3, 27)))
                .unwrap();
        assert_eq!(
            extension
                .later_line_capabilities()
                .iter()
                .map(|item| (item.property, item.value))
                .collect::<Vec<_>>(),
            vec![
                (
                    "ConfigurationExtensionCompatibilityMode",
                    Some("Version8_5_1")
                ),
                (
                    "ConfigurationExtensionCompatibilityMode",
                    Some("Version8_5_4")
                ),
            ]
        );
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn compatibility_modes_of_a_line_exist_from_that_line() {
        for (extension, property) in [
            (false, "CompatibilityMode"),
            (true, "ConfigurationExtensionCompatibilityMode"),
        ] {
            let kind = if extension {
                SourceSetKind::Extension
            } else {
                SourceSetKind::Configuration
            };
            let field = PropertiesSite::root_of(kind, None)
                .unwrap()
                .descriptor_fields()
                .find(|field| field.names()[0] == property)
                .unwrap();
            for (line, offered) in [
                (PlatformLine::new(8, 3, 27), &[][..]),
                (PlatformLine::new(8, 5, 1), &["Version8_5_1"][..]),
                (
                    PlatformLine::new(8, 5, 4),
                    &["Version8_5_1", "Version8_5_4"][..],
                ),
            ] {
                let values = field.allowed_values_in(Some(line));
                for mode in ["Version8_5_1", "Version8_5_4"] {
                    assert_eq!(
                        values.contains(&mode),
                        offered.contains(&mode),
                        "{property} {line} {mode}"
                    );
                }
                assert!(values.contains(&"Version8_3_27"));
                assert!(!values.contains(&"DontUse"));
            }
            let site = PropertiesSite::root_of(kind, Some(PlatformLine::new(8, 5, 1))).unwrap();
            let values = json!({ property: "Version8_5_4" });
            assert_eq!(
                site.later_line_refusal("main", values.as_object().unwrap()),
                Some((
                    property.to_string(),
                    format!("Value `Version8_5_4` of `{property}` exists from platform 8.5.4 (format 2.22); source set `main` is in format 2.21 (platform 8.5.1).")
                ))
            );
        }
    }

    #[test]
    fn edit_type_profile_uses_the_actual_source_kind() {
        for source_kind in [
            SourceSetKind::Configuration,
            SourceSetKind::Extension,
            SourceSetKind::ExternalReport,
        ] {
            let site = PropertiesSite::from_address(
                &QualifiedAddress::parse("main:Catalog.Probe").unwrap(),
                source_kind,
            )
            .unwrap();
            let contract = PropertiesOperation::Set.contract(site);
            let fields = &contract.schema["properties"]["values"]["properties"];
            assert_eq!(
                fields.get("EditType").is_some(),
                source_kind == SourceSetKind::Configuration
            );
            let oracle = jsonschema::validator_for(&contract.schema).unwrap();
            assert!(oracle.is_valid(&json!({"values":{"Comment":"independent"}})));
            assert_eq!(
                oracle.is_valid(&json!({"values":{"EditType":"InList"}})),
                source_kind == SourceSetKind::Configuration
            );
        }
    }

    #[test]
    fn ws_reference_location_url_has_a_closed_existing_descriptor_contract() {
        let address = QualifiedAddress::parse("main:WSReference.Probe").unwrap();
        let site = PropertiesSite::from_address(&address, SourceSetKind::Configuration)
            .expect("an existing WSReference owns its LocationURL property profile");
        let contract = PropertiesOperation::Set.contract(site);
        let oracle = jsonschema::validator_for(&contract.schema).unwrap();
        assert!(oracle
            .is_valid(&json!({"values":{"LocationURL":"https://example.invalid/?a=1&b=тест"}})));
        for values in [
            json!({"LocationURL":false}),
            json!({"LocationURL":null}),
            json!({"Name":"Other"}),
            json!({"LocationURL":"one","locationURL":"two"}),
        ] {
            assert!(!oracle.is_valid(&json!({"values":values})), "{values}");
        }
        for values in [
            json!({"Comment":"only"}),
            json!({"LocationURL":"", "comment":"mixed"}),
        ] {
            assert!(oracle.is_valid(&json!({"values":values})));
            assert!(crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":values}),
                "args"
            )
            .is_ok());
        }
        for values in [
            json!({}),
            json!({"Unknown":"x"}),
            json!({"Comment":null}),
            json!({"Comment":"x","comment":"x"}),
        ] {
            assert!(crate::property_input::decode_properties(
                PropertiesOperation::Set,
                site,
                &json!({"values":values}),
                "args"
            )
            .is_err());
        }
        use unica_source_contract::context::{OperationSite, SourceSetTraits};
        use unica_source_contract::{SourceProfile, SourceSetKind};
        let profile = SourceProfile::platform_xml_8_3_27_format_2_20();
        let source =
            SourceSetTraits::from_profiles(SourceSetKind::Extension, Some(profile), profile);
        let operation_site = OperationSite::from_route(
            &address,
            source,
            unica_source_contract::route::RouteClass::Metadata,
        );
        let extension = PropertiesSite::from_site(&operation_site).unwrap();
        assert_eq!(
            extension,
            PropertiesSite::CommentOnly(NodeKind::WsReference)
        );
        let schema = PropertiesOperation::Set.contract(extension).schema;
        assert_eq!(
            schema["properties"]["values"]["properties"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn style_item_contract_uses_only_typed_color_evidence() {
        use crate::style_color::{StyleItemColor, StyleItemPropertiesObservation};
        use unica_operations::FamilyOperation;
        use unica_source_contract::context::{OperationSite, SourceSetTraits};
        use unica_source_contract::route::RouteClass;
        use unica_source_contract::{SourceProfile, SourceSetKind};
        let at = QualifiedAddress::parse("main:StyleItem.Accent").unwrap();
        let profile = SourceProfile::platform_xml_8_3_27_format_2_20();
        let source =
            SourceSetTraits::from_profiles(SourceSetKind::Configuration, Some(profile), profile);
        let site = OperationSite::from_route(&at, source, RouteClass::Metadata);
        let descriptor = PropertiesOperation::Set.descriptor();
        let plain = descriptor.argument_contract_at(&site).unwrap();
        assert!(plain.schema["properties"]["values"]["properties"]
            .get("Value")
            .is_some());
        assert!(descriptor
            .argument_contract_at_with_observations(&site, &|_, _| None)
            .is_none());
        assert!(!descriptor.admits_node(&site, &|_, _| None));
        assert!(descriptor
            .argument_contract_at_with_observations(&site, &|_, _| Some(Box::new(String::new())))
            .is_none());
        let color = StyleItemColor::parse_argument("#123456").unwrap();
        for observed in [
            StyleItemPropertiesObservation {
                comment: true,
                color: None,
            },
            StyleItemPropertiesObservation {
                comment: false,
                color: Some(color),
            },
            StyleItemPropertiesObservation {
                comment: true,
                color: Some(color),
            },
        ] {
            let lookup = |_: &QualifiedAddress, requested: unica_operations::ObservationType| {
                assert!(requested.is::<StyleItemPropertiesObservation>());
                Some(Box::new(observed) as Box<dyn std::any::Any>)
            };
            assert!(descriptor.admits_node(&site, &lookup));
            let contract = descriptor
                .argument_contract_at_with_observations(&site, &lookup)
                .unwrap();
            let fields = &contract.schema["properties"]["values"]["properties"];
            assert_eq!(fields.get("Comment").is_some(), observed.comment);
            assert_eq!(fields.get("comment").is_some(), observed.comment);
            assert_eq!(fields.get("Value").is_some(), observed.color.is_some());
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
            for values in [
                json!({"Comment":""}),
                json!({"comment":"text"}),
                json!({"Value":"#123456"}),
                json!({"Value":"#123456", "Comment":""}),
            ] {
                let args = json!({"values":values});
                let expected = (!values.as_object().unwrap().keys().any(|key| key == "Value")
                    || observed.color.is_some())
                    && (!values.as_object().unwrap().keys().any(|key| key != "Value")
                        || observed.comment);
                assert_eq!(contract.normalize(&args, "args").is_ok(), expected);
                assert!(crate::property_input::decode_properties(
                    PropertiesOperation::Set,
                    PropertiesSite::StyleItem,
                    &args,
                    "args"
                )
                .is_ok());
            }
        }
        let empty = StyleItemPropertiesObservation {
            comment: false,
            color: None,
        };
        assert!(!descriptor.admits_node(&site, &|_, _| Some(Box::new(empty))));
        assert!(descriptor
            .argument_contract_at_with_observations(&site, &|_, _| Some(Box::new(empty)))
            .is_none());
        let at = QualifiedAddress::parse("main:Role.Reader").unwrap();
        let site = OperationSite::from_route(&at, source, RouteClass::Metadata);
        assert_eq!(
            PropertiesOperation::Set.contract_at(&site).unwrap().schema,
            descriptor
                .argument_contract_at_with_observations(&site, &|_, _| Some(Box::new(empty)))
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
            "XDTOPackage",
            "Sequence",
            "Style",
        ] {
            let at = if kind == "CommonForm" {
                format!("main:{kind}.Probe.Metadata")
            } else {
                format!("main:{kind}.Probe")
            };
            let site = PropertiesSite::from_address(
                &QualifiedAddress::parse(&at).unwrap(),
                SourceSetKind::Configuration,
            )
            .unwrap_or_else(|| panic!("missing comment profile for {at}"));
            // A configuration's own language also changes LanguageCode; an
            // extension's adopted language keeps the Comment-only profile.
            let site = if site == PropertiesSite::Language {
                PropertiesSite::CommentOnly(NodeKind::Language)
            } else {
                site
            };
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
                    .and_then(|at| PropertiesSite::from_address(&at, SourceSetKind::Configuration))
                    .is_none(),
                "{at}"
            );
        }
    }

    #[test]
    fn functional_option_storage_properties_have_an_addressed_contract() {
        let at = QualifiedAddress::parse("main:FunctionalOption.UseGoods").unwrap();
        let site = PropertiesSite::from_address(&at, SourceSetKind::Configuration)
            .expect("FunctionalOption property profile");
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
                PropertiesSite::from_address(&at, SourceSetKind::Configuration),
                Some(PropertiesSite::Metadata {
                    kind: *kind,
                    source_kind: SourceSetKind::Configuration
                })
            );
        }
        assert_eq!(
            PropertiesSite::root_of(unica_source_contract::SourceSetKind::ExternalReport, None),
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
                PropertiesSite::from_address(
                    &QualifiedAddress::parse(at).unwrap(),
                    SourceSetKind::Configuration
                ),
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
            let contract = PropertiesOperation::Set.contract(PropertiesSite::Metadata {
                kind,
                source_kind: SourceSetKind::Configuration,
            });
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
