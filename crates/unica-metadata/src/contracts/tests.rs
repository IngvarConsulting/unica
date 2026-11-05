use crate::apply_operations::MetadataOperation;
use crate::contracts::MetadataSite;
use crate::MetadataKind;
use serde_json::json;

#[test]
fn tabular_section_set_publishes_only_proven_owner_properties() {
    for owner in MetadataKind::ALL {
        let contract = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
            source_kind: unica_source_contract::SourceSetKind::Configuration,
            owner: *owner,
            in_section: true,
        }));
        let fields = &contract.schema["properties"]["values"]["properties"];
        assert_eq!(
            fields.get("Use").is_some(),
            matches!(
                owner,
                MetadataKind::Catalog | MetadataKind::ChartOfCharacteristicTypes
            ),
            "{owner:?} Use"
        );
        assert_eq!(
            fields.get("required").is_some(),
            matches!(
                owner,
                MetadataKind::Catalog
                    | MetadataKind::Document
                    | MetadataKind::BusinessProcess
                    | MetadataKind::ExchangePlan
                    | MetadataKind::Report
                    | MetadataKind::DataProcessor
            ),
            "{owner:?} required"
        );
        for forbidden in ["type", "fillValue", "indexing"] {
            assert!(fields.get(forbidden).is_none(), "{owner:?} {forbidden}");
        }
        if owner == &MetadataKind::Catalog {
            assert_eq!(
                fields["Use"]["enum"],
                json!(["ForItem", "ForFolder", "ForFolderAndItem"])
            );
        }
    }
}

#[test]
fn tabular_properties_are_update_only_and_configuration_only() {
    for source_kind in [
        unica_source_contract::SourceSetKind::Extension,
        unica_source_contract::SourceSetKind::ExternalProcessor,
        unica_source_contract::SourceSetKind::ExternalReport,
    ] {
        let contract = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
            owner: MetadataKind::Catalog,
            in_section: true,
            source_kind,
        }));
        for values in [json!({"Use":"ForItem"}), json!({"required":true})] {
            assert!(contract
                .normalize(&json!({"values":values}), "args")
                .is_err());
        }
    }
    let add = MetadataOperation::TabularSectionAdd.contract_at(Some(MetadataSite {
        owner: MetadataKind::Catalog,
        in_section: false,
        source_kind: unica_source_contract::SourceSetKind::Configuration,
    }));
    for field in ["Use", "required", "type", "indexing", "fillValue"] {
        assert!(add.schema["properties"]["items"]["items"]["properties"]
            .get(field)
            .is_none());
    }
    let set = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
        owner: MetadataKind::Catalog,
        in_section: true,
        source_kind: unica_source_contract::SourceSetKind::Configuration,
    }));
    for values in [
        json!({"Use":"Unknown"}),
        json!({"required":"true"}),
        json!({"Use":"ForItem","indexing":"Index"}),
    ] {
        assert!(set.normalize(&json!({"values":values}), "args").is_err());
    }
}
