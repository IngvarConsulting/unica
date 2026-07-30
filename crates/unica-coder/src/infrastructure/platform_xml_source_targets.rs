use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, SourceTargetError, SourceTargetErrorCode,
    TargetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::{
    metadata_kind, metadata_kind_by_directory, supports_direct_module_role,
    supports_nested_form_or_command,
};
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::source_roots::{
    normalize_path_identity, resolve_named_source_set, NamedSourceSetError,
    NamedSourceSetErrorKind, ResolvedNamedSourceSet,
};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub(crate) struct PlatformXmlResolution {
    pub(crate) resolved: ResolvedTarget,
    pub(crate) handle: ClosedPlatformXmlTarget,
}

#[derive(Clone)]
pub(crate) struct ClosedPlatformXmlTarget {
    source_target: SourceTarget,
    workspace_root: PathBuf,
    source_root_lexical: PathBuf,
    source_root: PathBuf,
    source_set_kind: SourceSetKind,
    source_format: SourceFormat,
    target_path: PathBuf,
}

impl fmt::Debug for ClosedPlatformXmlTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClosedPlatformXmlTarget")
            .field("source_target", &self.source_target)
            .field("source_set_kind", &self.source_set_kind)
            .field("source_format", &self.source_format)
            .field("physical_handle", &"<closed>")
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct RevalidatedPlatformXmlTarget {
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformXmlModuleIdentity {
    pub(crate) owner: String,
    pub(crate) address: MetadataAddress,
    pub(crate) role: PlatformXmlModuleRole,
    pub(crate) descriptors: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlModuleRole {
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
    pub(crate) fn as_str(self) -> &'static str {
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

pub(crate) fn resolve_platform_xml_target(
    context: &WorkspaceContext,
    target: &SourceTarget,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    if target.source_set.is_empty() {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::SourceSetRequired,
            "sourceSet must name an exact project source set",
        ));
    }
    let selected = resolve_named_source_set(context, &target.source_set)
        .map_err(|error| public_source_set_error(&target.source_set, error))?;
    validate_source_set(&selected)?;
    if target.metadata_path.is_none() {
        return resolve_platform_xml_root(context, target, selected);
    }
    let address = target.metadata_path.as_ref().expect("checked above");
    if address.target_kind() != TargetKind::Module {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "metadataPath does not identify a module terminal",
        ));
    }

    let relative_path = module_path_for_address(address).map_err(|error| {
        SourceTargetError::new(SourceTargetErrorCode::MetadataAddressNotFound, error)
    })?;
    let identity = platform_xml_module_identity(&relative_path).map_err(|error| {
        SourceTargetError::new(SourceTargetErrorCode::MetadataAddressNotFound, error)
    })?;
    if &identity.address != address {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            "module layout did not round-trip to the requested metadata address",
        ));
    }
    validate_platform_xml_module_descriptors(context, &selected.path, &identity.descriptors)
        .map_err(|error| public_evidence_error(target, error))?;

    let target_path = selected.path.join(&relative_path);
    let target_path = WorkspacePathPolicy::new(context)
        .resolve_write(target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    ensure_no_link_components(&selected.path, &target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    validate_regular_module(&target_path, &target.source_set)?;
    let target_identity = normalize_path_identity(&target_path)
        .map_err(|_| target_containment_error(&target.source_set))?;
    if !target_identity.starts_with(&selected.path) {
        return Err(target_containment_error(&target.source_set));
    }

    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| target_containment_error(&target.source_set))?;
    Ok(PlatformXmlResolution {
        resolved: ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: Some(identity.address),
            target_kind: TargetKind::Module,
        },
        handle: ClosedPlatformXmlTarget {
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path,
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_path,
        },
    })
}

fn resolve_platform_xml_root(
    context: &WorkspaceContext,
    target: &SourceTarget,
    selected: ResolvedNamedSourceSet,
) -> Result<PlatformXmlResolution, SourceTargetError> {
    let root_path = WorkspacePathPolicy::new(context)
        .resolve_write(selected.lexical_path.clone())
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    let metadata = fs::symlink_metadata(&root_path)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(source_root_containment_error(&target.source_set));
    }
    let root_identity = normalize_path_identity(&root_path)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    if root_identity != selected.path {
        return Err(source_root_containment_error(&target.source_set));
    }
    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| source_root_containment_error(&target.source_set))?;
    Ok(PlatformXmlResolution {
        resolved: ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: None,
            target_kind: TargetKind::SourceRoot,
        },
        handle: ClosedPlatformXmlTarget {
            source_target: target.clone(),
            workspace_root,
            source_root_lexical: selected.lexical_path,
            source_root: selected.path.clone(),
            source_set_kind: selected.source_set.kind,
            source_format: selected.source_set.source_format,
            target_path: selected.path,
        },
    })
}

