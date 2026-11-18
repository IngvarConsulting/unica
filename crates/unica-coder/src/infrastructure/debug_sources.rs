//! Workspace-backed resolution of the saved modules supported by DAP attach.

use std::path::Path;

use roxmltree::{Document, Node};
use unica_address::address::QualifiedAddress;
use unica_source_contract::{
    debug_source::{DebugModuleId, DebugModuleSource},
    read::SourceDirectory,
    SourceFormat, SourceSetKind, SourceSetState,
};
use unica_source_designer::{
    module_layout::{
        logical_module_from_metadata, platform_xml_module_identity, PlatformXmlModuleRole,
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

    add_module(
        root,
        set,
        extension.as_deref(),
        Path::new("Ext/ManagedApplicationModule.bsl"),
        owner,
        modules,
    )?;
    for entry in cf_edit_child_object_entries(&configuration)? {
        if entry.type_name != "CommonModule" {
            continue;
        }
        let target = unica_address::source_target::MetadataAddress::parse_logical(&format!(
            "CommonModule.{}.Module",
            entry.object_name
        ))?;
        let module_path =
            unica_source_designer::module_layout::platform_xml_module_relative(&target)?;
        if root.relative_file_stamp(&module_path)?.is_none() {
            continue;
        }
        let identity = platform_xml_module_identity(&module_path)?;
        let path = identity
            .descriptors
            .last()
            .ok_or("missing common module descriptor")?;
        let xml = descriptor(root, path)?;
        let document = Document::parse(&xml)?;
        let owner = metadata_owner(&document, "CommonModule")?;
        if property(owner, "Name") != Some(entry.object_name.as_str()) {
            return Err(format!(
                "wrong metadata descriptor for {} in {set}",
                entry.object_name
            )
            .into());
        }
        add_module(
            root,
            set,
            extension.as_deref(),
            &module_path,
            owner,
            modules,
        )?;
    }
    Ok(())
}

fn add_module(
    root: &SourceDirectory,
    set: &str,
    extension: Option<&str>,
    path: &Path,
    owner: Node<'_, '_>,
    modules: &mut Vec<DebugModuleSource>,
) -> Result<()> {
    if root.relative_file_stamp(path)?.is_none() {
        return Ok(());
    }
    let identity = platform_xml_module_identity(path)?;
    let property = match identity.role {
        PlatformXmlModuleRole::Module => "d5963243-262e-4398-b4d7-fb16d06484f6",
        PlatformXmlModuleRole::ManagedApplicationModule => "d22e852a-cf8a-4f77-8ccb-3548e7792bea",
        _ => return Err("unsupported debug module role".into()),
    };
    let logical = logical_module_from_metadata(identity.address.as_str())
        .ok_or("module metadata has no logical address")?;
    modules.push(DebugModuleSource {
        address: QualifiedAddress::parse(&format!("{set}:{logical}"))?,
        runtime_id: DebugModuleId {
            extension: extension.map(str::to_owned),
            object: owner
                .attribute("uuid")
                .ok_or("missing metadata UUID")?
                .parse()?,
            property: property.parse()?,
        },
    });
    Ok(())
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
