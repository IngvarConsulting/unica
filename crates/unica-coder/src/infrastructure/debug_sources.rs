//! Workspace-backed resolution of the saved modules supported by DAP attach.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

use roxmltree::{Document, Node};
use unica_address::{
    address::{NodeKind, QualifiedAddress},
    module_routes::{has_direct_module_route, has_nested_module_role, ModuleRole},
    source_target::MetadataAddress,
};
use unica_source_contract::{
    debug_source::{
        DebugModuleId, DebugModuleMatches, DebugModuleQuery, DebugModuleSource,
        ResolveDebugModules, COMMON_MODULE_PROPERTY_ID, OBJECT_MODULE_PROPERTY_ID,
    },
    read::{FileIdentity, FileStamp, ReadFile, SourceDirectory, SourceSelectionReadPass},
    ProjectSourceSet, SourceFormat, SourceSetKind, SourceSetState,
};
use unica_source_designer::{
    layout::metadata_descriptor_relative,
    module_layout::{
        logical_module_from_metadata, platform_xml_module_relative, PlatformXmlModuleRole,
    },
    registration::root_child_objects,
};
use unica_workspace::{
    discovery::{lexical_contained_source_root, reject_linked_source_root_route},
    read::open_source_directory,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// Only a negative UUID match can avoid reading XML. A candidate source still proves its
// current descriptor, registration and BSL file. Retain at most one reverse query's observations.
type OwnerIds = HashMap<PathBuf, (FileStamp, uuid::Uuid)>;

#[derive(PartialEq, Eq)]
struct DebugSourceRoot {
    name: String,
    kind: SourceSetKind,
    path: PathBuf,
    identity: Option<FileIdentity>,
}

pub fn workspace_debug_resolver(
    workspace: &Path,
    source_set: Option<&str>,
) -> Result<ResolveDebugModules> {
    if !workspace.is_absolute() {
        return Err("rootProject must be an absolute Unica workspace path".into());
    }
    let mut pass = unica_workspace::selection_evidence::open_source_selection_read_pass(workspace)?;
    let workspace = pass.workspace_path().to_path_buf();
    if !unica_source_designer::project_sources::has_debug_source_set(
        &mut pass,
        unica_workspace::paths::host_path_text,
        source_set,
    )? {
        return Err(match source_set {
            Some(name) => format!(
                "debug source set {name} is not available; use the names resolved by workspace"
            ),
            None => "workspace has no supported debug source set".to_string(),
        }
        .into());
    }
    let mut previous = OwnerIds::new();
    let mut source_sets = Vec::new();
    let mut revision = 0u64;
    Ok(Box::new(move |query| {
        let mut observed = OwnerIds::new();
        let (modules, roots) = resolve_modules(&workspace, query, &previous, &mut observed)?;
        if source_sets != roots {
            revision = revision
                .checked_add(1)
                .ok_or("source set revision overflow")?;
            source_sets = roots;
        }
        if matches!(query, DebugModuleQuery::Runtime(_)) {
            previous = observed;
        }
        Ok(DebugModuleMatches {
            modules,
            source_sets_revision: revision,
        })
    }))
}

#[cfg(test)]
fn workspace_debug_modules(
    workspace: &Path,
    query: DebugModuleQuery<'_>,
) -> Result<Vec<DebugModuleSource>> {
    Ok(workspace_debug_resolver(workspace, None)?(query)?.modules)
}

fn resolve_modules(
    workspace: &Path,
    query: DebugModuleQuery<'_>,
    previous: &OwnerIds,
    observed: &mut OwnerIds,
) -> Result<(Vec<DebugModuleSource>, Vec<DebugSourceRoot>)> {
    let address = match query {
        DebugModuleQuery::Address(text) => Some(QualifiedAddress::parse(text)?),
        DebugModuleQuery::Frame { .. } | DebugModuleQuery::Runtime(_) => None,
    };
    let requested_set = address.as_ref().map(QualifiedAddress::source_set);
    let admission =
        super::source_workspace::discover_project_source_admission(workspace, &mut || Ok(()))?;
    let map = admission.map();
    let canonical = Path::new(&map.workspace_root);
    let mut modules = Vec::new();
    let mut roots = Vec::new();
    for set in &map.source_sets {
        if set.source_format != SourceFormat::PlatformXml
            || set.source_state != SourceSetState::Supported
            || !matches!(
                set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        {
            continue;
        }
        let path = lexical_contained_source_root(canonical, &set.path)?;
        let identity = admission.source_root_identity(path.strip_prefix(canonical)?);
        roots.push(DebugSourceRoot {
            name: set.name.clone(),
            kind: set.kind,
            path: path.clone(),
            identity,
        });
        if requested_set.is_some_and(|name| name != set.name) {
            continue;
        }
        let frame_address = if let DebugModuleQuery::Frame { module, id } = query {
            if id.extension.is_some() != (set.kind == SourceSetKind::Extension) {
                continue;
            }
            Some(module.in_source_set(&set.name)?)
        } else {
            None
        };
        reject_linked_source_root_route(canonical, &path)?;
        let root = open_source_directory(&path)?;
        if identity != Some(root.identity()) {
            return Err(format!("debug source set {} changed during discovery", set.name).into());
        }
        read_modules(
            &root,
            set,
            query,
            address.as_ref().or(frame_address.as_ref()),
            &mut modules,
            previous,
            observed,
        )?;
        root.validate_named_identity()?;
    }
    Ok((modules, roots))
}

fn read_modules(
    root: &SourceDirectory,
    source_set: &ProjectSourceSet,
    query: DebugModuleQuery<'_>,
    address: Option<&QualifiedAddress>,
    modules: &mut Vec<DebugModuleSource>,
    previous: &OwnerIds,
    observed: &mut OwnerIds,
) -> Result<()> {
    let configuration = descriptor(root, Path::new("Configuration.xml"))?;
    let document = Document::parse(&configuration)?;
    let owner = metadata_owner(&document, "Configuration")?;
    let extension = if source_set.kind == SourceSetKind::Extension {
        Some(
            property(owner, "Name")
                .ok_or("missing extension name")?
                .to_owned(),
        )
    } else {
        None
    };

    if runtime_ids(query).is_some_and(|ids| !ids.iter().any(|id| id.extension == extension)) {
        return Ok(());
    }
    let owner_filter = address.and_then(|at| at.segments().first());
    let mut index = SourceSetModules {
        root,
        set: &source_set.name,
        query,
        address,
        extension: extension.as_deref(),
        modules,
        previous,
        observed,
        directories: HashMap::new(),
    };
    if index.wants_role(PlatformXmlModuleRole::ManagedApplicationModule)
        && owner_filter.is_none_or(|seg| seg.kind() == NodeKind::Module)
        && root
            .relative_file_stamp(Path::new("Ext/ManagedApplicationModule.bsl"))?
            .is_some()
    {
        index.add(
            "ManagedApplicationModule",
            PlatformXmlModuleRole::ManagedApplicationModule,
            owner,
        )?;
    }
    if owner_filter.is_some_and(|segment| segment.kind() == NodeKind::Module) {
        return Ok(());
    }
    let registrations = root_child_objects(&document)?
        .children()
        .filter(Node::is_element)
        .filter_map(|entry| {
            let name = entry.text().unwrap_or_default().trim();
            if owner_filter.is_some_and(|seg| seg.name() != Some(name)) {
                return None;
            }
            let kind = NodeKind::parse(entry.tag_name().name()).ok()?;
            owner_filter
                .is_none_or(|seg| seg.kind() == kind)
                .then_some((kind, name))
        });
    for (kind, name) in registrations {
        if index.supports_kind(kind) {
            index.read_registered(kind, name)?;
        }
    }
    for parent in index.directories.values() {
        parent.validate_named_identity()?;
    }
    Ok(())
}

struct SourceSetModules<'a> {
    root: &'a SourceDirectory,
    set: &'a str,
    query: DebugModuleQuery<'a>,
    address: Option<&'a QualifiedAddress>,
    extension: Option<&'a str>,
    modules: &'a mut Vec<DebugModuleSource>,
    previous: &'a OwnerIds,
    observed: &'a mut OwnerIds,
    directories: HashMap<PathBuf, SourceDirectory>,
}

impl SourceSetModules<'_> {
    fn wants_role(&self, role: PlatformXmlModuleRole) -> bool {
        runtime_ids(self.query).is_none_or(|ids| {
            ids.iter().any(|id| {
                id.extension.as_deref() == self.extension
                    && Some(id.property) == role_property(role)
            })
        })
    }

    fn supports_kind(&self, kind: NodeKind) -> bool {
        match kind {
            NodeKind::CommonModule => self.wants_role(PlatformXmlModuleRole::Module),
            NodeKind::CommonForm => self.wants_role(PlatformXmlModuleRole::FormModule),
            _ => {
                (self.wants_role(PlatformXmlModuleRole::ObjectModule)
                    && has_direct_module_route(kind, ModuleRole::Object))
                    || (self.wants_role(PlatformXmlModuleRole::FormModule)
                        && has_nested_module_role(kind, ModuleRole::Form))
            }
        }
    }

    fn read_registered(&mut self, kind: NodeKind, name: &str) -> Result<()> {
        let metadata = format!("{}.{name}", kind.as_str());
        match kind {
            NodeKind::CommonModule => {
                self.read(
                    &format!("{metadata}.Module"),
                    PlatformXmlModuleRole::Module,
                    kind,
                    name,
                )?;
            }
            NodeKind::CommonForm => {
                self.read(
                    &format!("{metadata}.FormModule"),
                    PlatformXmlModuleRole::FormModule,
                    kind,
                    name,
                )?;
            }
            _ => self.read_owner(&metadata, kind, name)?,
        }
        Ok(())
    }

    fn read_owner(&mut self, metadata: &str, kind: NodeKind, name: &str) -> Result<()> {
        let descriptor_path = descriptor_path(metadata)?;
        if self.root.relative_file_stamp(&descriptor_path)?.is_none() {
            return Ok(());
        }
        let xml = descriptor(self.root, &descriptor_path)?;
        let document = Document::parse(&xml)?;
        let owner = metadata_owner(&document, kind.as_str())?;
        self.check_name(owner, name)?;

        if self.wants_role(PlatformXmlModuleRole::ObjectModule)
            && has_direct_module_route(kind, ModuleRole::Object)
            && self.address.is_none_or(|at| {
                at.segments()
                    .get(1)
                    .is_some_and(|segment| segment.kind() == NodeKind::Module)
            })
        {
            let target = format!("{metadata}.ObjectModule");
            if self.module_exists(&target)? {
                self.add(&target, PlatformXmlModuleRole::ObjectModule, owner)?;
            }
        }

        if self.wants_role(PlatformXmlModuleRole::FormModule)
            && has_nested_module_role(kind, ModuleRole::Form)
        {
            for form in owner
                .children()
                .filter(|node| node.has_tag_name("ChildObjects"))
                .flat_map(|children| children.children())
                .filter(|node| node.has_tag_name("Form"))
            {
                let form_name = form.text().ok_or("missing registered form name")?.trim();
                if self.address.is_some_and(|at| {
                    !at.segments().get(1).is_some_and(|seg| {
                        seg.kind() == NodeKind::Form && seg.name() == Some(form_name)
                    })
                }) {
                    continue;
                }
                self.read(
                    &format!("{metadata}.Form.{form_name}.FormModule"),
                    PlatformXmlModuleRole::FormModule,
                    NodeKind::Form,
                    form_name,
                )?;
            }
        }
        Ok(())
    }

    fn read(
        &mut self,
        metadata: &str,
        role: PlatformXmlModuleRole,
        kind: NodeKind,
        name: &str,
    ) -> Result<()> {
        let reverse = matches!(self.query, DebugModuleQuery::Runtime(_));
        if !reverse && !self.module_exists(metadata)? {
            return Ok(());
        }
        match self.read_descriptor(metadata, role, kind, name) {
            Err(_) if reverse && !self.module_exists(metadata)? => Ok(()),
            result => result,
        }
    }

    fn read_descriptor(
        &mut self,
        metadata: &str,
        role: PlatformXmlModuleRole,
        kind: NodeKind,
        name: &str,
    ) -> Result<()> {
        let owner_metadata = metadata
            .rsplit_once('.')
            .ok_or("debug module has no metadata owner")?
            .0;
        let path = descriptor_path(owner_metadata)?;
        let mut file = self.open_descriptor(&path)?;
        let key = self.root.path().join(&path);
        let stamp = if let DebugModuleQuery::Runtime(ids) = self.query {
            let stamp = file.stamp().ok();
            if let Some((stamp, object)) = stamp.and_then(|stamp| {
                self.previous
                    .get(&key)
                    .filter(|(old, _)| *old == stamp)
                    .copied()
            }) {
                if !ids.iter().any(|id| id.object == object) {
                    self.observed.insert(key, (stamp, object));
                    return Ok(());
                }
            }
            stamp
        } else {
            None
        };
        let xml = read_descriptor_file(&mut file)?;
        let document = Document::parse(&xml)?;
        let owner = metadata_owner(&document, kind.as_str())?;
        if let DebugModuleQuery::Runtime(ids) = self.query {
            let object: uuid::Uuid = owner
                .attribute("uuid")
                .ok_or("missing metadata UUID")?
                .parse()?;
            if let Some(stamp) = stamp.filter(|s| {
                s.settled(
                    SystemTime::now(),
                    super::v13_large_configuration::SETTLE_WINDOW,
                )
            }) {
                if file.stamp().ok() == Some(stamp) {
                    self.observed.insert(key, (stamp, object));
                }
            }
            if !ids.iter().any(|id| id.object == object) || !self.module_exists(metadata)? {
                return Ok(());
            }
        }
        self.check_name(owner, name)?;
        self.add(metadata, role, owner)
    }

    fn open_descriptor(&mut self, path: &Path) -> Result<ReadFile> {
        if !matches!(self.query, DebugModuleQuery::Runtime(_)) {
            return Ok(self.root.open_relative_regular_nofollow(path)?);
        }
        let parent = path.parent().ok_or("descriptor has no parent")?;
        let leaf = Path::new(path.file_name().ok_or("descriptor has no filename")?);
        let directory = match self.directories.entry(parent.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut directory = self.root.clone();
                for component in parent.components() {
                    let std::path::Component::Normal(name) = component else {
                        return Err(
                            "debug descriptor parent must stay inside its source set".into()
                        );
                    };
                    directory = directory.retain_directory_child(name)?;
                }
                entry.insert(directory)
            }
        };
        Ok(directory.open_relative_regular_nofollow(leaf)?)
    }

    fn module_exists(&self, metadata: &str) -> Result<bool> {
        let relative = platform_xml_module_relative(&MetadataAddress::parse_logical(metadata)?)?;
        Ok(self.root.relative_file_stamp(&relative)?.is_some())
    }

    fn check_name(&self, owner: Node<'_, '_>, name: &str) -> Result<()> {
        if property(owner, "Name") != Some(name) {
            return Err(format!("wrong metadata descriptor for {name} in {}", self.set).into());
        }
        Ok(())
    }

    fn add(
        &mut self,
        metadata: &str,
        role: PlatformXmlModuleRole,
        owner: Node<'_, '_>,
    ) -> Result<()> {
        let property = role_property(role).ok_or("unsupported debug module role")?;
        let logical = logical_module_from_metadata(metadata)
            .ok_or("module metadata has no logical address")?;
        let source = DebugModuleSource {
            address: QualifiedAddress::parse(&format!("{}:{logical}", self.set))?,
            runtime_id: DebugModuleId {
                extension: self.extension.map(str::to_owned),
                object: owner
                    .attribute("uuid")
                    .ok_or("missing metadata UUID")?
                    .parse()?,
                property,
            },
        };
        let matches = match self.query {
            DebugModuleQuery::Address(expected) => source.address.to_string() == expected,
            DebugModuleQuery::Frame { id, .. } => {
                source.runtime_id == *id && self.address == Some(&source.address)
            }
            DebugModuleQuery::Runtime(expected) => expected.contains(&source.runtime_id),
        };
        if matches {
            self.modules.push(source);
        }
        Ok(())
    }
}

