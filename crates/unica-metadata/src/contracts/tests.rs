use crate::apply_operations::MetadataOperation;
use crate::contracts::MetadataSite;
use crate::MetadataKind;
use serde_json::json;

#[test]
fn tabular_section_set_publishes_only_proven_owner_properties() {
    for owner in MetadataKind::ALL {
        let contract = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
            tabular_section_properties: true,
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
fn tabular_properties_require_update_and_admitted_capability() {
    {
        let contract = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
            owner: MetadataKind::Catalog,
            in_section: true,
            tabular_section_properties: false,
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
        tabular_section_properties: true,
    }));
    for field in ["Use", "required", "type", "indexing", "fillValue"] {
        assert!(add.schema["properties"]["items"]["items"]["properties"]
            .get(field)
            .is_none());
    }
    let set = MetadataOperation::TabularSectionSet.contract_at(Some(MetadataSite {
        owner: MetadataKind::Catalog,
        in_section: true,
        tabular_section_properties: true,
    }));
    for values in [
        json!({"Use":"Unknown"}),
        json!({"required":"true"}),
        json!({"Use":"ForItem","indexing":"Index"}),
    ] {
        assert!(set.normalize(&json!({"values":values}), "args").is_err());
    }
}
