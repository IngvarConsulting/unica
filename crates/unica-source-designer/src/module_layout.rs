use crate::layout::{
    external_owner_parts, metadata_descriptor_relative, metadata_kind, metadata_kind_by_directory,
    MetadataLayout,
};
use std::path::{Component, Path, PathBuf};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_address::module_routes::{
    has_direct_module_route, has_nested_module_route, ModuleRole, ModuleRouteKind,
};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_source_contract::SourceSetKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformXmlModuleIdentity {
    pub owner: String,
    pub address: MetadataAddress,
    pub role: PlatformXmlModuleRole,
    pub descriptors: Vec<PathBuf>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformXmlModuleRole {
    Module,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    FormModule,
    CommandModule,
    ManagedApplicationModule,
    OrdinaryApplicationModule,
    SessionModule,
    ExternalConnectionModule,
}
impl PlatformXmlModuleRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::FormModule => "FormModule",
            Self::CommandModule => "CommandModule",
            Self::ManagedApplicationModule => "ManagedApplicationModule",
            Self::OrdinaryApplicationModule => "OrdinaryApplicationModule",
            Self::SessionModule => "SessionModule",
            Self::ExternalConnectionModule => "ExternalConnectionModule",
        }
    }
}
pub fn platform_xml_module_identity(relative: &Path) -> Result<PlatformXmlModuleIdentity, String> {
    module_layout_for_relative(relative).map(|(_, identity)| identity)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformXmlModuleLayoutFamily {
    RootApplication,
    OwnerModule,
    CommonForm,
    CommonCommand,
    DirectMetadata,
    NestedForm,
    NestedCommand,
}

const OWNER_MODULE_KINDS: &[&str] = &[
    "CommonModule",
    "Bot",
    "HTTPService",
    "WebService",
    "IntegrationService",
];

#[derive(Debug, Clone, Copy)]
enum ModuleLayoutToken {
    Literal(&'static str),
    MetadataKind,
    MetadataDirectory,
    OwnerName,
    ChildName,
    Role(ModuleRoleClass),
    RoleFile(ModuleRoleClass),
}

#[derive(Debug, Clone, Copy)]
enum ModuleRoleClass {
    Root,
    Direct,
}

#[derive(Debug, Clone, Copy)]
enum ModuleLayoutCapability {
    Any,
    OwnerModule,
    DirectModule,
    NestedFormOrCommand,
}

#[derive(Debug, Clone, Copy)]
enum ModuleDescriptorRule {
    Root,
    Common {
        kind: &'static str,
        directory: &'static str,
    },
    Owner,
    Nested {
        /// Collection directory holding the child; the child's kind is derived
        /// from the logical address, so it is not repeated here.
        child_directory: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
struct PlatformXmlModuleLayoutDescriptor {
    family: PlatformXmlModuleLayoutFamily,
    physical: &'static [ModuleLayoutToken],
    logical: &'static [ModuleLayoutToken],
    capability: ModuleLayoutCapability,
    fixed_role: Option<PlatformXmlModuleRole>,
    descriptor_rule: ModuleDescriptorRule,
}

const PLATFORM_XML_MODULE_LAYOUT_FAMILIES: &[PlatformXmlModuleLayoutDescriptor] = &[
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::RootApplication,
        physical: &[
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::RoleFile(ModuleRoleClass::Root),
        ],
        logical: &[ModuleLayoutToken::Role(ModuleRoleClass::Root)],
        capability: ModuleLayoutCapability::Any,
        fixed_role: None,
        descriptor_rule: ModuleDescriptorRule::Root,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::OwnerModule,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Module"),
        ],
        capability: ModuleLayoutCapability::OwnerModule,
        fixed_role: Some(PlatformXmlModuleRole::Module),
        descriptor_rule: ModuleDescriptorRule::Owner,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::CommonForm,
        physical: &[
            ModuleLayoutToken::Literal("CommonForms"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::Literal("CommonForm"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("FormModule"),
        ],
        capability: ModuleLayoutCapability::Any,
        fixed_role: Some(PlatformXmlModuleRole::FormModule),
        descriptor_rule: ModuleDescriptorRule::Common {
            kind: "CommonForm",
            directory: "CommonForms",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::CommonCommand,
        physical: &[
            ModuleLayoutToken::Literal("CommonCommands"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("CommandModule.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::Literal("CommonCommand"),
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("CommandModule"),
        ],
        capability: ModuleLayoutCapability::Any,
        fixed_role: Some(PlatformXmlModuleRole::CommandModule),
        descriptor_rule: ModuleDescriptorRule::Common {
            kind: "CommonCommand",
            directory: "CommonCommands",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::DirectMetadata,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::RoleFile(ModuleRoleClass::Direct),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Role(ModuleRoleClass::Direct),
        ],
        capability: ModuleLayoutCapability::DirectModule,
        fixed_role: None,
        descriptor_rule: ModuleDescriptorRule::Owner,
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::NestedForm,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Forms"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::Literal("Module.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Form"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("FormModule"),
        ],
        capability: ModuleLayoutCapability::NestedFormOrCommand,
        fixed_role: Some(PlatformXmlModuleRole::FormModule),
        descriptor_rule: ModuleDescriptorRule::Nested {
            child_directory: "Forms",
        },
    },
    PlatformXmlModuleLayoutDescriptor {
        family: PlatformXmlModuleLayoutFamily::NestedCommand,
        physical: &[
            ModuleLayoutToken::MetadataDirectory,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Commands"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("Ext"),
            ModuleLayoutToken::Literal("CommandModule.bsl"),
        ],
        logical: &[
            ModuleLayoutToken::MetadataKind,
            ModuleLayoutToken::OwnerName,
            ModuleLayoutToken::Literal("Command"),
            ModuleLayoutToken::ChildName,
            ModuleLayoutToken::Literal("CommandModule"),
        ],
        capability: ModuleLayoutCapability::NestedFormOrCommand,
        fixed_role: Some(PlatformXmlModuleRole::CommandModule),
        descriptor_rule: ModuleDescriptorRule::Nested {
            child_directory: "Commands",
        },
    },
];

impl PlatformXmlModuleLayoutDescriptor {
    fn identity_from_parts(
        &self,
        parts: &[&str],
    ) -> Option<Result<PlatformXmlModuleIdentity, String>> {
        self.capture(self.physical, parts)
            .map(|captures| self.identity(&captures))
    }

    fn path_from_address(&self, parts: &[&str]) -> Option<Result<PathBuf, String>> {
        self.capture(self.logical, parts).map(|captures| {
            let mut path = PathBuf::new();
            for segment in self.render(self.physical, &captures) {
                path.push(segment);
            }
            Ok(path)
        })
    }

    fn capture<'a>(
        &self,
        template: &[ModuleLayoutToken],
        parts: &[&'a str],
    ) -> Option<ModuleLayoutCaptures<'a>> {
        if template.len() != parts.len() {
            return None;
        }
        let mut captures = ModuleLayoutCaptures::default();
        for (token, value) in template.iter().zip(parts.iter().copied()) {
            match token {
                ModuleLayoutToken::Literal(expected) if *expected == value => {}
                ModuleLayoutToken::Literal(_) => return None,
                ModuleLayoutToken::MetadataKind => {
                    captures.kind = Some(metadata_kind(value)?);
                }
                ModuleLayoutToken::MetadataDirectory => {
                    captures.kind = Some(metadata_kind_by_directory(value)?);
                }
                ModuleLayoutToken::OwnerName => captures.owner_name = Some(value),
                ModuleLayoutToken::ChildName => captures.child_name = Some(value),
                ModuleLayoutToken::Role(class) => {
                    captures.role = Some(class.parse(value)?);
                }
                ModuleLayoutToken::RoleFile(class) => {
                    captures.role = Some(class.parse(value.strip_suffix(".bsl")?)?);
                }
            }
        }
        self.accepts(&captures).then_some(captures)
    }

    fn accepts(&self, captures: &ModuleLayoutCaptures<'_>) -> bool {
        match self.capability {
            ModuleLayoutCapability::Any => true,
            ModuleLayoutCapability::OwnerModule => captures
                .kind
                .is_some_and(|kind| OWNER_MODULE_KINDS.contains(&kind.tag)),
            ModuleLayoutCapability::DirectModule => {
                captures
                    .kind
                    .zip(captures.role)
                    .is_some_and(|(kind, role)| {
                        NodeKind::parse(kind.tag)
                            .ok()
                            .zip(ModuleRole::from_v12_terminal(role.as_str()))
                            .is_some_and(|(owner, role)| has_direct_module_route(owner, role))
                    })
            }
            ModuleLayoutCapability::NestedFormOrCommand => captures.kind.is_some_and(|kind| {
                NodeKind::parse(kind.tag)
                    .ok()
                    .is_some_and(has_nested_module_route)
            }),
        }
    }

    fn render(
        &self,
        template: &[ModuleLayoutToken],
        captures: &ModuleLayoutCaptures<'_>,
    ) -> Vec<String> {
        template
            .iter()
            .map(|token| match token {
                ModuleLayoutToken::Literal(value) => (*value).to_string(),
                ModuleLayoutToken::MetadataKind => captures
                    .kind
                    .expect("accepted layout must capture metadata kind")
                    .tag
                    .to_string(),
                ModuleLayoutToken::MetadataDirectory => captures
                    .kind
                    .expect("accepted layout must capture metadata kind")
                    .directory
                    .to_string(),
                ModuleLayoutToken::OwnerName => captures
                    .owner_name
                    .expect("accepted layout must capture owner name")
                    .to_string(),
                ModuleLayoutToken::ChildName => captures
                    .child_name
                    .expect("accepted layout must capture child name")
                    .to_string(),
                ModuleLayoutToken::Role(_) => captures
                    .role
                    .expect("accepted layout must capture module role")
                    .as_str()
                    .to_string(),
                ModuleLayoutToken::RoleFile(_) => format!(
                    "{}.bsl",
                    captures
                        .role
                        .expect("accepted layout must capture module role")
                        .as_str()
                ),
            })
            .collect()
    }

    fn identity(
        &self,
        captures: &ModuleLayoutCaptures<'_>,
    ) -> Result<PlatformXmlModuleIdentity, String> {
        let address = self.render(self.logical, captures).join(".");
        let role = self
            .fixed_role
            .or(captures.role)
            .ok_or_else(unsupported_module_layout)?;
        let (owner, descriptors) = match self.descriptor_rule {
            ModuleDescriptorRule::Root => (
                "Configuration".to_string(),
                vec![PathBuf::from("Configuration.xml")],
            ),
            ModuleDescriptorRule::Common { kind, directory } => {
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{kind}.{name}"),
                    vec![metadata_descriptor(directory, name)],
                )
            }
            ModuleDescriptorRule::Owner => {
                let kind = captures.kind.ok_or_else(unsupported_module_layout)?;
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{}.{name}", kind.tag),
                    vec![metadata_descriptor(kind.directory, name)],
                )
            }
            ModuleDescriptorRule::Nested { child_directory } => {
                let kind = captures.kind.ok_or_else(unsupported_module_layout)?;
                let name = captures.owner_name.ok_or_else(unsupported_module_layout)?;
                let child_name = captures.child_name.ok_or_else(unsupported_module_layout)?;
                (
                    format!("{}.{name}", kind.tag),
                    vec![
                        metadata_descriptor(kind.directory, name),
                        // The child's metadata descriptor sits beside its
                        // directory as `<Forms>/<Name>.xml`. The file under
                        // `<Name>/Ext/Form.xml` is the form's own content in the
                        // logform schema, not a `MetaDataObject` descriptor.
                        PathBuf::from(kind.directory)
                            .join(name)
                            .join(child_directory)
                            .join(format!("{child_name}.xml")),
                    ],
                )
            }
        };
        module_identity(address, owner, role, descriptors)
    }

    #[cfg(test)]
    fn family(&self) -> PlatformXmlModuleLayoutFamily {
        self.family
    }

    #[cfg(test)]
    fn render_physical(&self, address: &MetadataAddress) -> Result<PathBuf, String> {
        let parts = address.segments().collect::<Vec<_>>();
        self.path_from_address(&parts)
            .ok_or_else(unsupported_module_layout)?
    }

    #[cfg(test)]
    fn parse_physical(&self, relative: &Path) -> Result<MetadataAddress, String> {
        let components = relative_module_path_components(relative)?;
        let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
        self.identity_from_parts(&parts)
            .ok_or_else(unsupported_module_layout)?
            .map(|identity| identity.address)
    }
}

#[derive(Debug, Default)]
struct ModuleLayoutCaptures<'a> {
    kind: Option<&'static MetadataLayout>,
    owner_name: Option<&'a str>,
    child_name: Option<&'a str>,
    role: Option<PlatformXmlModuleRole>,
}

impl ModuleRoleClass {
    fn parse(self, raw: &str) -> Option<PlatformXmlModuleRole> {
        let role = match raw {
            "ObjectModule" => PlatformXmlModuleRole::ObjectModule,
            "ManagerModule" => PlatformXmlModuleRole::ManagerModule,
            "RecordSetModule" => PlatformXmlModuleRole::RecordSetModule,
            "ValueManagerModule" => PlatformXmlModuleRole::ValueManagerModule,
            "ManagedApplicationModule" => PlatformXmlModuleRole::ManagedApplicationModule,
            "OrdinaryApplicationModule" => PlatformXmlModuleRole::OrdinaryApplicationModule,
            "SessionModule" => PlatformXmlModuleRole::SessionModule,
            "ExternalConnectionModule" => PlatformXmlModuleRole::ExternalConnectionModule,
            _ => return None,
        };
        match self {
            Self::Root
                if matches!(
                    role,
                    PlatformXmlModuleRole::ManagedApplicationModule
                        | PlatformXmlModuleRole::OrdinaryApplicationModule
                        | PlatformXmlModuleRole::SessionModule
                        | PlatformXmlModuleRole::ExternalConnectionModule
                ) =>
            {
                Some(role)
            }
            Self::Direct
                if matches!(
                    role,
                    PlatformXmlModuleRole::ObjectModule
                        | PlatformXmlModuleRole::ManagerModule
                        | PlatformXmlModuleRole::RecordSetModule
                        | PlatformXmlModuleRole::ValueManagerModule
                ) =>
            {
                Some(role)
            }
            _ => None,
        }
    }
}

fn module_layout_for_relative(
    relative: &Path,
) -> Result<(PlatformXmlModuleLayoutFamily, PlatformXmlModuleIdentity), String> {
    let components = relative_module_path_components(relative)?;
    let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(identity) = family.identity_from_parts(&parts) {
            return identity.map(|identity| (family.family, identity));
        }
    }
    Err(unsupported_module_layout())
}

fn relative_module_path_components(relative: &Path) -> Result<Vec<String>, String> {
    relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "BSL module path is not valid UTF-8".to_string()),
            _ => Err("BSL module path must be relative to its source set".to_string()),
        })
        .collect()
}

