use unica_metadata_designer::validation_context::meta_validate_registrar_document_scan;
use unica_metadata_designer::validation_context::READABLE_NS;
use unica_metadata_designer::xml_model::MD_CLASSES_NS;

use std::fs;
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use unica_metadata::MetadataKind;

    fn xr_leaf_text<'a>(source: &'a str, label: &str) -> &'a str {
        let opening = format!("<xr:{label}>");
        let closing = format!("</xr:{label}>");
        let start = source.find(&opening).unwrap() + opening.len();
        let end = start + source[start..].find(&closing).unwrap();
        &source[start..end]
    }

    fn replace_first_xr_leaf(source: &str, label: &str, replacement: &str) -> String {
        let current = xr_leaf_text(source, label);
        source.replacen(
            &format!("<xr:{label}>{current}</xr:{label}>",),
            &format!("<xr:{label}>{replacement}</xr:{label}>",),
            1,
        )
    }

    fn temp_documents(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-registrar-scan-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn registrar_scan_matches_only_register_records_items() {
        let root = temp_documents("typed-path");
        let reference = "InformationRegister.Ledger";
        fs::write(
            root.join("CommentOnly.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>CommentOnly</Name><Comment>{reference}</Comment><RegisterRecords/></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();

        let (_, found) = meta_validate_registrar_document_scan(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(&root),
            &root,
            reference,
        )
        .unwrap();
        assert!(
            !found,
            "an unrelated property must not count as registrar evidence"
        );

        fs::write(
            root.join("Registrar.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Document><Properties><Name>Registrar</Name><RegisterRecords><Item>{reference}</Item></RegisterRecords></Properties></Document></MetaDataObject>"#
            ),
        )
        .unwrap();
        let (_, found) = meta_validate_registrar_document_scan(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(&root),
            &root,
            reference,
        )
        .unwrap();
        assert!(found);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registrar_scan_propagates_unreadable_document_evidence() {
        let root = temp_documents("invalid-utf8");
        fs::write(root.join("Broken.xml"), [0xff, 0xfe]).unwrap();

        let error = meta_validate_registrar_document_scan(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(&root),
            &root,
            "InformationRegister.Ledger",
        )
        .expect_err("unreadable registrar evidence must stay unavailable");

        assert!(error.contains("UTF-8"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn event_source_registration_requires_one_exact_direct_owner_entry() {
        let owner = |registrations: &str| {
            format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Configuration><Properties><Name>Configuration</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            )
        };
        unica_metadata_designer::validation_context::validate_event_source_registration(
            owner("<Catalog>Items</Catalog>").as_bytes(),
            "Catalog",
            "Items",
        )
        .unwrap();

        let duplicate = owner("<Catalog>Items</Catalog><Catalog>Items</Catalog>");
        assert!(
            unica_metadata_designer::validation_context::validate_event_source_registration(
                duplicate.as_bytes(),
                "Catalog",
                "Items"
            )
            .unwrap_err()
            .contains("found 2")
        );

        let padded = owner("<Catalog> Items </Catalog>");
        assert!(
            unica_metadata_designer::validation_context::validate_event_source_registration(
                padded.as_bytes(),
                "Catalog",
                "Items"
            )
            .unwrap_err()
            .contains("surrounding whitespace")
        );
    }

    #[test]
    fn event_source_dependency_requires_exact_generated_type_shape_and_ids() {
        let (descriptor, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::Catalog,
                "Items",
            )
            .unwrap();
        let prefixes = vec!["CatalogObject".to_string(), "CatalogRef".to_string()];
        unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
            descriptor.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .unwrap();

        let missing =
            descriptor.replacen("name=\"CatalogRef.Items\"", "name=\"CatalogRef.Shadow\"", 1);
        assert!(unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
            missing.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .unwrap_err()
        .contains("exactly one GeneratedType"));

        let invalid_id = descriptor.replacen("<xr:TypeId>", "<xr:TypeId>not-a-uuid<!--", 1);
        assert!(unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
            invalid_id.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .is_err());
    }

    #[test]
    fn event_source_dependency_uses_the_nested_recalculation_generated_name() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:xr="{READABLE_NS}" version="2.20"><Recalculation><InternalInfo><xr:GeneratedType name="RecalculationRecordSet.Payroll.Main" category="RecordSet"><xr:TypeId>2f141d6b-a37b-4528-af7b-b70d3b63b011</xr:TypeId><xr:ValueId>e36ad3f3-d8e3-467a-849a-f549adbe56d1</xr:ValueId></xr:GeneratedType></InternalInfo><Properties><Name>Main</Name></Properties></Recalculation></MetaDataObject>"#
        );
        let prefixes = vec!["RecalculationRecordSet".to_string()];

        unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
            descriptor.as_bytes(),
            "Recalculation",
            "Main",
            Some("Payroll.Main"),
            &prefixes,
        )
        .unwrap();
    }

    #[test]
    fn event_source_dependency_rejects_nil_generated_type_ids() {
        let (descriptor, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::Catalog,
                "Items",
            )
            .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];

        for label in ["TypeId", "ValueId"] {
            let nil_id =
                replace_first_xr_leaf(&descriptor, label, "00000000-0000-0000-0000-000000000000");
            let error = unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
                nil_id.as_bytes(),
                "Catalog",
                "Items",
                None,
                &prefixes,
            )
            .expect_err("nil GeneratedType identifiers must be rejected");
            assert!(error.contains("non-nil"), "{error}");
        }
    }

    #[test]
    fn event_source_dependency_rejects_non_v4_generated_type_ids() {
        let (descriptor, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::Catalog,
                "Items",
            )
            .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];

        for label in ["TypeId", "ValueId"] {
            let version_one =
                replace_first_xr_leaf(&descriptor, label, "67e55044-10b1-11eb-adc1-0242ac120002");
            let error = unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
                version_one.as_bytes(),
                "Catalog",
                "Items",
                None,
                &prefixes,
            )
            .expect_err("non-v4 GeneratedType identifiers must be rejected");
            assert!(error.contains("version 4"), "{error}");
        }
    }

    #[test]
    fn event_source_dependency_rejects_equal_generated_type_ids() {
        let (descriptor, _) =
            unica_metadata_designer::template_catalog::minimal_metadata_xml_for_tests(
                MetadataKind::Catalog,
                "Items",
            )
            .unwrap();
        let prefixes = vec!["CatalogObject".to_string()];
        let type_id = xr_leaf_text(&descriptor, "TypeId");
        let equal_ids = replace_first_xr_leaf(&descriptor, "ValueId", type_id);

        let error = unica_metadata_designer::validation_context::validate_event_source_dependency_descriptor(
            equal_ids.as_bytes(),
            "Catalog",
            "Items",
            None,
            &prefixes,
        )
        .expect_err("equal GeneratedType identifiers must be rejected");
        assert!(error.contains("distinct"), "{error}");
    }
}
