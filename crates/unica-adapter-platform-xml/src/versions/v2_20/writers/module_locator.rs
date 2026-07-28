use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use unica_format_core::{
    commands::{
        LocatedModuleArtifact, ModuleArtifactLocation, ModuleOwner, ModuleRole, SourceSetName,
    },
    ports::{ModuleArtifactLocatorPort, ModuleArtifactLocatorRequest, SemanticArtifactLease},
    semantic_ids::SemanticObjectKind,
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

use super::{
    filesystem::metadata_is_link_or_reparse_point,
    platform_xml_owner::{task8_metadata_kind_by_directory, task8_metadata_kind_tag},
    source_roots::{normalize_contained_source_root, normalize_path_identity, resolve_source_root},
};

pub(crate) struct PlatformModuleArtifactLocator;

#[derive(Debug, Clone)]
pub(crate) struct PlatformModuleLocatorSession {
    source_root: Option<PathBuf>,
    source_set: Option<String>,
    explicit_source: Option<String>,
    cwd: PathBuf,
    target: PathBuf,
    authorized_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct PlatformModuleArtifactLease {
    pub(crate) target: PathBuf,
    pub(crate) expected_preimage: Vec<u8>,
    pub(crate) descriptor_preimages: Vec<(PathBuf, Vec<u8>)>,
    pub(crate) source_declaration_preimages: Vec<(PathBuf, Vec<u8>)>,
}

impl PlatformModuleLocatorSession {
    pub(crate) fn new(source_root: &Path, target: &Path, authorized_root: &Path) -> Self {
        Self {
            source_root: Some(source_root.to_path_buf()),
            source_set: None,
            explicit_source: None,
            cwd: authorized_root.to_path_buf(),
            target: target.to_path_buf(),
            authorized_root: authorized_root.to_path_buf(),
        }
    }

    pub(crate) fn from_workspace(
        workspace_root: &Path,
        cwd: &Path,
        target: &Path,
        explicit_source: Option<&str>,
    ) -> Self {
        Self {
            source_root: None,
            source_set: None,
            explicit_source: explicit_source.map(str::to_string),
            cwd: cwd.to_path_buf(),
            target: target.to_path_buf(),
            authorized_root: workspace_root.to_path_buf(),
        }
    }
}

impl ModuleArtifactLocatorPort for PlatformModuleArtifactLocator {
    fn locate(
        &self,
        request: &ModuleArtifactLocatorRequest,
    ) -> Result<LocatedModuleArtifact, SourceAdapterError> {
        if request.cancellation().is_cancelled() {
            return Err(error("cancelled: module artifact location was cancelled"));
        }
        let session = request
            .session()
            .adapter_state::<PlatformModuleLocatorSession>()
            .ok_or_else(|| error("module locator has no bound Platform XML source"))?;
        locate(session).map_err(error)
    }
}

fn locate(session: &PlatformModuleLocatorSession) -> Result<LocatedModuleArtifact, String> {
    let mut source_declaration_preimages = Vec::new();
    let (source_root, source_set) = if let Some(source_root) = &session.source_root {
        (
            normalize_contained_source_root(&session.authorized_root, source_root)?,
            session.source_set.clone(),
        )
    } else {
        let declaration = session.authorized_root.join("v8project.yaml");
        ensure_regular_non_link_path(
            &session.authorized_root,
            &declaration,
            "project source declaration",
        )?;
        source_declaration_preimages.push((
            declaration.clone(),
            fs::read(&declaration)
                .map_err(|error| format!("project source declaration is unavailable: {error}"))?,
        ));
        let context = crate::operations::WorkspaceContext {
            cwd: session.cwd.clone(),
            workspace_root: session.authorized_root.clone(),
            cache_root: session.authorized_root.join(".build/unica"),
            workspace_epoch: 0,
        };
        let resolved = resolve_source_root(&context, session.explicit_source.as_deref())?;
        (resolved.path, resolved.source_set)
    };
    let relative = session
        .target
        .strip_prefix(
            session
                .source_root
                .as_deref()
                .unwrap_or(source_root.as_path()),
        )
        .map_err(|_| "BSL module is outside the selected source set".to_string())?;
    let bound_target = source_root.join(relative);
    ensure_regular_non_link_path(&source_root, &bound_target, "BSL module")?;
    let target = normalize_path_identity(&bound_target)?;
    if !target.starts_with(&source_root) {
        return Err("BSL module is outside the selected source set".to_string());
    }
    let relative = target
        .strip_prefix(&source_root)
        .map_err(|_| "failed to derive BSL module identity".to_string())?;
    let identity = module_identity(relative)?;
    let mut descriptor_preimages = Vec::with_capacity(identity.descriptors.len());
    for descriptor in &identity.descriptors {
        let path = source_root.join(descriptor);
        ensure_regular_non_link_path(&source_root, &path, "BSL module metadata descriptor")?;
        descriptor_preimages.push((
            path.clone(),
            fs::read(&path).map_err(|error| {
                format!("BSL module metadata descriptor is unavailable: {error}")
            })?,
        ));
    }
    if let Some(Component::Normal(directory)) = relative.components().next() {
        let root_owner = source_root.join(directory).with_extension("xml");
        if root_owner.exists()
            && !descriptor_preimages
                .iter()
                .any(|(path, _)| path == &root_owner)
        {
            ensure_regular_non_link_path(&source_root, &root_owner, "source set owner descriptor")?;
            descriptor_preimages.push((
                root_owner.clone(),
                fs::read(&root_owner).map_err(|error| {
                    format!("source set owner descriptor is unavailable: {error}")
                })?,
            ));
        }
    }
    let expected_preimage =
        fs::read(&target).map_err(|error| format!("BSL module is unavailable: {error}"))?;
    let mut location = ModuleArtifactLocation::new(identity.owner, identity.role);
    if let Some(source_set) = source_set {
        location = location.with_source_set(
            SourceSetName::new(source_set)
                .map_err(|error| format!("invalid module source set: {error}"))?,
        );
    }
    Ok(LocatedModuleArtifact::new(
        location,
        SemanticArtifactLease::new(PlatformModuleArtifactLease {
            target,
            expected_preimage,
            descriptor_preimages,
            source_declaration_preimages,
        }),
    ))
}

fn ensure_regular_non_link_path(root: &Path, target: &Path, label: &str) -> Result<(), String> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| format!("{label} is outside the selected source set"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("{label} path is not canonical"));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("{label} is unavailable {}: {error}", current.display()))?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "{label} must not traverse a link: {}",
                current.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(target)
        .map_err(|error| format!("{label} is unavailable {}: {error}", target.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!(
            "{label} must be a regular file: {}",
            target.display()
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct ModuleIdentity {
    owner: ModuleOwner,
    role: ModuleRole,
    descriptors: Vec<PathBuf>,
}

fn module_identity(relative: &Path) -> Result<ModuleIdentity, String> {
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
    match parts.as_slice() {
        ["Ext", file] => configuration_module_identity(file),
        [directory, name, "Ext", "Module.bsl"]
            if matches!(
                *directory,
                "CommonModules" | "HTTPServices" | "WebServices" | "IntegrationServices"
            ) =>
        {
            metadata_module_identity(directory, name, ModuleRole::Module)
        }
        ["CommonForms", name, "Ext", "Form", "Module.bsl"] => object_identity(
            SemanticObjectKind::CommonForm,
            name,
            ModuleRole::Form,
            vec![metadata_descriptor("CommonForms", name)],
        ),
        ["CommonCommands", name, "Ext", "CommandModule.bsl"] => object_identity(
            SemanticObjectKind::CommonCommand,
            name,
            ModuleRole::Command,
            vec![metadata_descriptor("CommonCommands", name)],
        ),
        [directory, name, "Ext", file] => {
            let role = direct_module_role(file).ok_or_else(unsupported_module_layout)?;
            let kind = task8_metadata_kind_by_directory(directory)
                .ok_or_else(unsupported_module_layout)?;
            let tag = task8_metadata_kind_tag(kind).ok_or_else(unsupported_module_layout)?;
            if !direct_role_is_supported(tag, role) {
                return Err(unsupported_module_layout());
            }
            object_identity(kind, name, role, vec![metadata_descriptor(directory, name)])
        }
        [directory, name, "Forms", form, "Ext", "Form", "Module.bsl"] => {
            nested_module_identity(directory, name, "Form", form, ModuleRole::Form)
        }
        [directory, name, "Commands", command, "Ext", "CommandModule.bsl"] => {
            nested_module_identity(directory, name, "Command", command, ModuleRole::Command)
        }
        _ => Err(unsupported_module_layout()),
    }
}

fn configuration_module_identity(file: &str) -> Result<ModuleIdentity, String> {
    let role = match file {
        "ManagedApplicationModule.bsl" => ModuleRole::ManagedApplication,
        "OrdinaryApplicationModule.bsl" => ModuleRole::OrdinaryApplication,
        "SessionModule.bsl" => ModuleRole::Session,
        "ExternalConnectionModule.bsl" => ModuleRole::ExternalConnection,
        _ => return Err(unsupported_module_layout()),
    };
    Ok(ModuleIdentity {
        owner: ModuleOwner::Configuration,
        role,
        descriptors: vec![PathBuf::from("Configuration.xml")],
    })
}

fn metadata_module_identity(
    directory: &str,
    name: &str,
    role: ModuleRole,
) -> Result<ModuleIdentity, String> {
    let kind = task8_metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    object_identity(kind, name, role, vec![metadata_descriptor(directory, name)])
}

fn nested_module_identity(
    directory: &str,
    name: &str,
    nested_kind: &str,
    nested_name: &str,
    role: ModuleRole,
) -> Result<ModuleIdentity, String> {
    let kind = task8_metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    let tag = task8_metadata_kind_tag(kind).ok_or_else(unsupported_module_layout)?;
    if !nested_modules_are_supported(tag) {
        return Err(unsupported_module_layout());
    }
    let child_directory = match nested_kind {
        "Form" => "Forms",
        "Command" => "Commands",
        _ => return Err(unsupported_module_layout()),
    };
    object_identity(
        kind,
        name,
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

fn object_identity(
    kind: SemanticObjectKind,
    name: &str,
    role: ModuleRole,
    descriptors: Vec<PathBuf>,
) -> Result<ModuleIdentity, String> {
    Ok(ModuleIdentity {
        owner: ModuleOwner::object(kind, name).map_err(|error| error.to_string())?,
        role,
        descriptors,
    })
}

fn direct_module_role(file: &str) -> Option<ModuleRole> {
    match file {
        "ObjectModule.bsl" => Some(ModuleRole::Object),
        "ManagerModule.bsl" => Some(ModuleRole::Manager),
        "RecordSetModule.bsl" => Some(ModuleRole::RecordSet),
        "ValueManagerModule.bsl" => Some(ModuleRole::ValueManager),
        _ => None,
    }
}

fn direct_role_is_supported(kind: &str, role: ModuleRole) -> bool {
    match role {
        ModuleRole::Object => matches!(
            kind,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
        ),
        ModuleRole::Manager => matches!(
            kind,
            "Catalog"
                | "Document"
                | "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "ExchangePlan"
                | "Enum"
                | "Report"
                | "DataProcessor"
                | "Constant"
                | "DocumentJournal"
                | "FilterCriterion"
                | "SettingsStorage"
        ),
        ModuleRole::RecordSet => matches!(
            kind,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ),
        ModuleRole::ValueManager => kind == "Constant",
        _ => false,
    }
}

fn nested_modules_are_supported(kind: &str) -> bool {
    matches!(
        kind,
        "Document"
            | "Catalog"
            | "DataProcessor"
            | "Report"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
            | "DocumentJournal"
            | "Enum"
            | "Constant"
            | "Sequence"
            | "DocumentNumerator"
    )
}

fn metadata_descriptor(directory: &str, name: &str) -> PathBuf {
    PathBuf::from(directory).join(format!("{name}.xml"))
}

fn unsupported_module_layout() -> String {
    "unica.code.patch v1 accepts only a supported canonical platform XML BSL module path"
        .to_string()
}

fn error(message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_layout_matrix_is_owned_by_the_versioned_locator() {
        for (path, role) in [
            (
                "Ext/ManagedApplicationModule.bsl",
                ModuleRole::ManagedApplication,
            ),
            ("CommonModules/Service/Ext/Module.bsl", ModuleRole::Module),
            ("HTTPServices/Api/Ext/Module.bsl", ModuleRole::Module),
            ("WebServices/Api/Ext/Module.bsl", ModuleRole::Module),
            ("IntegrationServices/Bus/Ext/Module.bsl", ModuleRole::Module),
            ("CommonForms/Main/Ext/Form/Module.bsl", ModuleRole::Form),
            (
                "CommonCommands/Print/Ext/CommandModule.bsl",
                ModuleRole::Command,
            ),
            ("Catalogs/Items/Ext/ObjectModule.bsl", ModuleRole::Object),
            (
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
                ModuleRole::RecordSet,
            ),
            (
                "Constants/Mode/Ext/ValueManagerModule.bsl",
                ModuleRole::ValueManager,
            ),
            (
                "Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
                ModuleRole::Form,
            ),
            (
                "Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
                ModuleRole::Command,
            ),
        ] {
            let identity = module_identity(Path::new(path)).unwrap_or_else(|error| {
                panic!("expected canonical module identity for {path}: {error}")
            });
            assert_eq!(identity.role, role, "{path}");
        }
    }

    #[test]
    fn noncanonical_layouts_are_rejected_in_the_adapter() {
        for path in [
            "Catalogs/Items/Trash/Ext/FakeModule.bsl",
            "Catalogs/Items/Ext/Module.bsl",
            "CommonModules/X/Ext/ObjectModule.bsl",
            "Languages/Ru/Ext/ManagerModule.bsl",
            "Catalogs/Items/Forms/Main/Ext/Module.bsl",
        ] {
            assert!(module_identity(Path::new(path)).is_err(), "{path}");
        }
    }
}
