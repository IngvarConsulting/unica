//! Choice settings of existing application fields, separate from root properties.
use crate::{MetaCollection, MetadataKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberChoiceProperty {
    FoldersAndItems,
    Form,
    HistoryOnInput,
    QuickChoice,
    CreateOnInput,
}

impl MemberChoiceProperty {
    pub const ALL: &'static [Self] = &[
        Self::FoldersAndItems,
        Self::Form,
        Self::HistoryOnInput,
        Self::QuickChoice,
        Self::CreateOnInput,
    ];
    pub const fn argument(self) -> &'static str {
        match self {
            Self::FoldersAndItems => "choiceFoldersAndItems",
            Self::Form => "choiceForm",
            Self::HistoryOnInput => "choiceHistoryOnInput",
            Self::QuickChoice => "quickChoice",
            Self::CreateOnInput => "createOnInput",
        }
    }
    pub const fn public_name(self) -> &'static str {
        match self {
            Self::FoldersAndItems => "ChoiceFoldersAndItems",
            Self::Form => "ChoiceForm",
            Self::HistoryOnInput => "ChoiceHistoryOnInput",
            Self::QuickChoice => "QuickChoice",
            Self::CreateOnInput => "CreateOnInput",
        }
    }
    pub const fn values(self) -> Option<&'static [&'static str]> {
        match self {
            Self::FoldersAndItems => Some(&["Items", "Folders", "FoldersAndItems"]),
            Self::HistoryOnInput => Some(&["Auto", "DontUse"]),
            Self::QuickChoice | Self::CreateOnInput => Some(&["Auto", "Use", "DontUse"]),
            Self::Form => None,
        }
    }
    pub fn accepts(self, value: &str) -> bool {
        self.values().is_none_or(|values| values.contains(&value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemberChoiceChanges(pub Vec<(MemberChoiceProperty, String)>);

/// This slice supports configuration Catalog/Document fields and independent
/// information-register fields. Numeric accumulation resources are outside it.
pub fn member_choice_site(
    owner: MetadataKind,
    collection: MetaCollection,
    in_section: bool,
) -> bool {
    match (owner, collection) {
        (MetadataKind::Catalog | MetadataKind::Document, MetaCollection::Attributes) => true,
        (
            MetadataKind::InformationRegister,
            MetaCollection::Attributes | MetaCollection::Dimensions | MetaCollection::Resources,
        ) => !in_section,
        _ => false,
    }
}