fn runtime_ids(query: DebugModuleQuery<'_>) -> Option<&[DebugModuleId]> {
    match query {
        DebugModuleQuery::Address(_) => None,
        DebugModuleQuery::Frame { id, .. } => Some(std::slice::from_ref(id)),
        DebugModuleQuery::Runtime(ids) => Some(ids),
    }
}

/// Property UUID in the RDBG module identity distinguishes these four module routes.
fn role_property(role: PlatformXmlModuleRole) -> Option<uuid::Uuid> {
    Some(match role {
        PlatformXmlModuleRole::Module => COMMON_MODULE_PROPERTY_ID,
        PlatformXmlModuleRole::ManagedApplicationModule => {
            uuid::Uuid::from_u128(0xd22e852a_cf8a_4f77_8ccb_3548e7792bea)
        }
        PlatformXmlModuleRole::ObjectModule => OBJECT_MODULE_PROPERTY_ID,
        PlatformXmlModuleRole::FormModule => {
            uuid::Uuid::from_u128(0x32e087ab_1491_49b6_aba7_43571b41ac2b)
        }
        _ => return None,
    })
}

fn descriptor_path(metadata: &str) -> Result<std::path::PathBuf> {
    Ok(metadata_descriptor_relative(
        &MetadataAddress::parse_logical(metadata)?,
        SourceSetKind::Configuration,
    )?)
}

