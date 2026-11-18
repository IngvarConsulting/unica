//! Workspace-backed resolution of the saved modules supported by DAP attach.

use std::path::Path;

use roxmltree::{Document, Node};
use unica_address::{
    address::{NodeKind, QualifiedAddress},
    module_routes::{has_direct_module_route, has_nested_module_role, ModuleRole},
    source_target::MetadataAddress,
};
use unica_source_contract::{
    debug_source::{DebugModuleId, DebugModuleSource},
    read::SourceDirectory,
    SourceFormat, SourceSetKind, SourceSetState,
};
use unica_source_designer::{
    layout::metadata_descriptor_relative,
    module_layout::{
        logical_module_from_metadata, platform_xml_module_relative, PlatformXmlModuleRole,
    },
    registration::cf_edit_child_object_entries,
};
use unica_workspace::{
    discovery::{lexical_contained_source_root, reject_linked_source_root_route},
    read::open_source_directory,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn workspace_debug_modules(workspace: &Path) -> Result<Vec<DebugModuleSource>> {
    if !workspace.is_absolute() {
        return Err("rootProject must be an absolute Unica workspace path".into());
    }
    let admission =
        super::source_workspace::discover_project_source_admission(workspace, &mut || Ok(()))?;
    let map = admission.map();
    let canonical = Path::new(&map.workspace_root);
    let mut modules = Vec::new();
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
        reject_linked_source_root_route(canonical, &path)?;
        let root = open_source_directory(&path)?;
        let relative = path.strip_prefix(canonical)?;
        if admission.source_root_identity(relative) != Some(root.identity()) {
            return Err(format!("debug source set {} changed during discovery", set.name).into());
        }
        read_modules(&root, &set.name, set.kind, &mut modules)?;
        root.validate_named_identity()?;
    }
    if modules.is_empty() {
        return Err("workspace has no supported saved debug modules".into());
    }
    Ok(modules)
}

fn read_modules(
    root: &SourceDirectory,
    set: &str,
    kind: SourceSetKind,
    modules: &mut Vec<DebugModuleSource>,
) -> Result<()> {
    let configuration = descriptor(root, Path::new("Configuration.xml"))?;
    let document = Document::parse(&configuration)?;
    let owner = metadata_owner(&document, "Configuration")?;
    let extension = if kind == SourceSetKind::Extension {
        Some(
            property(owner, "Name")
                .ok_or("missing extension name")?
                .to_owned(),
        )
    } else {
        None
    };

    let mut index = SourceSetModules {
        root,
        set,
        extension: extension.as_deref(),
        modules,
    };
    if root
        .relative_file_stamp(Path::new("Ext/ManagedApplicationModule.bsl"))?
        .is_some()
    {
        index.add(
            "ManagedApplicationModule",
            PlatformXmlModuleRole::ManagedApplicationModule,
            owner,
        )?;
    }
    for entry in cf_edit_child_object_entries(&configuration)? {
        let Ok(kind) = NodeKind::parse(&entry.type_name) else {
            continue;
        };
        let name = &entry.object_name;
        let metadata = format!("{}.{name}", kind.as_str());
        match kind {
            NodeKind::CommonModule => {
                index.read(
                    &format!("{metadata}.Module"),
                    PlatformXmlModuleRole::Module,
                    kind,
                    name,
                )?;
            }
            NodeKind::CommonForm => {
                index.read(
                    &format!("{metadata}.FormModule"),
                    PlatformXmlModuleRole::FormModule,
                    kind,
                    name,
                )?;
            }
            _ if has_direct_module_route(kind, ModuleRole::Object)
                || has_nested_module_role(kind, ModuleRole::Form) =>
            {
                index.read_owner(&metadata, kind, name)?;
            }
            _ => {}
        }
    }
    Ok(())
}

struct SourceSetModules<'a> {
    root: &'a SourceDirectory,
    set: &'a str,
    extension: Option<&'a str>,
    modules: &'a mut Vec<DebugModuleSource>,
}

