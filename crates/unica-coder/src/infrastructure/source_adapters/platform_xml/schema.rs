#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObjectsVocabulary {
    None,
    ConfigurationTopLevel,
    Object,
    TabularSection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataClassRole {
    Configuration,
    TopLevelObject,
    Attribute,
    TabularSection,
    Form,
    Template,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataClassProfile {
    pub(crate) class_name: &'static str,
    pub(crate) role: MetadataClassRole,
    pub(crate) child_objects: ChildObjectsVocabulary,
}

pub(crate) const ROOT_STRUCTURAL_CHILDREN: &[&str] = &["Properties", "ChildObjects"];

macro_rules! platform_xml_schema_registry {
    (top_level: [$($top_level:literal),+ $(,)?]) => {
        pub(crate) const LEGACY_TOP_LEVEL_METADATA_CLASSES: &[&str] = &[$($top_level),+];

        pub(crate) const METADATA_CLASS_PROFILES: &[MetadataClassProfile] = &[
            MetadataClassProfile {
                class_name: "Configuration",
                role: MetadataClassRole::Configuration,
                child_objects: ChildObjectsVocabulary::ConfigurationTopLevel,
            },
            $(MetadataClassProfile {
                class_name: $top_level,
                role: MetadataClassRole::TopLevelObject,
                child_objects: ChildObjectsVocabulary::Object,
            },)+
            MetadataClassProfile {
                class_name: "Form",
                role: MetadataClassRole::Form,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "Template",
                role: MetadataClassRole::Template,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "Attribute",
                role: MetadataClassRole::Attribute,
                child_objects: ChildObjectsVocabulary::None,
            },
            MetadataClassProfile {
                class_name: "TabularSection",
                role: MetadataClassRole::TabularSection,
                child_objects: ChildObjectsVocabulary::TabularSection,
            },
            MetadataClassProfile {
                class_name: "Command",
                role: MetadataClassRole::Command,
                child_objects: ChildObjectsVocabulary::None,
            },
        ];
    };
}

platform_xml_schema_registry! {
    top_level: [
        "Language", "Subsystem", "StyleItem", "Style", "CommonPicture", "SessionParameter",
        "Role", "CommonTemplate", "FilterCriterion", "CommonModule", "Bot", "CommonAttribute",
        "ExchangePlan", "XDTOPackage", "WebService", "HTTPService", "WSReference",
        "EventSubscription", "ScheduledJob", "SettingsStorage", "FunctionalOption",
        "FunctionalOptionsParameter", "DefinedType", "CommonCommand", "CommandGroup", "Constant",
        "CommonForm", "Catalog", "Document", "DocumentNumerator", "Sequence", "DocumentJournal",
        "Enum", "Report", "DataProcessor", "InformationRegister", "AccumulationRegister",
        "ChartOfCharacteristicTypes", "ChartOfAccounts", "AccountingRegister",
        "ChartOfCalculationTypes", "CalculationRegister", "BusinessProcess", "Task",
        "IntegrationService"
    ]
}

pub(crate) fn metadata_class_profile(class_name: &str) -> Option<&'static MetadataClassProfile> {
    METADATA_CLASS_PROFILES
        .iter()
        .find(|profile| profile.class_name == class_name)
}

pub(crate) fn child_metadata_class_profile(
    owner: &MetadataClassProfile,
    child_class_name: &str,
) -> Option<&'static MetadataClassProfile> {
    let child = metadata_class_profile(child_class_name)?;
    let allowed = match owner.child_objects {
        ChildObjectsVocabulary::None => false,
        ChildObjectsVocabulary::ConfigurationTopLevel => {
            child.role == MetadataClassRole::TopLevelObject
        }
        ChildObjectsVocabulary::Object => matches!(
            child.role,
            MetadataClassRole::Attribute
                | MetadataClassRole::TabularSection
                | MetadataClassRole::Form
                | MetadataClassRole::Template
                | MetadataClassRole::Command
        ),
        ChildObjectsVocabulary::TabularSection => child.role == MetadataClassRole::Attribute,
    };
    allowed.then_some(child)
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::{
        metadata_kinds::{METADATA_KINDS, METADATA_KIND_TAGS},
        source_adapters::platform_xml::schema::LEGACY_TOP_LEVEL_METADATA_CLASSES,
    };

    #[test]
    fn legacy_metadata_kind_mapping_uses_the_shared_top_level_class_source() {
        assert_eq!(METADATA_KIND_TAGS, LEGACY_TOP_LEVEL_METADATA_CLASSES);
        assert_eq!(
            METADATA_KINDS.iter().map(|kind| kind.tag).collect::<Vec<_>>(),
            LEGACY_TOP_LEVEL_METADATA_CLASSES,
        );
    }
}