pub(crate) fn revalidate_platform_xml_target(
    context: &WorkspaceContext,
    handle: &ClosedPlatformXmlTarget,
) -> Result<RevalidatedPlatformXmlTarget, SourceTargetError> {
    let workspace_root = normalize_path_identity(&context.workspace_root)
        .map_err(|_| source_map_rebind_error(&handle.source_target.source_set))?;
    if workspace_root != handle.workspace_root {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }
    let selected = resolve_named_source_set(context, &handle.source_target.source_set)
        .map_err(|_| source_map_rebind_error(&handle.source_target.source_set))?;
    if selected.lexical_path != handle.source_root_lexical
        || selected.path != handle.source_root
        || selected.source_set.kind != handle.source_set_kind
        || selected.source_set.source_format != handle.source_format
    {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }

    let current = resolve_platform_xml_target(context, &handle.source_target)?;
    if current.handle.target_path != handle.target_path {
        return Err(source_map_rebind_error(&handle.source_target.source_set));
    }
    Ok(RevalidatedPlatformXmlTarget {
        path: current.handle.target_path,
    })
}

fn public_source_set_error(source_set: &str, error: NamedSourceSetError) -> SourceTargetError {
    match error.kind {
        NamedSourceSetErrorKind::NotFound => SourceTargetError::new(
            SourceTargetErrorCode::SourceSetNotFound,
            format!("sourceSet `{source_set}` was not found"),
        ),
        NamedSourceSetErrorKind::Containment => source_root_containment_error(source_set),
        NamedSourceSetErrorKind::Ambiguous => SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!("sourceSet `{source_set}` is ambiguous"),
        ),
        NamedSourceSetErrorKind::Discovery => SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!("sourceSet `{source_set}` is not addressable"),
        ),
    }
}

fn public_evidence_error(
    target: &SourceTarget,
    error: PlatformXmlEvidenceError,
) -> SourceTargetError {
    match error.kind {
        PlatformXmlEvidenceErrorKind::Containment => target_containment_error(&target.source_set),
        PlatformXmlEvidenceErrorKind::Unavailable | PlatformXmlEvidenceErrorKind::NotRegular => {
            SourceTargetError::new(
                SourceTargetErrorCode::MetadataAddressNotFound,
                format!(
                    "metadata owner evidence is unavailable for `{}` in sourceSet `{}`",
                    target
                        .metadata_path
                        .as_ref()
                        .map(MetadataAddress::as_str)
                        .unwrap_or("<root>"),
                    target.source_set
                ),
            )
        }
    }
}

fn target_containment_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("resolved target containment was denied for sourceSet `{source_set}`"),
    )
}

fn source_root_containment_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("selected source root containment was denied for sourceSet `{source_set}`"),
    )
}

fn source_map_rebind_error(source_set: &str) -> SourceTargetError {
    SourceTargetError::new(
        SourceTargetErrorCode::ContainmentDenied,
        format!("source-map binding changed for sourceSet `{source_set}`"),
    )
}

fn validate_source_set(selected: &ResolvedNamedSourceSet) -> Result<(), SourceTargetError> {
    if !matches!(
        selected.source_set.kind,
        SourceSetKind::Configuration | SourceSetKind::Extension
    ) || selected.source_set.source_format != SourceFormat::PlatformXml
    {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::SourceRootNotAddressable,
            format!(
                "source set `{}` must be a Platform XML Configuration or Extension",
                selected.source_set.name
            ),
        ));
    }
    Ok(())
}

fn validate_regular_module(path: &Path, source_set: &str) -> Result<(), SourceTargetError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            format!("module target is unavailable in sourceSet `{source_set}`"),
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(target_containment_error(source_set));
    }
    if !metadata.is_file() {
        return Err(SourceTargetError::new(
            SourceTargetErrorCode::MetadataAddressNotFound,
            format!("module target is not a regular file in sourceSet `{source_set}`"),
        ));
    }
    Ok(())
}