impl SourceSetModules<'_> {
    fn read_owner(&mut self, metadata: &str, kind: NodeKind, name: &str) -> Result<()> {
        let descriptor_path = descriptor_path(metadata)?;
        if self.root.relative_file_stamp(&descriptor_path)?.is_none() {
            return Ok(());
        }
        let xml = descriptor(self.root, &descriptor_path)?;
        let document = Document::parse(&xml)?;
        let owner = metadata_owner(&document, kind.as_str())?;
        self.check_name(owner, name)?;

        if has_direct_module_route(kind, ModuleRole::Object) {
            let target = format!("{metadata}.ObjectModule");
            if self.module_exists(&target)? {
                self.add(&target, PlatformXmlModuleRole::ObjectModule, owner)?;
            }
        }

        if has_nested_module_role(kind, ModuleRole::Form) {
            for form in owner
                .children()
                .filter(|node| node.has_tag_name("ChildObjects"))
                .flat_map(|children| children.children())
                .filter(|node| node.has_tag_name("Form"))
            {
                let form_name = form.text().ok_or("missing registered form name")?.trim();
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
        if !self.module_exists(metadata)? {
            return Ok(());
        }
        let owner_metadata = metadata
            .rsplit_once('.')
            .ok_or("debug module has no metadata owner")?
            .0;
        let xml = descriptor(self.root, &descriptor_path(owner_metadata)?)?;
        let document = Document::parse(&xml)?;
        let owner = metadata_owner(&document, kind.as_str())?;
        self.check_name(owner, name)?;
        self.add(metadata, role, owner)
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
        let property = match role {
            PlatformXmlModuleRole::Module => "d5963243-262e-4398-b4d7-fb16d06484f6",
            PlatformXmlModuleRole::ManagedApplicationModule => {
                "d22e852a-cf8a-4f77-8ccb-3548e7792bea"
            }
            PlatformXmlModuleRole::ObjectModule => "a637f77f-3840-441d-a1c3-699c8c5cb7e0",
            PlatformXmlModuleRole::FormModule => "32e087ab-1491-49b6-aba7-43571b41ac2b",
            _ => return Err("unsupported debug module role".into()),
        };
        let logical = logical_module_from_metadata(metadata)
            .ok_or("module metadata has no logical address")?;
        self.modules.push(DebugModuleSource {
            address: QualifiedAddress::parse(&format!("{}:{logical}", self.set))?,
            runtime_id: DebugModuleId {
                extension: self.extension.map(str::to_owned),
                object: owner
                    .attribute("uuid")
                    .ok_or("missing metadata UUID")?
                    .parse()?,
                property: property.parse()?,
            },
        });
        Ok(())
    }
}

fn descriptor_path(metadata: &str) -> Result<std::path::PathBuf> {
    Ok(metadata_descriptor_relative(
        &MetadataAddress::parse_logical(metadata)?,
        SourceSetKind::Configuration,
    )?)
}

fn descriptor(root: &SourceDirectory, path: &Path) -> Result<String> {
    const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024 * 1024;
    Ok(String::from_utf8(root.read_relative_regular_bounded(
        path,
        MAX_DESCRIPTOR_BYTES,
    )?)?)
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
        let modules = workspace_debug_modules(&workspace).unwrap();
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
            let module = modules
                .iter()
                .find(|module| module.address.to_string() == address)
                .unwrap_or_else(|| panic!("missing debug module {address}"));
            assert_eq!(module.runtime_id.object.to_string(), object);
            assert_eq!(module.runtime_id.property.to_string(), property);
        }
        assert!(modules.iter().all(|module| {
            let address = module.address.to_string();
            !address.contains(".Ghost.") && !address.contains(".Orphan.")
        }));
    }

    #[test]
    fn workspace_distinguishes_borrowed_module_from_its_base() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-borrowed");
        let modules = workspace_debug_modules(&workspace).unwrap();
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
            let module = modules
                .iter()
                .find(|module| module.address.to_string() == address)
                .unwrap();
            assert_eq!(module.runtime_id.extension.as_deref(), extension);
            assert_eq!(module.runtime_id.object.to_string(), uuid);
            assert_eq!(
                module.runtime_id.property.to_string(),
                "d5963243-262e-4398-b4d7-fb16d06484f6"
            );
        }
    }
}
