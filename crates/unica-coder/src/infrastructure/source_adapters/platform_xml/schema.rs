#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildObjectsVocabulary {
    ConfigurationTopLevel,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataClassProfile {
    pub(crate) class_name: &'static str,
    pub(crate) child_objects: ChildObjectsVocabulary,
}

pub(crate) const ROOT_STRUCTURAL_CHILDREN: &[&str] = &["Properties", "ChildObjects"];
pub(crate) const OBJECT_CHILD_OBJECTS: &[&str] =
    &["Attribute", "TabularSection", "Form", "Template", "Command"];
pub(crate) const TABULAR_SECTION_CHILDREN: &[&str] = &["Properties", "ChildObjects"];

macro_rules! platform_xml_schema_registry {
    (
        top_level: [$($top_level:literal),+ $(,)?],
        standalone: [$($standalone:literal),+ $(,)?]
    ) => {
        pub(crate) const LEGACY_TOP_LEVEL_METADATA_CLASSES: &[&str] = &[$($top_level),+];

        pub(crate) const METADATA_CLASS_PROFILES: &[MetadataClassProfile] = &[
            MetadataClassProfile {
                class_name: "Configuration",
                child_objects: ChildObjectsVocabulary::ConfigurationTopLevel,
            },
            $(MetadataClassProfile {
                class_name: $top_level,
                child_objects: ChildObjectsVocabulary::Object,
            },)+
            $(MetadataClassProfile {
                class_name: $standalone,
                child_objects: ChildObjectsVocabulary::Object,
            },)+
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
    ],
    standalone: ["Form", "Template", "Attribute", "TabularSection", "Command"]
}

pub(crate) fn metadata_class_profile(class_name: &str) -> Option<&'static MetadataClassProfile> {
    METADATA_CLASS_PROFILES
        .iter()
        .find(|profile| profile.class_name == class_name)
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