pub(crate) fn platform_xml_module_identity(
    relative: &Path,
) -> Result<PlatformXmlModuleIdentity, String> {
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

const PLATFORM_XML_MODULE_LAYOUT_FAMILIES: &[PlatformXmlModuleLayoutFamily] = &[
    PlatformXmlModuleLayoutFamily::RootApplication,
    PlatformXmlModuleLayoutFamily::OwnerModule,
    PlatformXmlModuleLayoutFamily::CommonForm,
    PlatformXmlModuleLayoutFamily::CommonCommand,
    PlatformXmlModuleLayoutFamily::DirectMetadata,
    PlatformXmlModuleLayoutFamily::NestedForm,
    PlatformXmlModuleLayoutFamily::NestedCommand,
];

const OWNER_MODULE_KINDS: &[&str] = &[
    "CommonModule",
    "HTTPService",
    "WebService",
    "IntegrationService",
];

impl PlatformXmlModuleLayoutFamily {
    fn identity_from_parts(
        self,
        parts: &[&str],
    ) -> Option<Result<PlatformXmlModuleIdentity, String>> {
        match self {
            Self::RootApplication => {
                let ["Ext", file] = parts else {
                    return None;
                };
                let role = root_module_role(file)?;
                Some(module_identity(
                    role.as_str().to_string(),
                    "Configuration".to_string(),
                    role,
                    vec![PathBuf::from("Configuration.xml")],
                ))
            }
            Self::OwnerModule => {
                let [directory, name, "Ext", "Module.bsl"] = parts else {
                    return None;
                };
                let kind = metadata_kind_by_directory(directory)?;
                if !OWNER_MODULE_KINDS.contains(&kind.tag) {
                    return None;
                }
                Some(metadata_module_identity(
                    directory,
                    name,
                    PlatformXmlModuleRole::Module,
                ))
            }
            Self::CommonForm => {
                let ["CommonForms", name, "Ext", "Form", "Module.bsl"] = parts else {
                    return None;
                };
                Some(module_identity(
                    format!("CommonForm.{name}.FormModule"),
                    format!("CommonForm.{name}"),
                    PlatformXmlModuleRole::FormModule,
                    vec![metadata_descriptor("CommonForms", name)],
                ))
            }
            Self::CommonCommand => {
                let ["CommonCommands", name, "Ext", "CommandModule.bsl"] = parts else {
                    return None;
                };
                Some(module_identity(
                    format!("CommonCommand.{name}.CommandModule"),
                    format!("CommonCommand.{name}"),
                    PlatformXmlModuleRole::CommandModule,
                    vec![metadata_descriptor("CommonCommands", name)],
                ))
            }
            Self::DirectMetadata => {
                let [directory, name, "Ext", file] = parts else {
                    return None;
                };
                let role = direct_module_role(file)?;
                let kind = metadata_kind_by_directory(directory)?;
                if !supports_direct_module_role(kind.tag, role.as_str()) {
                    return None;
                }
                Some(metadata_module_identity(directory, name, role))
            }
            Self::NestedForm => {
                let [directory, name, "Forms", form, "Ext", "Form", "Module.bsl"] = parts else {
                    return None;
                };
                Some(nested_module_identity(
                    directory,
                    name,
                    "Form",
                    form,
                    PlatformXmlModuleRole::FormModule,
                ))
            }
            Self::NestedCommand => {
                let [directory, name, "Commands", command, "Ext", "CommandModule.bsl"] = parts
                else {
                    return None;
                };
                Some(nested_module_identity(
                    directory,
                    name,
                    "Command",
                    command,
                    PlatformXmlModuleRole::CommandModule,
                ))
            }
        }
    }

    fn path_from_address(self, parts: &[&str]) -> Option<Result<PathBuf, String>> {
        match self {
            Self::RootApplication => {
                let [terminal] = parts else {
                    return None;
                };
                let role = root_module_role_name(terminal)?;
                Some(Ok(
                    PathBuf::from("Ext").join(format!("{}.bsl", role.as_str()))
                ))
            }
            Self::OwnerModule => {
                let [kind, name, "Module"] = parts else {
                    return None;
                };
                if !OWNER_MODULE_KINDS.contains(kind) {
                    return None;
                }
                Some(directory_for_kind(kind).map(|directory| {
                    PathBuf::from(directory)
                        .join(name)
                        .join("Ext")
                        .join("Module.bsl")
                }))
            }
            Self::CommonForm => {
                let ["CommonForm", name, "FormModule"] = parts else {
                    return None;
                };
                Some(Ok(PathBuf::from("CommonForms")
                    .join(name)
                    .join("Ext")
                    .join("Form")
                    .join("Module.bsl")))
            }
            Self::CommonCommand => {
                let ["CommonCommand", name, "CommandModule"] = parts else {
                    return None;
                };
                Some(Ok(PathBuf::from("CommonCommands")
                    .join(name)
                    .join("Ext")
                    .join("CommandModule.bsl")))
            }
            Self::DirectMetadata => {
                let [kind, name, role_name] = parts else {
                    return None;
                };
                let role = direct_module_role_name(role_name)?;
                if !supports_direct_module_role(kind, role.as_str()) {
                    return None;
                }
                Some(directory_for_kind(kind).map(|directory| {
                    PathBuf::from(directory)
                        .join(name)
                        .join("Ext")
                        .join(format!("{}.bsl", role.as_str()))
                }))
            }
            Self::NestedForm => {
                let [kind, name, "Form", form, "FormModule"] = parts else {
                    return None;
                };
                if !supports_nested_form_or_command(kind) {
                    return None;
                }
                Some(directory_for_kind(kind).map(|directory| {
                    PathBuf::from(directory)
                        .join(name)
                        .join("Forms")
                        .join(form)
                        .join("Ext")
                        .join("Form")
                        .join("Module.bsl")
                }))
            }
            Self::NestedCommand => {
                let [kind, name, "Command", command, "CommandModule"] = parts else {
                    return None;
                };
                if !supports_nested_form_or_command(kind) {
                    return None;
                }
                Some(directory_for_kind(kind).map(|directory| {
                    PathBuf::from(directory)
                        .join(name)
                        .join("Commands")
                        .join(command)
                        .join("Ext")
                        .join("CommandModule.bsl")
                }))
            }
        }
    }
}

fn module_layout_for_relative(
    relative: &Path,
) -> Result<(PlatformXmlModuleLayoutFamily, PlatformXmlModuleIdentity), String> {
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "BSL module path is not valid UTF-8".to_string()),
            _ => Err("BSL module path must be relative to its source set".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(identity) = family.identity_from_parts(&parts) {
            return identity.map(|identity| (*family, identity));
        }
    }
    Err(unsupported_module_layout())
}

