//! An external artifact's logical identity is independent of its export filename.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use unica_source_contract::SourceSetKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalArtifactBinding {
    kind: &'static str,
    name: String,
    descriptor: PathBuf,
    payload: PathBuf,
}

impl ExternalArtifactBinding {
    /// Identity has already been observed by the caller's descriptor reader.
    /// This value maps paths; it does not grant read or write authority.
    pub fn new(source_kind: SourceSetKind, name: &str, descriptor: &Path) -> Result<Self, String> {
        let kind = match source_kind {
            SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
            SourceSetKind::ExternalReport => "ExternalReport",
            _ => {
                return Err("external artifact mapping requires an external source set".to_string())
            }
        };
        if name.is_empty() || name.contains(['/', '\\', '.']) {
            return Err("external artifact has no valid logical owner name".to_string());
        }
        if !matches!(
            descriptor.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) || descriptor
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("xml"))
        {
            return Err("external artifact descriptor must be one immediate XML file".to_string());
        }
        let stem = descriptor
            .file_stem()
            .filter(|stem| !stem.is_empty())
            .ok_or("external artifact descriptor has no stem")?;
        Ok(Self {
            kind,
            name: name.to_string(),
            descriptor: descriptor.to_path_buf(),
            payload: PathBuf::from(stem),
        })
    }
    pub fn kind(&self) -> &'static str {
        self.kind
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn descriptor(&self) -> &Path {
        &self.descriptor
    }
    pub fn payload(&self) -> &Path {
        &self.payload
    }

    /// The canonical layout functions first describe a logical owner's path;
    /// only that exact first component is replaced by the proven physical root.
    pub fn physical_path(&self, canonical: &Path) -> Result<PathBuf, String> {
        let mut parts = canonical.components();
        let Some(Component::Normal(first)) = parts.next() else {
            return Err("external path has no owner component".to_string());
        };
        let remainder = parts.collect::<PathBuf>();
        if first == format!("{}.xml", self.name).as_str() && remainder.as_os_str().is_empty() {
            return Ok(self.descriptor.clone());
        }
        if first != self.name.as_str() {
            return Err("external path belongs to another logical owner".to_string());
        }
        Ok(self.payload.join(remainder))
    }
    pub fn logical_path(&self, physical: &Path) -> Option<PathBuf> {
        if physical == self.descriptor {
            return Some(PathBuf::from(format!("{}.xml", self.name)));
        }
        physical
            .strip_prefix(&self.payload)
            .ok()
            .map(|tail| PathBuf::from(&self.name).join(tail))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExternalArtifactInventory {
    bindings: BTreeMap<String, ExternalArtifactBinding>,
}
impl ExternalArtifactInventory {
    pub fn insert(&mut self, binding: ExternalArtifactBinding) -> Result<(), String> {
        if self.bindings.contains_key(binding.name()) {
            return Err(format!(
                "external owner `{}.{}` is ambiguous",
                binding.kind(),
                binding.name()
            ));
        }
        self.bindings.insert(binding.name().to_string(), binding);
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<&ExternalArtifactBinding> {
        self.bindings.get(name)
    }
    pub fn iter(&self) -> impl Iterator<Item = &ExternalArtifactBinding> {
        self.bindings.values()
    }
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}