#[cfg(test)]
fn module_layout_descriptors_for_test() -> &'static [PlatformXmlModuleLayoutDescriptor] {
    PLATFORM_XML_MODULE_LAYOUT_FAMILIES
}

fn module_identity(
    address: String,
    owner: String,
    role: PlatformXmlModuleRole,
    descriptors: Vec<PathBuf>,
) -> Result<PlatformXmlModuleIdentity, String> {
    let address = MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        &address,
    )
    .map_err(|error| error.to_string())?;
    Ok(PlatformXmlModuleIdentity {
        owner,
        address,
        role,
        descriptors,
    })
}

/// Pure configuration Platform XML module layout shared by logical readers and
/// the future staged event writer. External EPF/ERF roots remain a separate
/// source-set layout and are resolved before this mapper is called.
pub fn platform_xml_module_relative(address: &MetadataAddress) -> Result<PathBuf, String> {
    let parts = address.segments().collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(path) = family.path_from_address(&parts) {
            return path;
        }
    }
    Err(unsupported_module_layout())
}

pub fn metadata_descriptor(directory: &str, name: &str) -> PathBuf {
    PathBuf::from(directory).join(format!("{name}.xml"))
}
fn unsupported_module_layout() -> String {
    "unica.code.patch v1 accepts only a supported canonical platform XML BSL module path"
        .to_string()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_xml_module_layouts_share_one_family_registry_in_both_directions() {
        let cases = [
            (
                super::PlatformXmlModuleLayoutFamily::RootApplication,
                "ManagedApplicationModule",
                "Ext/ManagedApplicationModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::OwnerModule,
                "CommonModule.Shared.Module",
                "CommonModules/Shared/Ext/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::CommonForm,
                "CommonForm.Main.FormModule",
                "CommonForms/Main/Ext/Form/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::CommonCommand,
                "CommonCommand.Print.CommandModule",
                "CommonCommands/Print/Ext/CommandModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::DirectMetadata,
                "Catalog.Items.ManagerModule",
                "Catalogs/Items/Ext/ManagerModule.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::NestedForm,
                "Catalog.Items.Form.List.FormModule",
                "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            ),
            (
                super::PlatformXmlModuleLayoutFamily::NestedCommand,
                "Catalog.Items.Command.Open.CommandModule",
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            ),
        ];

        let descriptors = super::module_layout_descriptors_for_test();
        assert_eq!(descriptors.len(), cases.len());
        for ((expected_family, address, relative), descriptor) in cases.into_iter().zip(descriptors)
        {
            let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, address).unwrap();

            assert_eq!(descriptor.family(), expected_family);
            assert_eq!(
                descriptor.render_physical(&address).unwrap(),
                Path::new(relative)
            );
            assert_eq!(
                descriptor.parse_physical(Path::new(relative)).unwrap(),
                address
            );
        }
    }
}