fn descriptor(root: &SourceDirectory, path: &Path) -> Result<String> {
    read_descriptor_file(&mut root.open_relative_regular_nofollow(path)?)
}

fn read_descriptor_file(file: &mut ReadFile) -> Result<String> {
    // resource-limit: dap.saved-descriptor-bytes
    // class: step
    // unit: bytes
    // scope: file
    // value: 16777216
    // resource: Размер одного XML-дескриптора, читаемого целиком для привязки модуля DAP.
    // status: unproven
    // origin: 8e72f5805
    // problem: неизвестно
    // on-reach: Workspace отказывает в чтении дескриптора, attach возвращает ошибку.
    // issue: #1119
    const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024 * 1024;
    let mut contents = Vec::new();
    file.take(MAX_DESCRIPTOR_BYTES as u64 + 1)
        .read_to_end(&mut contents)?;
    if contents.len() > MAX_DESCRIPTOR_BYTES {
        return Err("debug module descriptor exceeds the byte limit".into());
    }
    Ok(String::from_utf8(contents)?)
}

fn metadata_owner<'a, 'input>(doc: &'a Document<'input>, kind: &str) -> Result<Node<'a, 'input>> {
    let root = doc.root_element();
    if !root.has_tag_name("MetaDataObject") {
        return Err("missing MetaDataObject root".into());
    }
    root.first_element_child()
        .filter(|owner| owner.has_tag_name(kind))
        .ok_or_else(|| format!("missing {kind} metadata owner").into())
}

