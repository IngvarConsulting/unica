//! Trusted binding for the existing bounded source-contained predefined read.
#[cfg(test)]
use crate::infrastructure::platform_xml_source_targets::ClosedPlatformXmlTarget;
use std::path::Path;
#[cfg(test)]
use unica_address::source_target::MetadataAddress;
#[cfg(test)]
use unica_metadata::*;
use unica_metadata_designer::predefined::*;
pub(crate) struct PredefinedResourceReadBinding;
impl PredefinedResourceRead for PredefinedResourceReadBinding {
    fn read_bounded_resource(
        &self,
        source_root: &Path,
        path: &Path,
        maximum: usize,
    ) -> std::io::Result<Vec<u8>> {
        unica_workspace::read::read_root_relative_regular_file(source_root, path, maximum, |_| {})
            .map(|observed| observed.bytes)
    }
    fn read_whole_resource(&self, source_root: &Path, path: &Path) -> std::io::Result<Vec<u8>> {
        unica_workspace::read::read_root_relative_regular_file_whole(source_root, path, |_| {})
            .map(|observed| observed.bytes)
    }
}
#[cfg(test)]
fn plan_predefined_resource(
    source_root: &Path,
    descriptor_path: &Path,
    owner: &MetadataAddress,
    kind: MetadataKind,
    descriptor_post_image: &str,
    operations: &[MetaEditOperation],
) -> Result<PlannedPredefinedResource<ClosedPlatformXmlTarget>, unica_metadata::request::MetaFailure>
{
    plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
        &PredefinedResourceReadBinding,
        source_root,
        unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
            path: descriptor_path,
            address: owner,
        },
        kind,
        descriptor_post_image,
        descriptor_post_image,
        operations,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_metadata::{
        MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetadataTypeVariant,
    };
    use unica_metadata_designer::predefined::test_support::{
        CFG_NS, MD_CLASSES_NS, PREDEFINED_NS, V8_NS, XSI_NS, XS_NS,
    };
    use unica_source_designer::roots::{
        platform_xml_owner_policy, platform_xml_publication_policy, PlatformXmlOwnerPolicy,
        PlatformXmlPublicationPolicy,
    };

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("unica-predefined-{label}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn owner(kind: &str) -> MetadataAddress {
        MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.Items"),
        )
        .unwrap()
    }

    #[test]
    fn exact_profile_rejects_entity_spelled_version_and_compact_item_uuid() {
        let valid = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name></Item></PredefinedData>"#;
        assert!(validate_predefined_text(MetadataKind::Catalog, valid).is_ok());
        assert!(validate_predefined_text(
            MetadataKind::Catalog,
            &valid.replace(r#"version="2.20""#, r#"version="2.2&#48;""#),
        )
        .is_err());
        assert!(validate_predefined_text(
            MetadataKind::Catalog,
            &valid.replace(
                "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "a7d2e6fc38244b56b4beae6be4944c0e",
            ),
        )
        .is_err());
    }

    #[test]
    fn code_writer_removes_controlled_xsi_type_for_empty_and_string_values() {
        let context = FragmentXmlContext::platform_default();
        let mut numeric = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="xs:decimal">643</Code></Item>"#
        );
        set_code_child(
            &mut numeric,
            "",
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            "\n",
            &context,
        )
        .unwrap();
        assert!(numeric.contains("<Code/>"), "{numeric}");
        assert!(!numeric.contains("xsi:type"), "{numeric}");

        let mut changed_type = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="xs:decimal">643</Code></Item>"#
        );
        set_code_child(
            &mut changed_type,
            "ABC",
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            "\n",
            &context,
        )
        .unwrap();
        assert!(changed_type.contains("<Code"), "{changed_type}");
        assert!(changed_type.contains(">ABC</Code>"), "{changed_type}");
        assert!(!changed_type.contains("xsi:type"), "{changed_type}");
    }

    #[test]
    fn equivalent_simple_update_preserves_attribute_order_byte_for_byte() {
        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Description future:stamp="keep" xmlns:future="urn:future">old</Description></Item>"#.to_string();
        let before = item.clone();
        set_simple_child(
            &mut item,
            "Description",
            "old",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();
        assert_eq!(item, before);
    }

    #[test]
    fn mixed_text_children_are_one_typed_value_for_validation_info_and_noop() {
        let invalid_number = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code xsi:type="xs:decimal">1<!--keep-->garbage</Code></Item></PredefinedData>"#
        );
        assert!(validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            &invalid_number,
        )
        .is_err());

        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Description>old<!--keep-->tail</Description></Item>"#.to_string();
        let before = item.clone();
        set_simple_child(
            &mut item,
            "Description",
            "old",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();
        assert_ne!(item, before, "old + tail must not compare equal to old");
        assert!(item.contains("<!--keep-->"), "{item}");
        assert!(!item.contains("tail"), "{item}");

        let info = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Description>old<!--keep-->tail</Description></Item></PredefinedData>"#
        );
        let data = read_predefined_items(info.as_bytes(), MetadataKind::Catalog, 10).unwrap();
        assert_eq!(data.items[0].description, "oldtail");
    }

    /// Планировщик `apply` пишет `Ext/Predefined.xml` сам: первый элемент
    /// создаёт файл, элемент находится по имени и во вложенных, последний
    /// удалённый элемент убирает файл.
    #[test]
    fn apply_post_image_creates_the_file_finds_items_by_name_and_drops_it_empty() {
        let owner = owner("Catalog");
        let descriptor = descriptor_image(MetadataKind::Catalog, "String");
        let id = "a1b2c3d4-1111-4111-8111-111111111111";
        let add = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: id.to_string(),
            name: "Рубль".to_string(),
            fields: MetaPredefinedFields::default(),
        }])
        .unwrap();
        let created = unica_metadata_designer::predefined::predefined_post_image(
            MetadataKind::Catalog,
            &owner,
            &descriptor,
            None,
            &add,
        )
        .unwrap()
        .expect("the first item creates the file");
        assert!(created.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(
            unica_metadata_designer::predefined::predefined_item_id(
                &created,
                MetadataKind::Catalog,
                "Рубль"
            )
            .unwrap(),
            Some(id.to_string())
        );
        assert_eq!(
            unica_metadata_designer::predefined::predefined_item_id(
                &created,
                MetadataKind::Catalog,
                "Евро"
            )
            .unwrap(),
            None
        );
        let nested = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="00000000-0000-4000-8000-000000000001"><Name>Parent</Name><Code/><Description/><IsFolder>true</IsFolder><ChildItems><Item id="00000000-0000-4000-8000-000000000002"><Name>Child</Name><Code/><Description/><IsFolder>false</IsFolder></Item></ChildItems></Item></PredefinedData>"#;
        assert_eq!(
            unica_metadata_designer::predefined::predefined_item_id(
                nested.as_bytes(),
                MetadataKind::Catalog,
                "Child"
            )
            .unwrap(),
            Some("00000000-0000-4000-8000-000000000002".to_string())
        );
        let remove = MetaEditOperation::remove_predefined_items(vec![id.to_string()]).unwrap();
        assert_eq!(
            unica_metadata_designer::predefined::predefined_post_image(
                MetadataKind::Catalog,
                &owner,
                &descriptor,
                Some(&created),
                &remove
            )
            .unwrap(),
            None,
            "the last item leaves no empty document"
        );
        let missing = MetaEditOperation::remove_predefined_items(vec![
            "00000000-0000-4000-8000-000000000009".to_string(),
        ])
        .unwrap();
        let failure = unica_metadata_designer::predefined::predefined_post_image(
            MetadataKind::Catalog,
            &owner,
            &descriptor,
            Some(&created),
            &missing,
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].field.as_deref(),
            Some("ids[0]"),
            "fields name the operation's own request"
        );
    }

    fn descriptor_image(kind: MetadataKind, code_type: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"{MD_CLASSES_NS}\" version=\"2.20\"><{kind}><Properties><Name>Items</Name><CodeType>{code_type}</CodeType></Properties></{kind}></MetaDataObject>",
            kind = kind.as_str()
        )
    }

    fn set_catalog_code_type(value: &str) -> MetaEditOperation {
        MetaEditOperation::SetProperties {
            values: MetaPropertyChanges::convert(
                MetadataKind::Catalog,
                vec![MetaPropertyInput::new(
                    "CodeType",
                    MetaPropertyValue::String(value.to_string()),
                )],
            )
            .unwrap(),
        }
    }

    /// #1119: корректный `Predefined.xml` больше прежних 8 МиБ читается
    /// целиком, а не отклоняется по размеру.
    #[test]
    fn predefined_plan_reads_a_resource_larger_than_eight_mebibytes() {
        let root = temp_root("predefined-large");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let padding = format!("<!--{}-->", "x".repeat(9 * 1024 * 1024));
        let image = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="CatalogPredefinedItems" version="2.20">{padding}<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code>643</Code></Item></PredefinedData>"#
        );
        assert!(image.len() > 8 * 1024 * 1024);
        fs::write(&path, &image).unwrap();
        let descriptor_text = descriptor_image(MetadataKind::Catalog, "String");
        let update = MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: None,
            fields: MetaPredefinedFields {
                code: Some("644".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let planned = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_text,
            &descriptor_text,
            std::slice::from_ref(&update),
        );
        assert!(planned.is_ok(), "{:?}", planned.err());
        assert_eq!(fs::read_to_string(&path).unwrap(), image);
        let _ = fs::remove_dir_all(&root);
    }

    /// #1381: описание без `CodeType` (так платформа выгружает заимствованный
    /// справочник) не даёт писателю проверять коды по придуманному виду:
    /// запись предопределённых отказывает тем же текстом, что и чтение.
    #[test]
    fn predefined_writer_refuses_an_owner_without_code_type_like_the_reader() {
        let root = temp_root("code-type-absent");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let image = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code>643</Code></Item></PredefinedData>"#
        );
        fs::write(&path, &image).unwrap();
        let without = descriptor_image(MetadataKind::Catalog, "String")
            .replace("<CodeType>String</CodeType>", "");
        let update = MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: None,
            fields: MetaPredefinedFields {
                code: Some("644".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let rejected = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &without,
            &without,
            std::slice::from_ref(&update),
        )
        .err()
        .expect("an unknown CodeType must not be defaulted");
        assert_eq!(
            rejected.diagnostics[0].message,
            unica_metadata_designer::predefined::MISSING_PREDEFINED_CODE_TYPE
        );
        assert_eq!(
            rejected.diagnostics[0].field.as_deref(),
            Some("operations[0].properties.CodeType")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), image);

        // Writing CodeType onto such an owner is not refused for the absence.
        let planned = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &without,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[set_catalog_code_type("String")],
        );
        if let Err(failure) = planned {
            assert_ne!(
                failure.diagnostics[0].message,
                unica_metadata_designer::predefined::MISSING_PREDEFINED_CODE_TYPE
            );
        }
    }

    #[test]
    fn code_type_transition_validates_or_explicitly_rewrites_predefined_codes_atomically() {
        let root = temp_root("code-type-transition");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let string_image = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xmlns:xs="{XS_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code>643</Code></Item></PredefinedData>"#
        );
        fs::write(&path, &string_image).unwrap();
        let set_number = set_catalog_code_type("Number");

        let rejected = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &descriptor_image(MetadataKind::Catalog, "Number"),
            std::slice::from_ref(&set_number),
        )
        .err()
        .expect("implicit Code migration must be rejected");
        assert_eq!(
            rejected.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(rejected.diagnostics[0].operation_index, Some(0));
        assert_eq!(
            rejected.diagnostics[0].field.as_deref(),
            Some("operations[0].values.CodeType")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), string_image);

        let number_image = string_image.replace(
            "<Code>643</Code>",
            r#"<Code xsi:type="xs:decimal">643</Code>"#,
        );
        fs::write(&path, &number_image).unwrap();
        let rejected_string =
            plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
                &PredefinedResourceReadBinding,
                &root,
                unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                    path: &descriptor,
                    address: &owner("Catalog"),
                },
                MetadataKind::Catalog,
                &descriptor_image(MetadataKind::Catalog, "Number"),
                &descriptor_image(MetadataKind::Catalog, "String"),
                &[set_catalog_code_type("String")],
            )
            .err()
            .expect("implicit xsi:type removal must be rejected");
        assert_eq!(
            rejected_string.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), number_image);
        fs::write(&path, &string_image).unwrap();

        let explicit_update =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    code: Some("643".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
        let planned = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[set_number, explicit_update],
        )
        .unwrap();
        let post_image = planned.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        assert!(String::from_utf8_lossy(post_image)
            .contains(r#"<Code xsi:type="xs:decimal">643</Code>"#));
        assert_eq!(planned.effects.len(), 1);
        assert_eq!(planned.effects[0].operation_index, Some(1));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_or_absent_code_type_companion_keeps_precise_preimage_guards() {
        let root = temp_root("code-type-companion-guards");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not xml").unwrap();
        let unchanged = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[set_catalog_code_type("String")],
        )
        .unwrap();
        assert!(unchanged.resources.file_mutations.is_empty());
        assert!(unchanged.resources.exact_file_guards.is_empty());

        let compatible = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code/></Item></PredefinedData>"#
        );
        fs::write(&path, &compatible).unwrap();
        let guarded = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[set_catalog_code_type("Number")],
        )
        .unwrap();
        assert!(guarded.resources.file_mutations.is_empty());
        assert_eq!(guarded.resources.exact_file_guards.len(), 1);
        assert_eq!(
            guarded.resources.exact_file_guards[0].1,
            compatible.as_bytes()
        );

        fs::remove_file(&path).unwrap();
        let changed = plan_predefined_resource_after_descriptor_edit::<ClosedPlatformXmlTarget>(
            &PredefinedResourceReadBinding,
            &root,
            unica_metadata_designer::resource_plan::MetadataDescriptorTarget {
                path: &descriptor,
                address: &owner("Catalog"),
            },
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[set_catalog_code_type("Number")],
        )
        .unwrap();
        assert_eq!(changed.resources.absent_path_guards, vec![path]);
        assert!(changed.resources.file_mutations.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn info_is_flat_document_order_with_explicit_root_parent() {
        let bytes = br#"<?xml version="1.0"?><PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Parent</Name><ChildItems><Item id="b7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Child</Name></Item></ChildItems></Item></PredefinedData>"#;
        let data = read_predefined_items(bytes, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].parent_id, None);
        assert_eq!(
            data.items[1].parent_id.as_deref(),
            Some("a7d2e6fc-3824-4b56-b4be-ae6be4944c0e")
        );
        let json = serde_json::to_value(&data).unwrap();
        assert!(json["items"][0].get("parentId").is_some());
        assert!(json["items"][0]["parentId"].is_null());
    }

    #[test]
    fn account_type_wire_value_round_trips_in_platform_case() {
        assert_eq!(
            serde_json::to_value(MetaPredefinedAccountType::ActivePassive).unwrap(),
            Value::String("ActivePassive".to_string())
        );
        assert_eq!(
            MetaPredefinedAccountType::parse("ActivePassive", "accountType").unwrap(),
            MetaPredefinedAccountType::ActivePassive
        );
    }

    #[test]
    fn predefined_root_uses_the_shared_platform_root_policies() {
        assert_eq!(
            platform_xml_publication_policy(PREDEFINED_NS, "PredefinedData"),
            Some(PlatformXmlPublicationPolicy::ExactRootVersion)
        );
        assert_eq!(
            platform_xml_owner_policy(PREDEFINED_NS, "PredefinedData"),
            Some(PlatformXmlOwnerPolicy::ContainerScoped)
        );
    }

    #[test]
    fn other_exact_registered_roots_are_not_predefined_data() {
        for xml in [
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"/>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"/>"#,
        ] {
            assert!(validate_predefined_text(MetadataKind::Catalog, xml).is_err());
        }
    }

    #[test]
    fn type_qname_is_resolved_by_namespace_not_by_prefix_spelling() {
        let bytes = br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="PlanOfCharacteristicKindPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:future="urn:future" future:note="a>b"><v8:Type>ent:CatalogRef.Items</v8:Type></Type><IsFolder>false</IsFolder></Item></PredefinedData>"#;
        let data =
            read_predefined_items(bytes, MetadataKind::ChartOfCharacteristicTypes, 10).unwrap();
        assert!(matches!(
            data.items[0].r#type.as_ref().unwrap().variants.as_slice(),
            [MetadataTypeVariant::Reference { metadata_path }]
                if metadata_path.as_str() == "Catalog.Items"
        ));

        let segmented = String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace("ent:CatalogRef.Items", "v8:Value<!--keep-->Storage");
        let data = read_predefined_items(
            segmented.as_bytes(),
            MetadataKind::ChartOfCharacteristicTypes,
            10,
        )
        .unwrap();
        assert!(matches!(
            data.items[0].r#type.as_ref().unwrap().variants.as_slice(),
            [MetadataTypeVariant::ValueStorage]
        ));

        let invalid = segmented.replace(
            "v8:Value<!--keep-->Storage",
            "v8:ValueStorage<!--keep-->garbage",
        );
        assert!(read_predefined_items(
            invalid.as_bytes(),
            MetadataKind::ChartOfCharacteristicTypes,
            10,
        )
        .is_err());

        let nested_extension = String::from_utf8(bytes.to_vec()).unwrap().replace(
            "</v8:Type>",
            concat!(
                "</v8:Type>",
                "<future:Extension>",
                "<v8:Type>v8:ValueStorage</v8:Type>",
                "</future:Extension>"
            ),
        );
        let data = read_predefined_items(
            nested_extension.as_bytes(),
            MetadataKind::ChartOfCharacteristicTypes,
            10,
        )
        .unwrap();
        assert!(matches!(
            data.items[0].r#type.as_ref().unwrap().variants.as_slice(),
            [MetadataTypeVariant::Reference { metadata_path }]
                if metadata_path.as_str() == "Catalog.Items"
        ));
    }

    #[test]
    fn type_update_preserves_the_item_child_indentation() {
        let mut item = concat!(
            "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n",
            "\t<Name>Kind</Name>\n",
            "\t<Type/>\n",
            "\t<IsFolder>false</IsFolder>\n",
            "</Item>"
        )
        .to_string();
        let fields = MetaPredefinedFields {
            r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(
            item.contains(&format!(
                "\n\t<Type xmlns:v8=\"{V8_NS}\" xmlns:xs=\"{XS_NS}\">\n\t\t<v8:Type>xs:boolean</v8:Type>\n\t</Type>\n"
            )),
            "{item}"
        );
        assert!(!item.lines().any(|line| line.starts_with("<v8:")), "{item}");
    }

    #[test]
    fn type_writer_reuses_aliases_or_declares_every_required_namespace_locally() {
        let aliases =
            FragmentXmlContext::from_namespaces([("core", V8_NS), ("schema", XS_NS)].into_iter());
        let metadata_type = MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap();
        let aliased = render_type(&metadata_type, "", "\n", &aliases);
        assert_eq!(
            aliased,
            "<Type>\n\t<core:Type>schema:boolean</core:Type>\n</Type>"
        );

        let local_type = MetadataType::new(vec![
            MetadataTypeVariant::Boolean,
            MetadataTypeVariant::Reference {
                metadata_path: owner("Catalog"),
            },
        ])
        .unwrap();
        let local = render_type(
            &local_type,
            "\t",
            "\n",
            &FragmentXmlContext::platform_default(),
        );
        for declaration in [
            format!(r#"xmlns:v8="{V8_NS}""#),
            format!(r#"xmlns:xs="{XS_NS}""#),
            format!(r#"xmlns:cfg="{CFG_NS}""#),
        ] {
            assert!(
                local.contains(&declaration),
                "missing {declaration}: {local}"
            );
        }
        let xml = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="PlanOfCharacteristicKindPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name>{local}<IsFolder>false</IsFolder></Item></PredefinedData>"#
        );
        validate_predefined_text(MetadataKind::ChartOfCharacteristicTypes, &xml).unwrap();
        let data =
            read_predefined_items(xml.as_bytes(), MetadataKind::ChartOfCharacteristicTypes, 10)
                .unwrap();
        assert_eq!(data.items[0].r#type.as_ref().unwrap(), &local_type);
    }

    #[test]
    fn type_update_avoids_a_local_prefix_collision_and_preserves_attribute_identity() {
        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:v8="urn:future" v8:stamp="keep"/><IsFolder>false</IsFolder></Item>"#.to_string();
        let fields = MetaPredefinedFields {
            r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains(&format!(r#"xmlns:v8_1="{V8_NS}""#)), "{item}");
        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        let document = Document::parse(&wrapped).unwrap();
        let type_node = document
            .descendants()
            .find(|node| is_predefined_element(*node, "Type"))
            .unwrap();
        assert_eq!(type_node.attribute(("urn:future", "stamp")), Some("keep"));
        assert_eq!(type_node.attribute((V8_NS, "stamp")), None);
        assert!(type_node
            .children()
            .any(|node| node.tag_name().namespace() == Some(V8_NS)));
    }

    #[test]
    fn type_update_recursively_preserves_unknown_extension_descendants() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:v8="{V8_NS}" xmlns:xs="{XS_NS}" xmlns:f="urn:future"><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits><f:BeforeDigits/>10<f:InsideDigits/></v8:Digits><f:InsideQualifiers/><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers><f:InsideType/></Type><IsFolder>false</IsFolder></Item>"#
        );
        let fields = MetaPredefinedFields {
            r#type: Some(
                MetadataType::new(vec![MetadataTypeVariant::Number {
                    digits: 12,
                    fraction: 3,
                    sign: unica_metadata::NumberSign::NonNegative,
                }])
                .unwrap(),
            ),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        for expected in [
            "<v8:Digits><f:BeforeDigits/>12<f:InsideDigits/></v8:Digits>",
            "<f:InsideQualifiers/>",
            "<f:InsideType/>",
            "<v8:FractionDigits>3</v8:FractionDigits>",
            "<v8:AllowedSign>Nonnegative</v8:AllowedSign>",
        ] {
            assert!(item.contains(expected), "missing {expected}: {item}");
        }
        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn type_update_rejects_discarding_an_extension_in_a_removed_typed_branch() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:v8="{V8_NS}" xmlns:xs="{XS_NS}" xmlns:f="urn:future"><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>10</v8:Digits><f:Keep/><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers></Type><IsFolder>false</IsFolder></Item>"#
        );
        let before = item.clone();
        let fields = MetaPredefinedFields {
            r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
            ..MetaPredefinedFields::default()
        };

        let error = patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap_err();

        assert!(error.contains("would discard unknown XML"), "{error}");
        assert_eq!(item, before);
    }

    #[test]
    fn type_update_uses_an_alias_not_shadowed_by_a_nested_declaration() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:v8="{V8_NS}" xmlns:xs="{XS_NS}"><v8:Type>xs:decimal</v8:Type><core:NumberQualifiers xmlns:core="{V8_NS}" xmlns:v8="urn:future"><core:Digits>10</core:Digits><core:FractionDigits>2</core:FractionDigits><core:AllowedSign>Any</core:AllowedSign></core:NumberQualifiers></Type><IsFolder>false</IsFolder></Item>"#
        );
        let fields = MetaPredefinedFields {
            r#type: Some(
                MetadataType::new(vec![MetadataTypeVariant::Number {
                    digits: 12,
                    fraction: 3,
                    sign: unica_metadata::NumberSign::NonNegative,
                }])
                .unwrap(),
            ),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        let document = Document::parse(&wrapped).unwrap();
        for name in [
            "NumberQualifiers",
            "Digits",
            "FractionDigits",
            "AllowedSign",
        ] {
            let node = document
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == name)
                .unwrap();
            assert_eq!(node.tag_name().namespace(), Some(V8_NS), "{item}");
        }
        assert!(item.contains(&format!(r#"xmlns:v8_1="{V8_NS}""#)), "{item}");
        assert!(item.contains(r#"xmlns:v8="urn:future""#), "{item}");
    }

    #[test]
    fn ext_dimension_update_inherits_an_alias_declared_on_its_container() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><ExtDimensionTypes xmlns:p="{PREDEFINED_NS}" xmlns:f="urn:future"><p:ExtDimensionType name="Kinds"><p:Turnover><f:BeforeTurnover/>false<f:TurnoverMarker/></p:Turnover><p:AccountingFlags><p:Flag ref="Debit"><?keep flag?>false<f:FlagMarker/></p:Flag></p:AccountingFlags></p:ExtDimensionType></ExtDimensionTypes></Item>"#
        );
        let requested = [MetaPredefinedExtDimensionType {
            name: "Kinds".to_string(),
            turnover: Some(true),
            accounting_flags: Some(BTreeMap::from([("Debit".to_string(), true)])),
        }];

        merge_ext_dimensions(
            &mut item,
            &requested,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(
            item.contains("<p:Turnover><f:BeforeTurnover/>true<f:TurnoverMarker/></p:Turnover>"),
            "{item}"
        );
        assert!(
            item.contains("<p:Flag ref=\"Debit\"><?keep flag?>true<f:FlagMarker/></p:Flag>"),
            "{item}"
        );
        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn replacement_preserves_local_namespace_declarations_and_unknown_attributes() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Description xmlns:p="{PREDEFINED_NS}" xmlns:future="urn:future" future:stamp="keep">old<future:Marker/></p:Description></Item>"#
        );

        set_simple_child(
            &mut item,
            "Description",
            "new",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains(&format!(
            r#"<p:Description xmlns:p="{PREDEFINED_NS}" xmlns:future="urn:future" future:stamp="keep">new<future:Marker/></p:Description>"#
        )), "{item}");
        let wrapped = format!("<Root xmlns=\"{PREDEFINED_NS}\">{item}</Root>");
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn replacement_preserves_extension_order_around_the_typed_text_slot() {
        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Description xmlns:f="urn:future"><f:Before/><?keep before?>old<!--after--><f:After/></Description></Item>"#.to_string();

        set_simple_child(
            &mut item,
            "Description",
            "new",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains(
            r#"<Description xmlns:f="urn:future"><f:Before/><?keep before?>new<!--after--><f:After/></Description>"#
        ), "{item}");
        let wrapped = format!("<Root xmlns=\"{PREDEFINED_NS}\">{item}</Root>");
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn explicit_empty_structural_fields_clear_typed_children_and_preserve_unknown_xml() {
        let context = FragmentXmlContext::platform_default();
        let mut item = concat!(
            "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n",
            "\t<AccountingFlags xmlns:future=\"urn:future\" future:stamp=\"keep\">\n",
            "\t\t<Flag ref=\"Debit\">true</Flag>  <future:Keep/>\n",
            "\t\t<Flag ref=\"Credit\">false</Flag>\n",
            "\t</AccountingFlags>\n",
            "\t<ExtDimensionTypes future:stamp=\"keep\" xmlns:future=\"urn:future\">\n",
            "\t\t<ExtDimensionType name=\"Kinds\"><Turnover>true</Turnover></ExtDimensionType>  <future:Keep/>\n",
            "\t</ExtDimensionTypes>\n",
            "</Item>"
        )
        .to_string();

        merge_flags_container(
            &mut item,
            "AccountingFlags",
            &BTreeMap::new(),
            "\n",
            &context,
        )
        .unwrap();
        merge_ext_dimensions(&mut item, &[], "\n", &context).unwrap();

        assert!(!item.contains("<Flag "), "{item}");
        assert!(!item.contains("<ExtDimensionType "), "{item}");
        assert_eq!(item.matches("<future:Keep/>").count(), 2, "{item}");
        assert_eq!(item.matches("\t\t  <future:Keep/>").count(), 2, "{item}");
        assert_eq!(item.matches("future:stamp=\"keep\"").count(), 2, "{item}");
    }

    #[test]
    fn planner_clears_explicit_empty_structural_fields_and_repeats_as_a_noop() {
        let root = temp_root("clear-structural-fields");
        let descriptor = root.join("ChartsOfAccounts/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:future="urn:future" xmlns:xsi="{XSI_NS}" xsi:type="ChartOfAccountsPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><AccountingFlags future:stamp="keep"><Flag ref="Debit">true</Flag><future:Keep/></AccountingFlags><ExtDimensionTypes future:stamp="keep"><ExtDimensionType name="Kinds"><Turnover>true</Turnover></ExtDimensionType><future:Keep/></ExtDimensionTypes></Item></PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    accounting_flags: Some(BTreeMap::new()),
                    ext_dimension_types: Some(Vec::new()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            std::slice::from_ref(&operation),
        )
        .unwrap();
        assert_eq!(first.resources.file_mutations.len(), 1);
        assert_ne!(first.effects[0].before, first.effects[0].after);
        let after = first.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let after_text = std::str::from_utf8(after).unwrap();
        assert!(!after_text.contains("<Flag "), "{after_text}");
        assert!(!after_text.contains("<ExtDimensionType "), "{after_text}");
        assert_eq!(after_text.matches("<future:Keep/>").count(), 2);
        assert_eq!(after_text.matches("future:stamp=\"keep\"").count(), 2);

        fs::write(&path, after).unwrap();
        let repeated = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            &[operation],
        )
        .unwrap();
        assert!(repeated.resources.file_mutations.is_empty());
        assert_eq!(repeated.effects[0].before, repeated.effects[0].after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_effect_reports_the_entire_subtree_in_document_order() {
        let parent = "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let child = "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let sibling = "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let xml = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="{parent}"><Name>Parent</Name><ChildItems><Item id="{child}"><Name>Child</Name></Item></ChildItems></Item><Item id="{sibling}"><Name>Sibling</Name></Item></PredefinedData>"#
        );
        let operation =
            MetaEditOperation::remove_predefined_items(vec![parent.to_string()]).unwrap();

        let before = selected_effect(&xml, MetadataKind::Catalog, &operation, false)
            .unwrap()
            .unwrap();
        let items = before.as_array().unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], parent);
        assert_eq!(items[0]["parentId"], Value::Null);
        assert_eq!(items[1]["id"], child);
        assert_eq!(items[1]["parentId"], parent);
        assert!(!items.iter().any(|item| item["id"] == sibling));
    }

    #[test]
    fn remove_preserves_bytes_before_an_inline_following_sibling() {
        let removed = "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let kept = "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let mut xml = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20">
	<Item id="{removed}"><Name>Drop</Name></Item>  <Item id="{kept}"><Name>Keep</Name></Item>
</PredefinedData>"#
        );
        let operation =
            MetaEditOperation::remove_predefined_items(vec![removed.to_string()]).unwrap();

        apply_operation(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &operation,
            &owner("Catalog"),
            0,
        )
        .unwrap();

        assert!(
            xml.contains(&format!("\n\t  <Item id=\"{kept}\">")),
            "{xml}"
        );
        validate_predefined_text(MetadataKind::Catalog, &xml).unwrap();
    }

    #[test]
    fn duplicate_direct_supported_child_is_rejected() {
        let xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>First</Name><Name>Second</Name></Item></PredefinedData>"#;
        assert!(validate_predefined_text(MetadataKind::Catalog, xml).is_err());
    }

    #[test]
    fn missing_supported_field_is_inserted_before_child_items() {
        let mut item = "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n\t<Name>Parent</Name>\n\t<ChildItems/>\n</Item>".to_string();
        set_simple_child(
            &mut item,
            "Code",
            "01",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();
        assert!(item.find("<Code>01</Code>").unwrap() < item.find("<ChildItems/>").unwrap());
    }

    #[test]
    fn planner_repairs_a_self_closing_item_when_update_supplies_its_name() {
        let root = temp_root("self-closing-item");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"/></PredefinedData>"#,
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: Some("Main".to_string()),
                fields: MetaPredefinedFields {
                    description: Some("Repaired".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = planned.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let data = read_predefined_items(after, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(data.items[0].name, "Main");
        assert_eq!(data.items[0].description, "Repaired");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_preserves_unknown_nodes_using_a_namespace_declared_on_the_document_root() {
        let root = temp_root("inherited-unknown-namespace");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let unknown = r#"<future:Extension future:mode="keep"><future:Nested/></future:Extension>"#;
        fs::write(
            &path,
            format!(
                r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:future="urn:unica:future" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e" future:stamp="keep"><Name>Main</Name>{unknown}</Item></PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    description: Some("Changed".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(after.contains(r#"future:stamp="keep""#));
        assert!(after.contains(unknown));
        assert!(after.contains("<Description>Changed</Description>"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_equivalence_does_not_ignore_an_existing_child_subtree() {
        let mut xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code/><Description/><IsFolder>false</IsFolder><ChildItems><Item id="b7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Child</Name></Item></ChildItems></Item></PredefinedData>"#.to_string();
        let owner = MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Items",
        )
        .unwrap();
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };
        let failure = add_item(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &element,
            PredefinedOperationContext {
                owner: &owner,
                operation_index: 0,
            },
            0,
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );
    }

    #[test]
    fn metadata_name_case_is_one_policy_for_flags_and_ext_dimensions() {
        // Имена метаданных 1С сравниваются без учёта регистра. Обновление
        // существующего флага другим написанием обязано попасть в него, а не
        // дописать семантический дубль; написание источника при этом остаётся.
        let context = FragmentXmlContext::platform_default();
        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><AccountingFlags><Flag ref="Debit">false</Flag></AccountingFlags></Item>"#.to_string();
        merge_flags_container(
            &mut item,
            "AccountingFlags",
            &BTreeMap::from([("debit".to_string(), true)]),
            "\n",
            &context,
        )
        .unwrap();
        assert_eq!(item.matches("<Flag ").count(), 1, "{item}");
        assert!(item.contains(r#"<Flag ref="Debit">true</Flag>"#), "{item}");

        // Дубликат, отличающийся только регистром, отвергается на чтении —
        // как и у ExtDimensionType.
        let document = Document::parse(
            r#"<AccountingFlags xmlns="http://v8.1c.ru/8.3/xcf/predef"><Flag ref="Debit">true</Flag><Flag ref="debit">false</Flag></AccountingFlags>"#,
        )
        .unwrap();
        let error = parse_flags(Some(document.root_element())).unwrap_err();
        assert!(error.contains("duplicated"), "{error}");
    }

    #[test]
    fn remove_of_a_parent_and_its_descendant_is_order_independent() {
        let base = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Parent</Name><ChildItems><Item id="b7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Child</Name></Item></ChildItems></Item><Item id="c7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kept</Name></Item></PredefinedData>"#;
        let owner = owner("Catalog");
        let mut results = Vec::new();
        for ids in [
            [
                "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
            ],
            [
                "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
            ],
        ] {
            let operation = MetaEditOperation::remove_predefined_items(
                ids.iter().map(|id| id.to_string()).collect(),
            )
            .unwrap();
            let mut text = base.to_string();
            apply_operation(
                &mut text,
                "\n",
                MetadataKind::Catalog,
                unica_metadata_designer::predefined::PredefinedCodeType::String,
                &operation,
                &owner,
                0,
            )
            .unwrap();
            assert!(!text.contains("Parent"), "{text}");
            assert!(!text.contains("Child"), "{text}");
            assert!(text.contains("Kept"), "{text}");
            results.push(text);
        }
        assert_eq!(results[0], results[1], "оба порядка дают один пост-образ");
    }

    #[test]
    fn add_anchors_on_the_parsed_root_and_ignores_a_trailing_comment_decoy() {
        let mut xml = format!(
            "{}\n<!-- archived copy: </PredefinedData> -->\n",
            r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name></Item></PredefinedData>"#
        );

        add_item(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &MetaPredefinedItemAdd {
                id: "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: "Second".to_string(),
                fields: MetaPredefinedFields::default(),
            },
            PredefinedOperationContext {
                owner: &owner("Catalog"),
                operation_index: 0,
            },
            0,
        )
        .unwrap();

        let document = Document::parse(&xml).unwrap();
        let items = document
            .root_element()
            .children()
            .filter(|node| is_predefined_element(*node, "Item"))
            .count();
        assert_eq!(items, 2, "the new item must land inside the parsed root");
        let comment_start = xml.find("<!--").unwrap();
        assert!(
            !xml[comment_start..].contains("<Item id=\"b7d2e6fc"),
            "the trailing comment decoy must stay untouched"
        );
    }

    #[test]
    fn add_equivalence_accepts_an_empty_child_items_container() {
        let mut xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code/><Description/><IsFolder>false</IsFolder><ChildItems/></Item></PredefinedData>"#.to_string();
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };

        add_item(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::String,
            &element,
            PredefinedOperationContext {
                owner: &owner("Catalog"),
                operation_index: 0,
            },
            0,
        )
        .unwrap();

        assert!(xml.contains("<ChildItems/>"));
    }

    #[test]
    fn planner_creates_bom_file_and_second_equivalent_add_is_noop() {
        let root = temp_root("create-noop");
        let descriptor = root.join("Catalogs/Items.xml");
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };
        let operation = MetaEditOperation::add_predefined_items(vec![element]).unwrap();
        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            std::slice::from_ref(&operation),
        )
        .unwrap();
        assert_eq!(first.resources.file_mutations.len(), 1);
        let bytes = first.resources.file_mutations[0]
            .post_image
            .as_ref()
            .unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        let parsed = read_predefined_items(bytes, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(parsed.items[0].name, "Main");

        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let second = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        assert!(second.resources.file_mutations.is_empty());
        assert_eq!(second.resources.exact_file_guards.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removing_the_last_item_deletes_the_file_like_the_platform_export() {
        // Импортировав пустой `PredefinedData`, 8.3.27.2074 при выгрузке файл
        // не пишет: канонический вид «нет предопределённых» — отсутствие
        // файла, а не пустой документ.
        let root = temp_root("remove-last");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"CatalogPredefinedItems\" version=\"2.20\">\n",
            "\t<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\"><Name>Main</Name></Item>\n",
            "</PredefinedData>"
        );
        fs::write(&path, before.as_bytes()).unwrap();

        let operation = MetaEditOperation::remove_predefined_items(vec![
            "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
        ])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            std::slice::from_ref(&operation),
        )
        .unwrap();

        assert_eq!(planned.resources.file_mutations.len(), 1);
        let mutation = &planned.resources.file_mutations[0];
        assert_eq!(mutation.path, path);
        assert!(mutation.pre_image.is_some());
        assert!(mutation.post_image.is_none(), "файл должен удаляться");
        assert_eq!(planned.resources.publication_plan.len(), 1);
        assert_eq!(
            planned.resources.publication_plan[0].action,
            MetaPublicationAction::Remove
        );
        assert_eq!(planned.resources.absent_path_guards, vec![path]);
        assert!(planned.resources.expected_post_images.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn planner_expands_self_closing_root_and_preserves_crlf_and_bom() {
        let root = temp_root("crlf");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"CatalogPredefinedItems\" version=\"2.20\"/>\r\n"
        );
        fs::write(&path, before.as_bytes()).unwrap();
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = planned.resources.file_mutations[0]
            .post_image
            .as_ref()
            .unwrap();
        assert!(after.starts_with(b"\xef\xbb\xbf"));
        let text = std::str::from_utf8(&after[3..]).unwrap();
        assert!(text.contains("\r\n\t<Item"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        assert!(text.contains("</PredefinedData>"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn planner_rejects_ambiguous_mixed_eol_and_preserves_uniform_lone_cr() {
        let root = temp_root("observed-eol");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let id = "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: id.to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    description: Some("new".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
        let mixed = format!(
            "<PredefinedData xmlns=\"{PREDEFINED_NS}\" xmlns:xsi=\"{XSI_NS}\" xsi:type=\"CatalogPredefinedItems\" version=\"2.20\">\r\n\t<Item id=\"{id}\"><Name>Main</Name><Description>old</Description></Item>\n</PredefinedData>\r\n"
        );
        fs::write(&path, &mixed).unwrap();
        let rejected = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            std::slice::from_ref(&operation),
        )
        .err()
        .expect("mixed EOL must not choose a global fallback");
        assert!(
            rejected.diagnostics[0].message.contains("ambiguous"),
            "{:?}",
            rejected.diagnostics
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), mixed);

        let lone_cr = mixed.replace("\r\n", "\r").replace('\n', "\r");
        fs::write(&path, &lone_cr).unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let post_image = planned.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        assert!(post_image.contains(&b'\r'));
        assert!(!post_image.contains(&b'\n'));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn numeric_code_uses_effective_descriptor_post_image_for_add_and_update() {
        let root = temp_root("numeric-code");
        let descriptor = root.join("Catalogs/Items.xml");
        let add = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields {
                code: Some("643".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[add],
        )
        .unwrap();
        let first_bytes = first.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let first_text = std::str::from_utf8(first_bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        assert!(first_text.contains(r#"<Code xsi:type="xs:decimal">643</Code>"#));
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            first_text,
        )
        .unwrap();

        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, first_bytes).unwrap();
        let update = MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: None,
            fields: MetaPredefinedFields {
                code: Some("840".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let second = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[update],
        )
        .unwrap();
        let second_text = std::str::from_utf8(
            second.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(second_text.contains(r#"<Code xsi:type="xs:decimal">840</Code>"#));
        assert_eq!(second_text.matches("xsi:type=\"xs:decimal\"").count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn numeric_code_rejects_non_decimal_public_values() {
        for (index, code) in ["ABC", "1e3", " 1", "1 ", "1."].into_iter().enumerate() {
            let root = temp_root(&format!("invalid-numeric-code-{index}"));
            let descriptor = root.join("Catalogs/Items.xml");
            let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
                id: format!("a7d2e6fc-3824-4b56-b4be-{:012x}", index + 1),
                name: "Main".to_string(),
                fields: MetaPredefinedFields {
                    code: Some(code.to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
            let failure = plan_predefined_resource(
                &root,
                &descriptor,
                &owner("Catalog"),
                MetadataKind::Catalog,
                &descriptor_image(MetadataKind::Catalog, "Number"),
                &[operation],
            )
            .err()
            .expect("invalid numeric code must be rejected");
            assert_eq!(
                failure.diagnostics[0].code,
                MetaDiagnosticCode::InvalidArguments,
                "{code}"
            );
            assert_eq!(
                failure.diagnostics[0].field.as_deref(),
                Some("operations[0].elements[0].code")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn numeric_code_validation_resolves_the_xs_decimal_qname_by_namespace() {
        let alias = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:d="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code xsi:type="d:decimal">643</Code></Item></PredefinedData>"#;
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            alias,
        )
        .unwrap();
        let wrong = alias.replace(
            "http://www.w3.org/2001/XMLSchema\" xmlns:xsi",
            "urn:not-xml-schema\" xmlns:xsi",
        );
        assert!(validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            &wrong,
        )
        .is_err());
    }

    #[test]
    fn numeric_code_writer_reuses_in_scope_namespace_aliases() {
        let root = temp_root("numeric-code-aliases");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:i="{XSI_NS}" xmlns:d="{XS_NS}" i:type="CatalogPredefinedItems" version="2.20"><p:Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Name>Main</p:Name><p:Code/></p:Item></p:PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    code: Some("643".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(
            after.contains(r#"<p:Code i:type="d:decimal">643</p:Code>"#),
            "{after}"
        );
        assert!(!after.contains("xmlns:xs="), "{after}");
        assert!(!after.contains("xmlns:xsi="), "{after}");
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            unica_metadata_designer::predefined::PredefinedCodeType::Number,
            after,
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prefixed_predefined_update_preserves_qnames_and_repairs_self_closing_code() {
        for (index, code) in ["<p:Code>1</p:Code>", "<p:Code/>"].into_iter().enumerate() {
            let root = temp_root(&format!("prefixed-update-{index}"));
            let descriptor = root.join("Catalogs/Items.xml");
            let path = descriptor.with_extension("").join("Ext/Predefined.xml");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                format!(
                    r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><p:Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Name>Main</p:Name>{code}</p:Item></p:PredefinedData>"#
                ),
            )
            .unwrap();
            let operation =
                MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                    id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                    name: None,
                    fields: MetaPredefinedFields {
                        code: Some("2".to_string()),
                        description: Some("Changed".to_string()),
                        ..MetaPredefinedFields::default()
                    },
                }])
                .unwrap();
            let planned = plan_predefined_resource(
                &root,
                &descriptor,
                &owner("Catalog"),
                MetadataKind::Catalog,
                &descriptor_image(MetadataKind::Catalog, "String"),
                &[operation],
            )
            .unwrap();
            let after = std::str::from_utf8(
                planned.resources.file_mutations[0]
                    .post_image
                    .as_deref()
                    .unwrap(),
            )
            .unwrap();
            assert!(after.contains("<p:Code>2</p:Code>"), "{after}");
            assert!(
                after.contains("<p:Description>Changed</p:Description>"),
                "{after}"
            );
            assert!(!after.contains("<Code>"), "{after}");
            let data = read_predefined_items(after.as_bytes(), MetadataKind::Catalog, 10).unwrap();
            assert_eq!(data.items[0].code, "2");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn prefixed_self_closing_root_add_uses_the_root_qname_for_the_whole_item() {
        let root = temp_root("prefixed-root-add");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"/>"#
            ),
        )
        .unwrap();
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        for expected in [
            "<p:Item ",
            "<p:Name>Main</p:Name>",
            "<p:Code/>",
            "<p:Description/>",
            "<p:IsFolder>false</p:IsFolder>",
            "</p:Item>",
            "</p:PredefinedData>",
        ] {
            assert!(after.contains(expected), "missing {expected}: {after}");
        }
        read_predefined_items(after.as_bytes(), MetadataKind::Catalog, 10).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn captured_platform_fixtures_cover_all_four_owner_shapes() {
        for (kind, bytes) in [
            (
                MetadataKind::Catalog,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/Catalog.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfAccounts,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfAccounts.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfCharacteristicTypes,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfCharacteristicTypes.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfCalculationTypes,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfCalculationTypes.xml"
                ))
                .as_slice(),
            ),
        ] {
            let data = read_predefined_items(bytes, kind, 10).unwrap();
            assert_eq!(data.total, 1, "{} fixture", kind.as_str());
            if kind == MetadataKind::ChartOfAccounts {
                assert_eq!(data.items[0].order.as_deref(), Some(" 01.01"));
            }
        }
    }

    #[test]
    fn chart_of_accounts_fixture_is_an_exact_add_noop_including_order_whitespace() {
        let root = temp_root("chart-account-noop");
        let descriptor = root.join("ChartsOfAccounts/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fixture = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfAccounts.xml"
        ));
        fs::write(&path, fixture).unwrap();
        let accounting_flags = BTreeMap::from([
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.Валютный".to_string(),
                false,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.Количественный".to_string(),
                false,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.УчетПоПодразделениям".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.НалоговыйУчет".to_string(),
                true,
            ),
        ]);
        let ext_flags = BTreeMap::from([
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Суммовой".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Валютный".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Количественный"
                    .to_string(),
                true,
            ),
        ]);
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "83d798ec-929f-4f72-b78b-44e770856be4".to_string(),
            name: "ОСвОрганизации".to_string(),
            fields: MetaPredefinedFields {
                code: Some("01.01".to_string()),
                description: Some("Основные средства в организации".to_string()),
                account_type: Some(MetaPredefinedAccountType::Active),
                off_balance: Some(false),
                order: Some(" 01.01".to_string()),
                accounting_flags: Some(accounting_flags),
                ext_dimension_types: Some(vec![MetaPredefinedExtDimensionType {
                    name: "ChartOfCharacteristicTypes.ВидыСубконтоХозрасчетные.ОсновныеСредства"
                        .to_string(),
                    turnover: Some(false),
                    accounting_flags: Some(ext_flags),
                }]),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            &[operation],
        )
        .unwrap();
        assert!(planned.resources.file_mutations.is_empty());
        assert_eq!(planned.resources.exact_file_guards.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
