use std::{collections::HashMap, fs, path::Path};

use unica_address::address::QualifiedAddress;
use unica_source_designer::module_layout::{
    logical_module_from_metadata, platform_xml_module_identity, PlatformXmlModuleRole as Role,
};

use crate::Result;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ModuleId {
    pub object: uuid::Uuid,
    pub property: uuid::Uuid,
}

#[derive(Default)]
pub(crate) struct Modules {
    by_address: HashMap<String, ModuleId>,
    by_id: HashMap<ModuleId, String>,
}

impl Modules {
    pub fn read(root: &Path, source_set: &str) -> Result<Self> {
        QualifiedAddress::parse(&format!("{source_set}:Module.ManagedApplication"))?;
        let mut index = Self::default();
        index.add(
            root,
            source_set,
            Path::new("Ext/ManagedApplicationModule.bsl"),
        )?;
        let commons = root.join("CommonModules");
        if commons.is_dir() {
            for entry in fs::read_dir(commons)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let module = Path::new("CommonModules")
                        .join(entry.file_name())
                        .join("Ext/Module.bsl");
                    index.add(root, source_set, &module)?;
                }
            }
        }
        Ok(index)
    }

    fn add(&mut self, root: &Path, source_set: &str, path: &Path) -> Result<()> {
        if !root.join(path).is_file() {
            return Ok(());
        }
        let identity = platform_xml_module_identity(path)?;
        let property = match identity.role {
            Role::Module => "d5963243-262e-4398-b4d7-fb16d06484f6",
            Role::ManagedApplicationModule => "d22e852a-cf8a-4f77-8ccb-3548e7792bea",
            _ => return Err("unsupported debug module role".into()),
        };
        let logical = logical_module_from_metadata(identity.address.as_str())
            .ok_or("module metadata has no logical address")?;
        let descriptor = identity
            .descriptors
            .last()
            .ok_or("missing module descriptor")?;
        let xml = fs::read_to_string(root.join(descriptor))?;
        let parsed = roxmltree::Document::parse(&xml)?;
        let owner = parsed
            .root_element()
            .first_element_child()
            .ok_or("missing metadata owner")?;
        let id = ModuleId {
            object: owner
                .attribute("uuid")
                .ok_or("missing metadata UUID")?
                .parse()?,
            property: property.parse()?,
        };
        let address = format!("{source_set}:{logical}");
        QualifiedAddress::parse(&address)?;
        if self.by_id.insert(id.clone(), address.clone()).is_some() {
            return Err(format!("ambiguous debug module {address}").into());
        }
        self.by_address.insert(address, id);
        Ok(())
    }

    pub fn id(&self, address: &str) -> Result<ModuleId> {
        self.by_address
            .get(address)
            .cloned()
            .ok_or_else(|| format!("unknown debug module {address}").into())
    }

    pub fn address(&self, id: &ModuleId) -> Option<&str> {
        self.by_id.get(id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_identifiers_resolve_to_public_module_addresses() {
        let root = tempfile::tempdir().unwrap();
        let test_uuid = "829019ad-9973-4df8-bd04-63ec82ad0830";
        for (source, descriptor, element) in [
            (
                "Ext/ManagedApplicationModule.bsl",
                "Configuration.xml",
                "Configuration",
            ),
            (
                "CommonModules/Probe/Ext/Module.bsl",
                "CommonModules/Probe.xml",
                "CommonModule",
            ),
        ] {
            let module = root.path().join(source);
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(module, "").unwrap();
            fs::write(
                root.path().join(descriptor),
                format!("<MetaDataObject><{element} uuid=\"{test_uuid}\"/></MetaDataObject>"),
            )
            .unwrap();
        }

        let modules = Modules::read(root.path(), "main").unwrap();
        for (address, property) in [
            (
                "main:Module.ManagedApplication",
                "d22e852a-cf8a-4f77-8ccb-3548e7792bea",
            ),
            (
                "main:CommonModule.Probe",
                "d5963243-262e-4398-b4d7-fb16d06484f6",
            ),
        ] {
            let runtime = ModuleId {
                object: test_uuid.parse().unwrap(),
                property: property.parse().unwrap(),
            };
            assert_eq!(modules.id(address).unwrap(), runtime);
            assert_eq!(modules.address(&runtime), Some(address));
        }
    }
}