fn property<'a>(owner: Node<'a, '_>, name: &str) -> Option<&'a str> {
    owner
        .children()
        .find(|node| node.has_tag_name("Properties"))?
        .children()
        .find(|node| node.has_tag_name(name))?
        .text()
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_source_contract::debug_source::DebugFrameModule;

    #[test]
    fn debug_attach_skips_a_declared_source_root_that_is_a_file() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        std::fs::write(root.join("broken"), "").unwrap();
        std::fs::create_dir(root.join("ext")).unwrap();
        std::fs::write(root.join("ext/Configuration.xml"), "").unwrap();
        for broken in ["broken", "broken/nested"] {
            std::fs::write(
                root.join("v8project.yaml"),
                format!("source-set:\n  - {{name: main, type: CONFIGURATION, path: {broken}}}\n  - {{name: ext, type: EXTENSION, path: ext}}\n"),
            )
            .unwrap();
            assert!(workspace_debug_resolver(root, None).is_ok());
            assert!(workspace_debug_resolver(root, Some("main")).is_err());
            assert!(workspace_debug_resolver(root, Some("ext")).is_ok());
        }
    }

    #[test]
    fn debug_attach_skips_a_linked_source_root_in_declared_and_autodetected_layouts() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("Configuration.xml"), "").unwrap();
        std::fs::create_dir_all(root.join("src/extensions/ext")).unwrap();
        std::fs::write(root.join("src/extensions/ext/Configuration.xml"), "").unwrap();
        if !super::super::test_fixtures::dir_link_or_skip(outside.path(), root.join("src/cf")) {
            return;
        }
        let assert_only_extension = || {
            assert!(workspace_debug_resolver(root, None).is_ok());
            assert!(workspace_debug_resolver(root, Some("main")).is_err());
            assert!(workspace_debug_resolver(root, Some("ext")).is_ok());
        };
        std::fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - {name: main, type: CONFIGURATION, path: src/cf}\n  - {name: ext, type: EXTENSION, path: src/extensions/ext}\n",
        )
        .unwrap();
        assert_only_extension();
        std::fs::remove_file(root.join("v8project.yaml")).unwrap();
        assert_only_extension();
    }

    #[test]
    fn debug_attach_requires_a_regular_file_for_edt_format_evidence() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        std::fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - {name: main, type: CONFIGURATION, path: src}\n",
        )
        .unwrap();
        let src = root.join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("Configuration.xml"), "").unwrap();
        let marker = src.join(".project");
        std::fs::create_dir(&marker).unwrap();
        assert!(workspace_debug_resolver(root, Some("main")).is_ok());
        std::fs::remove_dir(&marker).unwrap();
        std::fs::write(marker, "").unwrap();
        assert!(workspace_debug_resolver(root, Some("main")).is_err());
    }

    #[test]
    fn debug_modules_use_registered_object_and_form_descriptors() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        for (address, object, property) in [
            (
                "main:DataProcessor.Processor.Module.Object",
                "f3317000-0000-4000-8000-000000000003",
                "a637f77f-3840-441d-a1c3-699c8c5cb7e0",
            ),
            (
                "main:DataProcessor.Processor.Form.Main.Module.Form",
                "f3317000-0000-4000-8000-000000000004",
                "32e087ab-1491-49b6-aba7-43571b41ac2b",
            ),
            (
                "main:CommonForm.Main.Module.Form",
                "f3317000-0000-4000-8000-000000000005",
                "32e087ab-1491-49b6-aba7-43571b41ac2b",
            ),
        ] {
            let module = workspace_debug_modules(&workspace, DebugModuleQuery::Address(address))
                .unwrap()
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("missing debug module {address}"));
            assert_eq!(module.runtime_id.object.to_string(), object);
            assert_eq!(module.runtime_id.property.to_string(), property);
        }
        for missing in [
            "main:DataProcessor.Orphan.Module.Object",
            "main:CommonForm.Orphan.Module.Form",
            "main:DataProcessor.Orphan.Form.Main.Module.Form",
            "main:DataProcessor.Processor.Form.Ghost.Module.Form",
        ] {
            assert!(
                workspace_debug_modules(&workspace, DebugModuleQuery::Address(missing))
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn addressed_breakpoint_reads_current_descriptor_without_opening_other_owners() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let src = workspace.path().join("src");
        std::fs::write(src.join("DataProcessors/Processor.xml"), "broken XML").unwrap();
        let address = "main:CommonModule.Probe";
        let resolve = || {
            workspace_debug_modules(workspace.path(), DebugModuleQuery::Address(address)).unwrap()
        };
        let before = resolve().pop().unwrap();
        let descriptor = src.join("CommonModules/Probe.xml");
        let xml = std::fs::read_to_string(&descriptor).unwrap();
        let changed_id = "9ef29706-e9aa-43a3-8d32-a67074059840";
        std::fs::write(
            &descriptor,
            xml.replace(&before.runtime_id.object.to_string(), changed_id),
        )
        .unwrap();
        let after = resolve().pop().unwrap();
        assert_eq!(after.runtime_id.object.to_string(), changed_id);
        assert_eq!(after.address, before.address);
        std::fs::remove_file(src.join("CommonModules/Probe/Ext/Module.bsl")).unwrap();
        assert!(resolve().is_empty());
    }

    #[test]
    fn named_processor_frame_verifies_the_registered_object_and_its_saved_bsl() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let src = workspace.path().join("src");
        std::fs::write(src.join("Documents/Invoice.xml"), "unrelated broken object").unwrap();
        let mut resolve = workspace_debug_resolver(workspace.path(), None).unwrap();
        let id = DebugModuleId {
            extension: None,
            object: uuid::Uuid::from_u128(0xf3317000_0000_4000_8000_000000000003),
            property: OBJECT_MODULE_PROPERTY_ID,
        };
        let mut named = |name: &str, candidate: &DebugModuleId| {
            let module = DebugFrameModule::from_presentation(name, &id).unwrap();
            resolve(DebugModuleQuery::Frame {
                module: &module,
                id: candidate,
            })
            .unwrap()
            .modules
        };
        let frame = "Обработка.Processor.МодульОбъекта.Выполнить()";
        let found = named(frame, &id);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].address.to_string(),
            "main:DataProcessor.Processor.Module.Object"
        );
        assert_eq!(found[0].runtime_id, id);
        for wrong in [
            DebugModuleId {
                object: uuid::Uuid::nil(),
                ..id.clone()
            },
            DebugModuleId {
                property: COMMON_MODULE_PROPERTY_ID,
                ..id.clone()
            },
            DebugModuleId {
                extension: Some("Another".into()),
                ..id.clone()
            },
        ] {
            assert!(named(frame, &wrong).is_empty());
        }
        // Orphan has the same UUID and a saved object module, but no registration.
        assert!(named("DataProcessor.Orphan.ObjectModule.Run()", &id).is_empty());
        std::fs::remove_file(src.join("DataProcessors/Processor/Ext/ObjectModule.bsl")).unwrap();
        assert!(named(frame, &id).is_empty());
    }

    #[test]
    fn named_frame_checks_only_the_registered_form_and_its_full_id() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let src = workspace.path().join("src");
        std::fs::write(src.join("CommonForms/Main.xml"), "unrelated broken form").unwrap();
        let id = DebugModuleId {
            extension: None,
            object: uuid::Uuid::from_u128(0xf3317000_0000_4000_8000_000000000004),
            property: role_property(PlatformXmlModuleRole::FormModule).unwrap(),
        };
        let mut resolve = workspace_debug_resolver(workspace.path(), None).unwrap();
        let mut named = |name: &str, candidate: &DebugModuleId| {
            let module = DebugFrameModule::from_presentation(name, candidate).unwrap();
            resolve(DebugModuleQuery::Frame {
                module: &module,
                id: candidate,
            })
            .unwrap()
            .modules
        };
        let frame = "Обработка.Processor.Форма.Main.Форма.ПриСозданииНаСервере()";
        let found = named(frame, &id);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].address.to_string(),
            "main:DataProcessor.Processor.Form.Main.Module.Form"
        );
        assert_eq!(found[0].runtime_id, id);
        for wrong in [
            DebugModuleId {
                object: uuid::Uuid::nil(),
                ..id.clone()
            },
            DebugModuleId {
                property: role_property(PlatformXmlModuleRole::ObjectModule).unwrap(),
                ..id.clone()
            },
            DebugModuleId {
                extension: Some("Other".into()),
                ..id.clone()
            },
        ] {
            assert!(named(frame, &wrong).is_empty());
        }
        // Ghost has a saved form in the fixture, but the owner did not register it.
        assert!(named("Обработка.Processor.Форма.Ghost.Форма.Открыть()", &id).is_empty());
        std::fs::remove_file(src.join("DataProcessors/Processor/Forms/Main/Ext/Form/Module.bsl"))
            .unwrap();
        assert!(named(frame, &id).is_empty());
    }

    #[test]
    fn named_frame_retains_each_matching_source_set_and_checks_the_extension_name() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources/src");
        let workspace = tempfile::tempdir().unwrap();
        for path in ["base", "copy", "extension"] {
            crate::infrastructure::test_fixtures::copy_tree(&fixture, &workspace.path().join(path));
        }
        std::fs::write(workspace.path().join("v8project.yaml"), "source-set:\n  - {name: main, type: CONFIGURATION, path: base}\n  - {name: other, type: CONFIGURATION, path: copy}\n  - {name: ext, type: EXTENSION, path: extension}\n").unwrap();
        let mut resolve = workspace_debug_resolver(workspace.path(), None).unwrap();
        let mut id = DebugModuleId {
            extension: None,
            object: uuid::Uuid::from_u128(0xf3317000_0000_4000_8000_000000000004),
            property: role_property(PlatformXmlModuleRole::FormModule).unwrap(),
        };
        let module = DebugFrameModule::from_presentation(
            "DataProcessor.Processor.Form.Main.Form.Open()",
            &id,
        )
        .unwrap();
        let mut source_sets = |id: &DebugModuleId| {
            let mut sets: Vec<_> = resolve(DebugModuleQuery::Frame {
                module: &module,
                id,
            })
            .unwrap()
            .modules
            .into_iter()
            .map(|source| source.address.source_set().to_owned())
            .collect();
            sets.sort();
            sets
        };
        assert_eq!(source_sets(&id), ["main", "other"]);
        id.extension = Some("ResourceResolve".into());
        assert_eq!(source_sets(&id), ["ext"]);
        id.extension = Some("ext".into());
        assert!(source_sets(&id).is_empty());
    }

    #[test]
    fn address_lookup_reports_a_changed_source_set_list_before_another_reverse_search() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let address = "main:CommonModule.Probe";
        let mut resolve = workspace_debug_resolver(workspace.path(), None).unwrap();
        let original = resolve(DebugModuleQuery::Address(address)).unwrap();
        let id = &original.modules[0].runtime_id;
        let before = resolve(DebugModuleQuery::Runtime(std::slice::from_ref(id))).unwrap();
        assert_eq!(before.modules.len(), 1);
        crate::infrastructure::test_fixtures::copy_tree(
            &workspace.path().join("src"),
            &workspace.path().join("second"),
        );
        std::fs::write(workspace.path().join("v8project.yaml"), "source-set:\n  - {name: main, type: CONFIGURATION, path: src}\n  - {name: copy, type: CONFIGURATION, path: second}\n").unwrap();
        let checked = resolve(DebugModuleQuery::Address(address)).unwrap();
        assert_eq!(&checked.modules[0].runtime_id, id);
        assert_ne!(checked.source_sets_revision, before.source_sets_revision);
        let after = resolve(DebugModuleQuery::Runtime(std::slice::from_ref(id))).unwrap();
        assert_eq!(after.source_sets_revision, checked.source_sets_revision);
        assert_eq!(after.modules.len(), 2);
    }

    #[test]
    fn reverse_query_detects_duplicate_module_created_after_attach() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-resolve-resources");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let src = workspace.path().join("src");
        let id = workspace_debug_modules(
            workspace.path(),
            DebugModuleQuery::Address("main:CommonModule.Probe"),
        )
        .unwrap()
        .pop()
        .unwrap()
        .runtime_id;
        let matches = || {
            workspace_debug_modules(
                workspace.path(),
                DebugModuleQuery::Runtime(std::slice::from_ref(&id)),
            )
            .unwrap()
        };
        assert_eq!(matches().len(), 1);
        let config = src.join("Configuration.xml");
        let xml = std::fs::read_to_string(&config).unwrap();
        std::fs::write(
            config,
            xml.replace(
                "</ChildObjects>",
                "<CommonModule>Duplicate</CommonModule></ChildObjects>",
            ),
        )
        .unwrap();
        let module_dir = src.join("CommonModules");
        crate::infrastructure::test_fixtures::copy_tree(
            &module_dir.join("Probe"),
            &module_dir.join("Duplicate"),
        );
        let original = std::fs::read_to_string(module_dir.join("Probe.xml")).unwrap();
        std::fs::write(
            module_dir.join("Duplicate.xml"),
            original.replace("Probe", "Duplicate"),
        )
        .unwrap();
        // A descriptor replaced between stops must not reuse its previously observed UUID.
        let copy = module_dir.join("Duplicate.xml");
        let other = uuid::Uuid::from_u128(0x199e14fb_c4f5_4189_bea0_aa40749dff10);
        let duplicate = std::fs::read_to_string(&copy).unwrap();
        std::fs::write(
            &copy,
            duplicate.replace(&id.object.to_string(), &other.to_string()),
        )
        .unwrap();
        let source_root = unica_workspace::paths::normalize_path_identity(&src).unwrap();
        let root = open_source_directory(&source_root).unwrap();
        let relative = Path::new("CommonModules/Duplicate.xml");
        let key = root.path().join(relative);
        let stamp = root.relative_file_stamp(relative).unwrap().unwrap();
        let prior = HashMap::from([(key.clone(), (stamp, other))]);
        let mut observed = HashMap::new();
        let query = DebugModuleQuery::Runtime(std::slice::from_ref(&id));
        assert_eq!(
            resolve_modules(workspace.path(), query, &prior, &mut observed)
                .unwrap()
                .0
                .len(),
            1
        );
        assert_eq!(observed.get(&key), Some(&(stamp, other)));
        let fresh = module_dir.join("replacement.xml");
        std::fs::write(&fresh, duplicate).unwrap();
        std::fs::rename(fresh, &copy).unwrap();
        assert_ne!(root.relative_file_stamp(relative).unwrap(), Some(stamp));
        let mut addresses: Vec<_> =
            resolve_modules(workspace.path(), query, &prior, &mut HashMap::new())
                .unwrap()
                .0
                .into_iter()
                .map(|module| module.address.to_string())
                .collect();
        addresses.sort();
        assert_eq!(
            addresses,
            ["main:CommonModule.Duplicate", "main:CommonModule.Probe"]
        );
        std::fs::remove_file(module_dir.join("Duplicate/Ext/Module.bsl")).unwrap();
        std::fs::write(module_dir.join("Duplicate.xml"), "no saved module").unwrap();
        assert_eq!(matches().len(), 1);
    }

    #[test]
    fn named_common_module_frame_checks_only_the_registered_module_in_the_matching_extension() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-borrowed");
        let workspace = tempfile::tempdir().unwrap();
        crate::infrastructure::test_fixtures::copy_tree(&fixture, workspace.path());
        let mut resolve = workspace_debug_resolver(workspace.path(), None).unwrap();
        for (set, root, frame) in [
            ("main", "cf", "КешЗначений.Получить()"),
            (
                "ext",
                "ext",
                "ПриёмочноеРасширение ОбщийМодуль.КешЗначений.Модуль.Получить()",
            ),
        ] {
            let address = format!("{set}:CommonModule.КешЗначений");
            let id = resolve(DebugModuleQuery::Address(&address))
                .unwrap()
                .modules
                .pop()
                .unwrap()
                .runtime_id;
            let candidate = DebugFrameModule::from_presentation(frame, &id).unwrap();
            let src = workspace.path().join("src").join(root);
            std::fs::write(
                src.join("CommonModules/СерверныеФункции.xml"),
                "unrelated broken module",
            )
            .unwrap();
            let mut find = |id: &DebugModuleId| {
                resolve(DebugModuleQuery::Frame {
                    module: &candidate,
                    id,
                })
                .unwrap()
                .modules
            };
            let found = find(&id);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].address.to_string(), address);
            assert_eq!(found[0].runtime_id, id);
            assert!(find(&DebugModuleId {
                object: uuid::Uuid::nil(),
                ..id.clone()
            })
            .is_empty());
            assert!(find(&DebugModuleId {
                property: role_property(PlatformXmlModuleRole::FormModule).unwrap(),
                ..id.clone()
            })
            .is_empty());
            let bsl = src.join("CommonModules/КешЗначений/Ext/Module.bsl");
            std::fs::remove_file(bsl).unwrap();
            assert!(find(&id).is_empty());
        }
    }

    #[test]
    fn workspace_distinguishes_borrowed_module_from_its_base() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-borrowed");
        for (address, extension, uuid) in [
            (
                "main:CommonModule.КешЗначений",
                None,
                "3f6c2a10-5b7e-4c1d-9e2f-0a1b2c3d4e51",
            ),
            (
                "ext:CommonModule.КешЗначений",
                Some("ПриёмочноеРасширение"),
                "6d1e0c2b-8a4f-4b3e-a5c6-7d8e9f0a1b21",
            ),
        ] {
            let module = workspace_debug_modules(&workspace, DebugModuleQuery::Address(address))
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            let reverse = workspace_debug_modules(
                &workspace,
                DebugModuleQuery::Runtime(std::slice::from_ref(&module.runtime_id)),
            )
            .unwrap();
            assert_eq!(reverse.len(), 1);
            assert_eq!(reverse[0].address.to_string(), address);
            assert_eq!(module.runtime_id.extension.as_deref(), extension);
            assert_eq!(module.runtime_id.object.to_string(), uuid);
            assert_eq!(
                module.runtime_id.property.to_string(),
                "d5963243-262e-4398-b4d7-fb16d06484f6"
            );
        }
    }
}