#[cfg(test)]
fn module_layout_family_id_for_relative(
    relative: &Path,
) -> Result<PlatformXmlModuleLayoutFamily, String> {
    module_layout_for_relative(relative).map(|(family, _)| family)
}

fn metadata_module_identity(
    directory: &str,
    name: &str,
    role: PlatformXmlModuleRole,
) -> Result<PlatformXmlModuleIdentity, String> {
    let kind = metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    let owner = format!("{}.{name}", kind.tag);
    module_identity(
        format!("{owner}.{}", role.as_str()),
        owner,
        role,
        vec![metadata_descriptor(directory, name)],
    )
}

fn nested_module_identity(
    directory: &str,
    name: &str,
    nested_kind: &str,
    nested_name: &str,
    role: PlatformXmlModuleRole,
) -> Result<PlatformXmlModuleIdentity, String> {
    let kind = metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    if !supports_nested_form_or_command(kind.tag) {
        return Err(unsupported_module_layout());
    }
    let child_directory = match nested_kind {
        "Form" => "Forms",
        "Command" => "Commands",
        _ => return Err(unsupported_module_layout()),
    };
    let owner = format!("{}.{name}", kind.tag);
    module_identity(
        format!("{owner}.{nested_kind}.{nested_name}.{}", role.as_str()),
        owner,
        role,
        vec![
            metadata_descriptor(directory, name),
            PathBuf::from(directory)
                .join(name)
                .join(child_directory)
                .join(nested_name)
                .join("Ext")
                .join(format!("{nested_kind}.xml")),
        ],
    )
}

fn module_identity(
    address: String,
    owner: String,
    role: PlatformXmlModuleRole,
    descriptors: Vec<PathBuf>,
) -> Result<PlatformXmlModuleIdentity, String> {
    let address = MetadataAddress::parse(
        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
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

fn module_path_for_address(address: &MetadataAddress) -> Result<PathBuf, String> {
    let parts = address.segments().collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if let Some(path) = family.path_from_address(&parts) {
            return path;
        }
    }
    Err(unsupported_module_layout())
}

#[cfg(test)]
fn module_layout_family_id_for_address(
    address: &MetadataAddress,
) -> Result<PlatformXmlModuleLayoutFamily, String> {
    let parts = address.segments().collect::<Vec<_>>();
    for family in PLATFORM_XML_MODULE_LAYOUT_FAMILIES {
        if family.path_from_address(&parts).is_some() {
            return Ok(*family);
        }
    }
    Err(unsupported_module_layout())
}

fn directory_for_kind(kind: &str) -> Result<&'static str, String> {
    metadata_kind(kind)
        .map(|kind| kind.directory)
        .ok_or_else(unsupported_module_layout)
}

