//! Closed assignments of an existing SettingsStorage's own forms.
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFormSlot {
    DefaultSave,
    DefaultLoad,
    AuxiliarySave,
    AuxiliaryLoad,
}
impl StorageFormSlot {
    pub const ALL: [Self; 4] = [
        Self::DefaultSave,
        Self::DefaultLoad,
        Self::AuxiliarySave,
        Self::AuxiliaryLoad,
    ];
    pub const fn property(self) -> &'static str {
        match self {
            Self::DefaultSave => "DefaultSaveForm",
            Self::DefaultLoad => "DefaultLoadForm",
            Self::AuxiliarySave => "AuxiliarySaveForm",
            Self::AuxiliaryLoad => "AuxiliaryLoadForm",
        }
    }
    pub const fn key(self) -> &'static str {
        match self {
            Self::DefaultSave => "defaultSaveForm",
            Self::DefaultLoad => "defaultLoadForm",
            Self::AuxiliarySave => "auxiliarySaveForm",
            Self::AuxiliaryLoad => "auxiliaryLoadForm",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.key() == key)
    }
}
#[derive(Debug, Clone)]
pub struct StorageFormAssignment {
    pub slot: StorageFormSlot,
    pub form: Option<MetadataAddress>,
}
impl StorageFormAssignment {
    pub fn parse(
        slot: StorageFormSlot,
        raw: &str,
        owner: &MetadataAddress,
        source_set: &str,
    ) -> Result<Self, String> {
        if raw.is_empty() {
            return Ok(Self { slot, form: None });
        }
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("a storage form must have a name".into());
        }
        let path = if raw.contains('.') || raw.contains(':') {
            raw.to_string()
        } else {
            format!("{}.Form.{raw}", owner.as_str())
        };
        let at =
            QualifiedAddress::resolve_input(&path, &[source_set]).map_err(|e| e.to_string())?;
        if at.source_set() != source_set {
            return Err("a storage form must belong to the same source set as its owner".into());
        }
        if !matches!(at.segments(),[root,form] if root.kind()==NodeKind::SettingsStorage && root.name().is_some() && form.kind()==NodeKind::Form && form.name().is_some())
            || at.prefix(1).logical_path() != owner.as_str()
        {
            return Err("choose an existing form of this SettingsStorage owner".into());
        }
        let form = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &at.logical_path())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            slot,
            form: Some(form),
        })
    }
}
