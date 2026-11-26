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
    debug_source::{DebugModuleId, DebugModuleQuery, DebugModuleSource, ResolveDebugModules},
    read::{FileStamp, ReadFile, SourceDirectory},
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

pub fn workspace_debug_resolver() -> ResolveDebugModules {
    let mut previous = OwnerIds::new();
    Box::new(move |workspace, query| {
        let mut observed = OwnerIds::new();
        let found = resolve_modules(workspace, query, &previous, &mut observed)?;
        if matches!(query, DebugModuleQuery::Runtime(_)) {
            previous = observed;
        }
        Ok(found)
    })
}

#[cfg(test)]
fn workspace_debug_modules(
    workspace: &Path,
    query: DebugModuleQuery<'_>,
) -> Result<Vec<DebugModuleSource>> {
    workspace_debug_resolver()(workspace, query)
}

fn resolve_modules(
    workspace: &Path,
    query: DebugModuleQuery<'_>,
    previous: &OwnerIds,
    observed: &mut OwnerIds,
) -> Result<Vec<DebugModuleSource>> {
    let address = match query {
        DebugModuleQuery::Address(text) => Some(QualifiedAddress::parse(text)?),
        _ => None,
    };
    let requested_set = match query {
        DebugModuleQuery::FirstSaved { source_set } => source_set,
        _ => address.as_ref().map(QualifiedAddress::source_set),
    };
    if !workspace.is_absolute() {
        return Err("rootProject must be an absolute Unica workspace path".into());
    }
    let admission =
        super::source_workspace::discover_project_source_admission(workspace, &mut || Ok(()))?;
    let map = admission.map();
    let canonical = Path::new(&map.workspace_root);
    let mut modules = Vec::new();
    for set in &map.source_sets {
        if requested_set.is_some_and(|name| name != set.name)
            || set.source_format != SourceFormat::PlatformXml
            || set.source_state != SourceSetState::Supported
            || !matches!(
                set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        {
            continue;
        }
        let path = lexical_contained_source_root(canonical, &set.path)?;
        reject_linked_source_root_route(canonical, &path)?;
        let root = open_source_directory(&path)?;
        let relative = path.strip_prefix(canonical)?;
        if admission.source_root_identity(relative) != Some(root.identity()) {
            return Err(format!("debug source set {} changed during discovery", set.name).into());
        }
        read_modules(
            &root,
            set,
            query,
            address.as_ref(),
            &mut modules,
            previous,
            observed,
        )?;
        root.validate_named_identity()?;
        if matches!(query, DebugModuleQuery::FirstSaved { .. }) && !modules.is_empty() {
            break;
        }
    }
    Ok(modules)
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

    if matches!(query, DebugModuleQuery::Runtime(ids) if !ids.iter().any(|id| id.extension == extension))
    {
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
    if index.first_found() || owner_filter.is_some_and(|segment| segment.kind() == NodeKind::Module)
    {
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
            if index.first_found() {
                break;
            }
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
    fn first_found(&self) -> bool {
        matches!(self.query, DebugModuleQuery::FirstSaved { .. }) && !self.modules.is_empty()
    }

    fn wants_role(&self, role: PlatformXmlModuleRole) -> bool {
        match self.query {
            DebugModuleQuery::Runtime(ids) => ids.iter().any(|id| {
                id.extension.as_deref() == self.extension
                    && Some(id.property) == role_property(role)
            }),
            _ => true,
        }
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
            && !self.first_found()
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
                if self.first_found() {
                    break;
                }
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
            DebugModuleQuery::FirstSaved { .. } => true,
            DebugModuleQuery::Address(expected) => source.address.to_string() == expected,
            DebugModuleQuery::Runtime(expected) => expected.contains(&source.runtime_id),
        };
        if matches {
            self.modules.push(source);
        }
        Ok(())
    }
}

/// Property UUID in the RDBG module identity distinguishes these four module routes.
fn role_property(role: PlatformXmlModuleRole) -> Option<uuid::Uuid> {
    Some(uuid::Uuid::from_u128(match role {
        PlatformXmlModuleRole::Module => 0xd5963243_262e_4398_b4d7_fb16d06484f6,
        PlatformXmlModuleRole::ManagedApplicationModule => 0xd22e852a_cf8a_4f77_8ccb_3548e7792bea,
        PlatformXmlModuleRole::ObjectModule => 0xa637f77f_3840_441d_a1c3_699c8c5cb7e0,
        PlatformXmlModuleRole::FormModule => 0x32e087ab_1491_49b6_aba7_43571b41ac2b,
        _ => return None,
    }))
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
        let root = open_source_directory(&src).unwrap();
        let relative = Path::new("CommonModules/Duplicate.xml");
        let prior = HashMap::from([(
            root.path().join(relative),
            (root.relative_file_stamp(relative).unwrap().unwrap(), other),
        )]);
        let query = DebugModuleQuery::Runtime(std::slice::from_ref(&id));
        assert_eq!(
            resolve_modules(workspace.path(), query, &prior, &mut HashMap::new())
                .unwrap()
                .len(),
            1
        );
        let fresh = module_dir.join("replacement.xml");
        std::fs::write(&fresh, duplicate).unwrap();
        std::fs::rename(fresh, &copy).unwrap();
        let mut addresses: Vec<_> =
            resolve_modules(workspace.path(), query, &prior, &mut HashMap::new())
                .unwrap()
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