fn direct_module_role(file: &str) -> Option<PlatformXmlModuleRole> {
    file.strip_suffix(".bsl").and_then(direct_module_role_name)
}

fn direct_module_role_name(role: &str) -> Option<PlatformXmlModuleRole> {
    match role {
        "ObjectModule" => Some(PlatformXmlModuleRole::ObjectModule),
        "ManagerModule" => Some(PlatformXmlModuleRole::ManagerModule),
        "RecordSetModule" => Some(PlatformXmlModuleRole::RecordSetModule),
        "ValueManagerModule" => Some(PlatformXmlModuleRole::ValueManagerModule),
        _ => None,
    }
}

fn root_module_role(file: &str) -> Option<PlatformXmlModuleRole> {
    file.strip_suffix(".bsl").and_then(root_module_role_name)
}

fn root_module_role_name(role: &str) -> Option<PlatformXmlModuleRole> {
    match role {
        "ManagedApplicationModule" => Some(PlatformXmlModuleRole::ManagedApplicationModule),
        "OrdinaryApplicationModule" => Some(PlatformXmlModuleRole::OrdinaryApplicationModule),
        "SessionModule" => Some(PlatformXmlModuleRole::SessionModule),
        "ExternalConnectionModule" => Some(PlatformXmlModuleRole::ExternalConnectionModule),
        _ => None,
    }
}

fn metadata_descriptor(directory: &str, name: &str) -> PathBuf {
    PathBuf::from(directory).join(format!("{name}.xml"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformXmlEvidenceErrorKind {
    Containment,
    Unavailable,
    NotRegular,
}

#[derive(Debug)]
pub(crate) struct PlatformXmlEvidenceError {
    kind: PlatformXmlEvidenceErrorKind,
    private_diagnostic: String,
}

impl PlatformXmlEvidenceError {
    fn new(kind: PlatformXmlEvidenceErrorKind, private_diagnostic: impl Into<String>) -> Self {
        Self {
            kind,
            private_diagnostic: private_diagnostic.into(),
        }
    }
}

impl fmt::Display for PlatformXmlEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.private_diagnostic)
    }
}

impl std::error::Error for PlatformXmlEvidenceError {}

