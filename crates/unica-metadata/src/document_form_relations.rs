//! Declared form assignments of a Document, independent of effective UI selection.
use serde::Serialize;
use unica_address::source_target::MetadataAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DocumentFormRole {
    DefaultObjectForm,
    DefaultListForm,
    DefaultChoiceForm,
    AuxiliaryObjectForm,
    AuxiliaryListForm,
    AuxiliaryChoiceForm,
}

impl DocumentFormRole {
    pub const ALL: [Self; 6] = [
        Self::DefaultObjectForm,
        Self::DefaultListForm,
        Self::DefaultChoiceForm,
        Self::AuxiliaryObjectForm,
        Self::AuxiliaryListForm,
        Self::AuxiliaryChoiceForm,
    ];

    pub const fn property_name(self) -> &'static str {
        match self {
            Self::DefaultObjectForm => "DefaultObjectForm",
            Self::DefaultListForm => "DefaultListForm",
            Self::DefaultChoiceForm => "DefaultChoiceForm",
            Self::AuxiliaryObjectForm => "AuxiliaryObjectForm",
            Self::AuxiliaryListForm => "AuxiliaryListForm",
            Self::AuxiliaryChoiceForm => "AuxiliaryChoiceForm",
        }
    }

    pub const fn relation_name(self) -> &'static str {
        match self {
            Self::DefaultObjectForm => "defaultObjectForm",
            Self::DefaultListForm => "defaultListForm",
            Self::DefaultChoiceForm => "defaultChoiceForm",
            Self::AuxiliaryObjectForm => "auxiliaryObjectForm",
            Self::AuxiliaryListForm => "auxiliaryListForm",
            Self::AuxiliaryChoiceForm => "auxiliaryChoiceForm",
        }
    }

    pub fn from_property_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|role| role.property_name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentFormReference {
    pub role: DocumentFormRole,
    /// Missing observation, known empty assignment, or a declared target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Option<MetadataAddress>>,
}