pub fn form_module_target(form: &MetadataAddress) -> Result<MetadataAddress, String> {
    MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{}.FormModule", form.as_str()),
    )
    .map_err(|error| error.to_string())
}
pub fn form_descriptor_requirements(
    form: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<Vec<PathBuf>, String> {
    let parts = form.as_str().split('.').collect::<Vec<_>>();
    let [owner_kind, owner_name, "Form", form_name] = parts.as_slice() else {
        if matches!(parts.as_slice(), ["CommonForm", _]) {
            return metadata_descriptor_relative(form, source_set_kind).map(|path| vec![path]);
        }
        return Err(String::from(
            "form target does not retain a typed metadata owner chain",
        ));
    };
    let owner = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{owner_name}"),
    )
    .map_err(|error| error.to_string())?;
    let nested = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{owner_name}.Form.{form_name}"),
    )
    .map_err(|error| error.to_string())?;
    Ok(vec![
        metadata_descriptor_relative(&owner, source_set_kind)?,
        metadata_descriptor_relative(&nested, source_set_kind)?,
    ])
}
pub fn module_descriptor_requirements(
    target: &MetadataAddress,
    route_kind: ModuleRouteKind,
    source_set_kind: SourceSetKind,
) -> Result<Vec<PathBuf>, String> {
    if route_kind == ModuleRouteKind::Root {
        return Ok(vec![PathBuf::from("Configuration.xml")]);
    }
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let descriptor_raw = match route_kind {
        ModuleRouteKind::Direct
        | ModuleRouteKind::Common
        | ModuleRouteKind::CommonForm
        | ModuleRouteKind::CommonCommand
        | ModuleRouteKind::Service
        | ModuleRouteKind::Bot => parts[..2].join("."),
        ModuleRouteKind::NestedForm | ModuleRouteKind::NestedCommand => parts[..4].join("."),
        ModuleRouteKind::Root | ModuleRouteKind::WebSocketClient => {
            return Err(String::from(
                "module descriptor requirements are unavailable for this layout",
            ))
        }
    };
    let descriptor = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &descriptor_raw)
        .map_err(|error| error.to_string())?;
    let nested = metadata_descriptor_relative(&descriptor, source_set_kind)?;
    if matches!(
        route_kind,
        ModuleRouteKind::NestedForm | ModuleRouteKind::NestedCommand
    ) && !matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &parts[..2].join("."))
            .map_err(|error| error.to_string())?;
        return Ok(vec![
            metadata_descriptor_relative(&owner, source_set_kind)?,
            nested,
        ]);
    }
    Ok(vec![nested])
}
pub fn module_source_address(
    module_at: &QualifiedAddress,
    owner_kind: NodeKind,
    role: ModuleRole,
    route_kind: ModuleRouteKind,
) -> Result<MetadataAddress, String> {
    let segments = module_at.segments();
    let logical = match route_kind {
        ModuleRouteKind::Root => match role {
            ModuleRole::ManagedApplication => "ManagedApplicationModule".to_string(),
            ModuleRole::OrdinaryApplication => "OrdinaryApplicationModule".to_string(),
            ModuleRole::Session => "SessionModule".to_string(),
            ModuleRole::ExternalConnection => "ExternalConnectionModule".to_string(),
            _ => return Err(unsupported_logical_module_layout(owner_kind, role)),
        },
        ModuleRouteKind::Common => format!(
            "CommonModule.{}.Module",
            required_segment_name(segments.first(), owner_kind, role)?
        ),
        ModuleRouteKind::Direct => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_logical_module_layout(owner_kind, role))?;
            let rendered_role = match role {
                ModuleRole::Object => "ObjectModule",
                ModuleRole::Manager => "ManagerModule",
                ModuleRole::RecordSet => "RecordSetModule",
                ModuleRole::ValueManager => "ValueManagerModule",
                _ => return Err(unsupported_logical_module_layout(owner_kind, role)),
            };
            format!(
                "{}.{}.{rendered_role}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), owner_kind, role)?
            )
        }
        ModuleRouteKind::CommonForm => format!(
            "CommonForm.{}.FormModule",
            required_segment_name(segments.first(), owner_kind, role)?
        ),
        ModuleRouteKind::CommonCommand => format!(
            "CommonCommand.{}.CommandModule",
            required_segment_name(segments.first(), owner_kind, role)?
        ),
        ModuleRouteKind::NestedForm | ModuleRouteKind::NestedCommand => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_logical_module_layout(owner_kind, role))?;
            let child = segments
                .get(1)
                .ok_or_else(|| unsupported_logical_module_layout(owner_kind, role))?;
            let terminal = if route_kind == ModuleRouteKind::NestedForm {
                "FormModule"
            } else {
                "CommandModule"
            };
            format!(
                "{}.{}.{}.{}.{terminal}",
                owner.kind().as_str(),
                required_segment_name(Some(owner), owner_kind, role)?,
                child.kind().as_str(),
                required_segment_name(Some(child), owner_kind, role)?,
            )
        }
        ModuleRouteKind::Service | ModuleRouteKind::Bot => {
            let owner = segments
                .first()
                .ok_or_else(|| unsupported_logical_module_layout(owner_kind, role))?;
            format!(
                "{}.{}.Module",
                owner.kind().as_str(),
                required_segment_name(Some(owner), owner_kind, role)?
            )
        }
        ModuleRouteKind::WebSocketClient => {
            return Err(
                "WebSocketClient source layout is not specified for platform profile 8.3.27"
                    .to_string(),
            )
        }
    };
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &logical)
        .map_err(|error| error.to_string())
}
fn required_segment_name(
    segment: Option<&AddressSegment>,
    owner_kind: NodeKind,
    role: ModuleRole,
) -> Result<&str, String> {
    segment
        .and_then(AddressSegment::name)
        .ok_or_else(|| unsupported_logical_module_layout(owner_kind, role))
}
fn unsupported_logical_module_layout(owner_kind: NodeKind, role: ModuleRole) -> String {
    format!(
        "module source layout is unavailable for {}.{}",
        owner_kind.as_str(),
        role.as_str()
    )
}
pub fn module_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    if matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        external_module_relative(target, source_set_kind)
    } else {
        platform_xml_module_relative(target)
    }
}
/// `Catalog.X.ObjectModule` → `Catalog.X.Module.Object`; the common module is
/// its own module node and a service module repeats the owner kind as role.
pub fn logical_module_from_metadata(metadata: &str) -> Option<String> {
    fn role(terminal: &str) -> Option<&str> {
        terminal
            .strip_suffix("Module")
            .filter(|role| !role.is_empty())
    }
    match metadata.split('.').collect::<Vec<_>>().as_slice() {
        [root] => Some(format!("Module.{}", role(root)?)),
        ["CommonModule", name, "Module"] => Some(format!("CommonModule.{name}")),
        [kind, name, "Module"] => Some(format!("{kind}.{name}.Module.{kind}")),
        [kind, name, terminal] => Some(format!("{kind}.{name}.Module.{}", role(terminal)?)),
        [kind, name, child_kind, child, terminal] => Some(format!(
            "{kind}.{name}.{child_kind}.{child}.Module.{}",
            role(terminal)?
        )),
        _ => None,
    }
}
pub fn external_module_metadata_address(
    relative: &Path,
    source_set_kind: SourceSetKind,
) -> Option<String> {
    let kind = match source_set_kind {
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
        SourceSetKind::Configuration | SourceSetKind::Extension => return None,
    };
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [owner, "Ext", "ObjectModule.bsl"] => Some(format!("{kind}.{owner}.ObjectModule")),
        [owner, "Forms", form, "Ext", "Form", "Module.bsl"] => {
            Some(format!("{kind}.{owner}.Form.{form}.FormModule"))
        }
        [owner, "Commands", command, "Ext", "CommandModule.bsl"] => {
            Some(format!("{kind}.{owner}.Command.{command}.CommandModule"))
        }
        _ => None,
    }
}
fn external_module_relative(
    target: &MetadataAddress,
    source_set_kind: SourceSetKind,
) -> Result<PathBuf, String> {
    let (_, owner_name, rest) = external_owner_parts(target, source_set_kind)?;
    match rest.as_slice() {
        [terminal] if *terminal == "ObjectModule" => Ok(PathBuf::from(owner_name)
            .join("Ext")
            .join("ObjectModule.bsl")),
        [nested_kind, nested_name, terminal] => {
            let (directory, file) = match (*nested_kind, *terminal) {
                ("Form", "FormModule") => ("Forms", PathBuf::from("Form/Module.bsl")),
                ("Command", "CommandModule") => ("Commands", PathBuf::from("CommandModule.bsl")),
                _ => return Err("external nested module target has unsupported layout".into()),
            };
            Ok(PathBuf::from(owner_name)
                .join(directory)
                .join(nested_name)
                .join("Ext")
                .join(file))
        }
        _ => Err("external module target has unsupported layout".into()),
    }
}