pub(crate) fn validate_platform_xml_module_descriptors(
    context: &WorkspaceContext,
    source_root: &Path,
    descriptors: &[PathBuf],
) -> Result<(), PlatformXmlEvidenceError> {
    for descriptor in descriptors {
        let path = WorkspacePathPolicy::new(context)
            .resolve_write(source_root.join(descriptor))
            .map_err(|error| {
                PlatformXmlEvidenceError::new(
                    PlatformXmlEvidenceErrorKind::Containment,
                    format!("BSL module descriptor containment denied: {error}"),
                )
            })?;
        ensure_no_link_components(source_root, &path).map_err(|error| {
            PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::Containment,
                format!("BSL module descriptor containment denied: {error}"),
            )
        })?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::Unavailable,
                format!(
                    "BSL module metadata descriptor is unavailable {}: {error}",
                    path.display()
                ),
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            return Err(PlatformXmlEvidenceError::new(
                PlatformXmlEvidenceErrorKind::NotRegular,
                format!(
                    "BSL module metadata descriptor must be a regular file: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn ensure_no_link_components(source_root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(source_root)
        .map_err(|_| "path is outside its selected source set".to_string())?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "failed to inspect path component {}: {error}",
                    current.display()
                ));
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "path component must not be a symbolic link or reparse point: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn unsupported_module_layout() -> String {
    "unica.code.patch v1 accepts only a supported canonical platform XML BSL module path"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{resolve_platform_xml_target, revalidate_platform_xml_target};
    use crate::domain::source_target::{
        MetadataAddress, SourceTarget, SourceTargetErrorCode, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::filesystem::create_dir_symlink_for_test;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use crate::infrastructure::source_roots::normalize_path_identity;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn platform_xml_source_targets_resolve_identical_addresses_in_configuration_and_extension() {
        let context = fixture(
            "config-extension",
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n  - name: addOn\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            write_module_fixture(
                &context.workspace_root.join(root),
                "CommonModules/Shared.xml",
                "CommonModules/Shared/Ext/Module.bsl",
            );
        }

        let configuration =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        let extension =
            resolve_platform_xml_target(&context, &target("addOn", "CommonModule.Shared.Module"))
                .unwrap();

        assert_eq!(
            configuration
                .resolved
                .metadata_path
                .as_ref()
                .unwrap()
                .as_str(),
            extension.resolved.metadata_path.as_ref().unwrap().as_str()
        );
        assert_eq!(
            configuration.handle.target_path,
            normalize_path_identity(
                &context
                    .workspace_root
                    .join("cfg/CommonModules/Shared/Ext/Module.bsl")
            )
            .unwrap()
        );
        assert_eq!(
            extension.handle.target_path,
            normalize_path_identity(
                &context
                    .workspace_root
                    .join("ext/CommonModules/Shared/Ext/Module.bsl")
            )
            .unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_target_handle_debug_does_not_disclose_physical_paths() {
        let context = fixture("closed-debug", project_yaml("main", "CONFIGURATION", "src"));
        write_module_fixture(
            &context.workspace_root.join("src"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();

        let debug = format!("{:?}", resolution.handle);

        assert!(!debug.contains(&context.workspace_root.display().to_string()));
        assert!(!debug.contains("Module.bsl"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_wrong_source_set_kind_and_format() {
        let context = fixture(
            "wrong-kind-format",
            "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n  - name: edt\n    type: CONFIGURATION\n    path: edt\n",
        );
        fs::create_dir_all(context.workspace_root.join("external")).unwrap();
        fs::create_dir_all(context.workspace_root.join("edt/Configuration")).unwrap();
        fs::write(context.workspace_root.join("edt/.project"), "edt").unwrap();

        for source_set in ["external", "edt"] {
            let error = resolve_platform_xml_target(
                &context,
                &target(source_set, "CommonModule.Shared.Module"),
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                SourceTargetErrorCode::SourceRootNotAddressable,
                "{source_set}: {error}"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_require_descriptor_evidence() {
        let context = fixture(
            "missing-descriptor",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let module = context
            .workspace_root
            .join("src/CommonModules/Missing/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Missing.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(
            error.message,
            "metadata owner evidence is unavailable for `CommonModule.Missing.Module` in sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("CommonModules/Missing.xml"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_serialize_missing_modules_without_io_details() {
        let context = fixture(
            "missing-module",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let descriptor = context.workspace_root.join("src/CommonModules/Missing.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(&descriptor, "<MetaDataObject/>").unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Missing.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        assert_eq!(
            error.message,
            "module target is unavailable in sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("os error"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_never_fall_back_from_the_named_source_set() {
        let context = fixture("exact-name", project_yaml("main", "CONFIGURATION", "src"));
        write_module_fixture(
            &context.workspace_root.join("src"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );

        let error =
            resolve_platform_xml_target(&context, &target("missing", "CommonModule.Shared.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceSetNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_duplicate_exact_source_set_names() {
        let context = fixture(
            "duplicate-name",
            "format: DESIGNER\nsource-set:\n  - name: duplicate\n    type: CONFIGURATION\n    path: cfg\n  - name: duplicate\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            write_module_fixture(
                &context.workspace_root.join(root),
                "CommonModules/Shared.xml",
                "CommonModules/Shared/Ext/Module.bsl",
            );
        }

        let error = resolve_platform_xml_target(
            &context,
            &target("duplicate", "CommonModule.Shared.Module"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::SourceRootNotAddressable);
        assert!(error.message.contains("ambiguous"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_cover_every_registered_module_layout_family() {
        let context = fixture(
            "layout-families",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        for descriptor in [
            "Configuration.xml",
            "CommonModules/Shared.xml",
            "Catalogs/Items.xml",
            "InformationRegisters/Prices.xml",
            "Constants/Mode.xml",
            "CommonForms/Main.xml",
            "CommonCommands/Print.xml",
            "Catalogs/Items/Forms/List/Ext/Form.xml",
            "Catalogs/Items/Commands/Open/Ext/Command.xml",
        ] {
            let path = root.join(descriptor);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "<MetaDataObject/>").unwrap();
        }
        let cases = [
            (
                "ManagedApplicationModule",
                "Ext/ManagedApplicationModule.bsl",
            ),
            (
                "CommonModule.Shared.Module",
                "CommonModules/Shared/Ext/Module.bsl",
            ),
            (
                "Catalog.Items.ObjectModule",
                "Catalogs/Items/Ext/ObjectModule.bsl",
            ),
            (
                "Catalog.Items.ManagerModule",
                "Catalogs/Items/Ext/ManagerModule.bsl",
            ),
            (
                "InformationRegister.Prices.RecordSetModule",
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
            ),
            (
                "Constant.Mode.ValueManagerModule",
                "Constants/Mode/Ext/ValueManagerModule.bsl",
            ),
            (
                "CommonForm.Main.FormModule",
                "CommonForms/Main/Ext/Form/Module.bsl",
            ),
            (
                "CommonCommand.Print.CommandModule",
                "CommonCommands/Print/Ext/CommandModule.bsl",
            ),
            (
                "Catalog.Items.Form.List.FormModule",
                "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            ),
            (
                "Catalog.Items.Command.Open.CommandModule",
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            ),
        ];
        for (address, relative) in cases {
            let module = root.join(relative);
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();

            let resolution =
                resolve_platform_xml_target(&context, &target("main", address)).unwrap();

            assert_eq!(
                resolution.handle.target_path,
                normalize_path_identity(&module).unwrap(),
                "{address}"
            );
            assert_eq!(
                resolution.resolved.metadata_path.unwrap().as_str(),
                address,
                "{relative}"
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_module_layouts_share_one_family_registry_in_both_directions() {
        let cases = [
            (
                "ManagedApplicationModule",
                "Ext/ManagedApplicationModule.bsl",
            ),
            (
                "CommonModule.Shared.Module",
                "CommonModules/Shared/Ext/Module.bsl",
            ),
            (
                "CommonForm.Main.FormModule",
                "CommonForms/Main/Ext/Form/Module.bsl",
            ),
            (
                "CommonCommand.Print.CommandModule",
                "CommonCommands/Print/Ext/CommandModule.bsl",
            ),
            (
                "Catalog.Items.ManagerModule",
                "Catalogs/Items/Ext/ManagerModule.bsl",
            ),
            (
                "Catalog.Items.Form.List.FormModule",
                "Catalogs/Items/Forms/List/Ext/Form/Module.bsl",
            ),
            (
                "Catalog.Items.Command.Open.CommandModule",
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
            ),
        ];

        for (address, relative) in cases {
            let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, address).unwrap();
            let from_address = super::module_layout_family_id_for_address(&address).unwrap();
            let from_path =
                super::module_layout_family_id_for_relative(Path::new(relative)).unwrap();

            assert_eq!(from_address, from_path, "{address} <-> {relative}");
        }
    }

    #[test]
    fn platform_xml_source_targets_reject_unregistered_module_roles() {
        let context = fixture(
            "unregistered-role",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        write_module_fixture(
            &context.workspace_root.join("src"),
            "Languages/Russian.xml",
            "Languages/Russian/Ext/ManagerModule.bsl",
        );

        let error = resolve_platform_xml_target(
            &context,
            &target("main", "Language.Russian.ManagerModule"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::MetadataAddressNotFound);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_revalidation_rejects_a_replaced_symlink_target() {
        let context = fixture(
            "revalidate-symlink",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        let target = root.join("CommonModules/Shared/Ext/Module.bsl");
        let real = root.join("CommonModules/Shared/Ext/Replacement.bsl");
        fs::write(&real, "Procedure Changed()\nEndProcedure\n").unwrap();
        fs::remove_file(&target).unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &target)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&context);
            return;
        }

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_symlinked_target() {
        let context = fixture(
            "symlink-target",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(root.join("CommonModules/Shared/Ext")).unwrap();
        fs::write(root.join("CommonModules/Shared.xml"), "<MetaDataObject/>").unwrap();
        let real = root.join("CommonModules/Shared/Ext/RealModule.bsl");
        let target = root.join("CommonModules/Shared/Ext/Module.bsl");
        fs::write(&real, "Procedure Run()\nEndProcedure\n").unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &target)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&context);
            return;
        }

        let error = resolve_platform_xml_target(
            &context,
            &target_for("main", "CommonModule.Shared.Module"),
        )
        .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_symlinked_layout_ancestor() {
        let context = fixture(
            "symlink-ancestor",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        write_module_fixture(
            &root,
            "RealCommonModules/Shared.xml",
            "RealCommonModules/Shared/Ext/Module.bsl",
        );
        let Some(result) =
            create_dir_symlink_for_test(root.join("RealCommonModules"), root.join("CommonModules"))
        else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "resolved target containment was denied for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("CommonModules"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_resolve_and_revalidate_configuration_and_extension_roots() {
        let context = fixture(
            "source-roots",
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n  - name: addOn\n    type: EXTENSION\n    path: ext\n",
        );
        for root in ["cfg", "ext"] {
            fs::create_dir_all(context.workspace_root.join(root)).unwrap();
        }

        for (source_set, root) in [("main", "cfg"), ("addOn", "ext")] {
            let resolution =
                resolve_platform_xml_target(&context, &root_target(source_set)).unwrap();

            assert_eq!(resolution.resolved.source_set, source_set);
            assert_eq!(resolution.resolved.metadata_path, None);
            assert_eq!(
                resolution.resolved.target_kind,
                crate::domain::source_target::TargetKind::SourceRoot
            );
            assert_eq!(
                revalidate_platform_xml_target(&context, &resolution.handle)
                    .unwrap()
                    .path,
                normalize_path_identity(&context.workspace_root.join(root)).unwrap()
            );
        }
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_a_configured_source_root_symlink() {
        let context = fixture(
            "symlink-source-root",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let real = context.workspace_root.join("real-src");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        let Some(result) = create_dir_symlink_for_test(&real, context.workspace_root.join("src"))
        else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error = resolve_platform_xml_target(&context, &root_target("main")).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "selected source root containment was denied for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("real-src"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_reject_a_source_root_replaced_by_a_symlink() {
        let context = fixture(
            "revalidate-source-root-symlink",
            project_yaml("main", "CONFIGURATION", "src"),
        );
        let root = context.workspace_root.join("src");
        fs::create_dir_all(&root).unwrap();
        let resolution = resolve_platform_xml_target(&context, &root_target("main")).unwrap();
        let replacement = context.workspace_root.join("replacement");
        fs::rename(&root, &replacement).unwrap();
        let Some(result) = create_dir_symlink_for_test(&replacement, &root) else {
            cleanup(&context);
            return;
        };
        result.unwrap();

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert_eq!(
            error.message,
            "source-map binding changed for sourceSet `main`"
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains(&context.workspace_root.display().to_string()));
        assert!(!serialized.contains("replacement"));
        cleanup(&context);
    }

    #[test]
    fn platform_xml_source_targets_revalidate_source_map_binding() {
        let context = fixture(
            "source-map-rebind",
            project_yaml("main", "EXTENSION", "ext"),
        );
        write_module_fixture(
            &context.workspace_root.join("ext"),
            "CommonModules/Shared.xml",
            "CommonModules/Shared/Ext/Module.bsl",
        );
        let resolution =
            resolve_platform_xml_target(&context, &target("main", "CommonModule.Shared.Module"))
                .unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            project_yaml("main", "EXTENSION", "other"),
        )
        .unwrap();
        fs::create_dir_all(context.workspace_root.join("other")).unwrap();

        let error = revalidate_platform_xml_target(&context, &resolution.handle).unwrap_err();

        assert_eq!(error.code, SourceTargetErrorCode::ContainmentDenied);
        assert!(error.message.contains("source-map"));
        cleanup(&context);
    }

    fn target(source_set: &str, metadata_path: &str) -> SourceTarget {
        SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: Some(
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, metadata_path).unwrap(),
            ),
        }
    }

    fn root_target(source_set: &str) -> SourceTarget {
        SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: None,
        }
    }

    fn target_for(source_set: &str, metadata_path: &str) -> SourceTarget {
        target(source_set, metadata_path)
    }

    fn fixture(name: &str, yaml: impl AsRef<str>) -> WorkspaceContext {
        let root = temp_root(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("v8project.yaml"), yaml.as_ref()).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn project_yaml(name: &str, kind: &str, path: &str) -> String {
        format!(
            "format: DESIGNER\nsource-set:\n  - name: {name}\n    type: {kind}\n    path: {path}\n"
        )
    }

    fn write_module_fixture(root: &Path, descriptor: &str, module: &str) {
        let descriptor = root.join(descriptor);
        let module = root.join(module);
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(descriptor, "<MetaDataObject/>").unwrap();
        fs::write(module, "Procedure Run()\nEndProcedure\n").unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-platform-xml-targets-{name}-{}-{nanos}-{nonce}",
            std::process::id()
        ))
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
