use unica_address::address::NodeKind;
#[cfg(test)]
use unica_address::module_routes::ModuleRole;
use unica_bsl::profile::PlatformProfile;
use unica_source_designer::layout::metadata_kind;

#[cfg(test)]
pub(crate) fn supports_direct_module_role(tag: &str, role: &str) -> bool {
    metadata_kind(tag)
        .and_then(|layout| NodeKind::parse(layout.tag).ok())
        .zip(ModuleRole::from_v12_terminal(role))
        .is_some_and(|(owner, role)| {
            PlatformProfile::v8_3_27().supports_direct_module_role(owner, role)
        })
}

pub(crate) fn supports_nested_form_or_command(tag: &str) -> bool {
    metadata_kind(tag)
        .and_then(|layout| NodeKind::parse(layout.tag).ok())
        .is_some_and(|owner| PlatformProfile::v8_3_27().supports_nested_form_or_command(owner))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which module roles a kind owns is a fact about the platform, not about
    /// what happens to be on disk. Measured against an 8.3.27 vendor-class dump
    /// of 12290 BSL files: every role below occurs there for the kinds listed,
    /// and the refusals below never occur. The empty modules the platform omits
    /// on export are exactly why this cannot be inferred from the filesystem.
    #[test]
    fn direct_module_roles_match_the_platform_rather_than_the_filesystem() {
        for (kind, role) in [
            ("Catalog", "ObjectModule"),
            ("Catalog", "ManagerModule"),
            ("Document", "ObjectModule"),
            ("Document", "ManagerModule"),
            ("DataProcessor", "ObjectModule"),
            ("Report", "ObjectModule"),
            ("BusinessProcess", "ObjectModule"),
            ("Task", "ObjectModule"),
            ("ExchangePlan", "ObjectModule"),
            ("ChartOfCharacteristicTypes", "ObjectModule"),
            ("InformationRegister", "RecordSetModule"),
            ("AccumulationRegister", "RecordSetModule"),
            ("AccountingRegister", "RecordSetModule"),
            ("CalculationRegister", "RecordSetModule"),
            ("InformationRegister", "ManagerModule"),
            ("Constant", "ValueManagerModule"),
            ("Constant", "ManagerModule"),
            ("Enum", "ManagerModule"),
            ("DocumentJournal", "ManagerModule"),
        ] {
            assert!(
                supports_direct_module_role(kind, role),
                "{kind} must own {role}"
            );
        }

        // The distinctions a filesystem scan cannot make: these kinds have no
        // object module at all, so an absent file is not an omitted empty one.
        for (kind, role) in [
            ("Enum", "ObjectModule"),
            ("DocumentJournal", "ObjectModule"),
            ("InformationRegister", "ObjectModule"),
            ("Constant", "ObjectModule"),
            ("Catalog", "RecordSetModule"),
            ("Catalog", "ValueManagerModule"),
            ("Document", "RecordSetModule"),
            ("CommonModule", "ObjectModule"),
            ("Enum", "RecordSetModule"),
        ] {
            assert!(
                !supports_direct_module_role(kind, role),
                "{kind} must not own {role}"
            );
        }
    }

    #[test]
    fn v12_module_capability_adapter_keeps_exact_canonical_tag_inputs() {
        assert!(supports_direct_module_role("Document", "ObjectModule"));
        assert!(supports_nested_form_or_command("Document"));

        // Russian spellings belong to the hidden qualified-address parser, not
        // to the existing Platform XML layout registry consumed by v0.12.
        assert!(!supports_direct_module_role("Документ", "ObjectModule"));
        assert!(!supports_nested_form_or_command("Документ"));
        assert!(!supports_direct_module_role(
            "WebSocketClient",
            "ObjectModule"
        ));
        assert!(!supports_direct_module_role(
            "ExternalDataProcessor",
            "ObjectModule"
        ));
        assert!(!supports_nested_form_or_command("ExternalDataProcessor"));
        assert!(!supports_direct_module_role(
            "ExternalReport",
            "ObjectModule"
        ));
        assert!(!supports_nested_form_or_command("ExternalReport"));
    }
}