#[cfg(test)]
mod code_layout_tests {
    use super::platform_xml_module_identity as module_identity;
    use std::path::Path;
    #[test]
    fn canonical_module_layouts_have_stable_owner_and_role() {
        let direct_cases = [
            (
                "Ext/ManagedApplicationModule.bsl",
                "Configuration",
                "ManagedApplicationModule",
            ),
            (
                "Ext/OrdinaryApplicationModule.bsl",
                "Configuration",
                "OrdinaryApplicationModule",
            ),
            ("Ext/SessionModule.bsl", "Configuration", "SessionModule"),
            (
                "Ext/ExternalConnectionModule.bsl",
                "Configuration",
                "ExternalConnectionModule",
            ),
            (
                "CommonModules/Service/Ext/Module.bsl",
                "CommonModule.Service",
                "Module",
            ),
            (
                "HTTPServices/Api/Ext/Module.bsl",
                "HTTPService.Api",
                "Module",
            ),
            ("WebServices/Api/Ext/Module.bsl", "WebService.Api", "Module"),
            (
                "IntegrationServices/Bus/Ext/Module.bsl",
                "IntegrationService.Bus",
                "Module",
            ),
            (
                "CommonForms/Main/Ext/Form/Module.bsl",
                "CommonForm.Main",
                "FormModule",
            ),
            (
                "CommonCommands/Print/Ext/CommandModule.bsl",
                "CommonCommand.Print",
                "CommandModule",
            ),
            (
                "Catalogs/Items/Ext/ObjectModule.bsl",
                "Catalog.Items",
                "ObjectModule",
            ),
            (
                "DocumentJournals/Sales/Ext/ManagerModule.bsl",
                "DocumentJournal.Sales",
                "ManagerModule",
            ),
            (
                "FilterCriteria/ByPartner/Ext/ManagerModule.bsl",
                "FilterCriterion.ByPartner",
                "ManagerModule",
            ),
            (
                "SettingsStorages/Ui/Ext/ManagerModule.bsl",
                "SettingsStorage.Ui",
                "ManagerModule",
            ),
            (
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
                "InformationRegister.Prices",
                "RecordSetModule",
            ),
            (
                "Constants/Mode/Ext/ValueManagerModule.bsl",
                "Constant.Mode",
                "ValueManagerModule",
            ),
        ];
        for (path, owner, role) in direct_cases {
            assert_module_identity(path, owner, role);
        }

        let nested_kinds = [
            ("Catalogs", "Catalog"),
            ("Documents", "Document"),
            ("ExchangePlans", "ExchangePlan"),
            ("ChartsOfAccounts", "ChartOfAccounts"),
            ("ChartsOfCharacteristicTypes", "ChartOfCharacteristicTypes"),
            ("ChartsOfCalculationTypes", "ChartOfCalculationTypes"),
            ("BusinessProcesses", "BusinessProcess"),
            ("Tasks", "Task"),
            ("Reports", "Report"),
            ("DataProcessors", "DataProcessor"),
            ("InformationRegisters", "InformationRegister"),
            ("AccumulationRegisters", "AccumulationRegister"),
            ("AccountingRegisters", "AccountingRegister"),
            ("CalculationRegisters", "CalculationRegister"),
            ("DocumentJournals", "DocumentJournal"),
            ("Enums", "Enum"),
            ("Constants", "Constant"),
            ("Sequences", "Sequence"),
            ("DocumentNumerators", "DocumentNumerator"),
        ];
        for (directory, tag) in nested_kinds {
            assert_module_identity(
                &format!("{directory}/Owner/Forms/Main/Ext/Form/Module.bsl"),
                &format!("{tag}.Owner"),
                "FormModule",
            );
            assert_module_identity(
                &format!("{directory}/Owner/Commands/Print/Ext/CommandModule.bsl"),
                &format!("{tag}.Owner"),
                "CommandModule",
            );
        }
    }
    #[test]
    fn noncanonical_or_unsupported_module_layouts_are_rejected() {
        for path in [
            "Catalogs/Items/Trash/Ext/FakeModule.bsl",
            "Catalogs/Items/Ext/Module.bsl",
            "CommonModules/X/Ext/ObjectModule.bsl",
            "Languages/Ru/Ext/ManagerModule.bsl",
            "Catalogs/Items/Forms/Main/Ext/Module.bsl",
            "Catalogs/Items/Commands/Print/Ext/Module.bsl",
            "Catalogs/Items/Ext/FakeModule.bsl",
        ] {
            let error = module_identity(Path::new(path)).unwrap_err();
            assert!(error.contains("supported canonical"), "{path}: {error}");
        }
    }
    fn assert_module_identity(path: &str, owner: &str, role: &str) {
        let identity = module_identity(Path::new(path)).unwrap_or_else(|error| {
            panic!("expected canonical module identity for {path}: {error}")
        });
        assert_eq!(identity.owner, owner, "{path}");
        assert_eq!(identity.role.as_str(), role, "{path}");
    }
}
