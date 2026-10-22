//! Existing retained-root and registered-only topology assertions.
mod tests {

    use unica_metadata::subsystem::*;
    use unica_metadata::subsystem_topology::*;
    use unica_metadata_designer::subsystem_topology::*;
    use unica_source_contract::read::SecureTreeCaptureLimits;
    use unica_workspace::read::RetainedRootSecureRead;
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    use std::cell::Cell;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::rc::Rc;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_workspace::read::{with_secure_tree_test_hook, SecureTreePhase};
    use unica_workspace::test_support::{
        create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
        FileLinkFixtureOutcome,
    };
    use uuid::Uuid;

    fn write_configuration(root: &Path, subsystem_names: &[&str]) {
        let registrations = subsystem_names
            .iter()
            .map(|name| format!("<Subsystem>{name}</Subsystem>"))
            .collect::<String>();
        fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn write_subsystem(
        root: &Path,
        ancestors: &[&str],
        name: &str,
        include: &str,
        content: &[&str],
        children: &[&str],
    ) {
        let mut directory = root.join("Subsystems");
        for ancestor in ancestors {
            directory = directory.join(ancestor).join("Subsystems");
        }
        fs::create_dir_all(&directory).unwrap();
        let content = content
            .iter()
            .map(|reference| format!(r#"<xr:Item xsi:type="xr:MDObjectRef">{reference}</xr:Item>"#))
            .collect::<String>();
        let children = children
            .iter()
            .map(|child| format!("<Subsystem>{child}</Subsystem>"))
            .collect::<String>();
        fs::write(
            directory.join(format!("{name}.xml")),
            format!(
                r#"<MetaDataObject xmlns="{MD_NS}" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Subsystem><Properties><Name>{name}</Name><IncludeInCommandInterface>{include}</IncludeInCommandInterface><Content>{content}</Content></Properties><ChildObjects>{children}</ChildObjects></Subsystem></MetaDataObject>"#
            ),
        )
        .unwrap();
    }

    fn checkpoint() -> io::Result<()> {
        Ok(())
    }

    #[test]
    fn registration_order_drives_roles_and_interface_membership() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Zeta", "Alpha"]);
        write_subsystem(root.path(), &[], "Zeta", "true", &[], &["Child"]);
        write_subsystem(
            root.path(),
            &["Zeta"],
            "Child",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        write_subsystem(root.path(), &[], "Alpha", "false", &[], &["Hidden"]);
        write_subsystem(
            root.path(),
            &["Alpha"],
            "Hidden",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );

        let source_root = root.path().canonicalize().unwrap();
        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert_eq!(
            topology
                .roots
                .iter()
                .map(|node| node.address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta", "Alpha"]
        );
        assert_eq!(
            topology
                .interface_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta", "Zeta.Child"]
        );
        assert_eq!(
            topology
                .functional_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Alpha.Hidden"]
        );
        assert_eq!(
            topology
                .interface_memberships("InformationRegister.Ledger")
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Zeta.Child"]
        );
        assert_eq!(
            topology
                .functional_memberships("InformationRegister.Ledger")
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Alpha.Hidden"]
        );
    }

    #[test]
    fn registered_dependency_paths_follow_registration_order_exactly() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Zeta", "Alpha"]);
        write_subsystem(root.path(), &[], "Zeta", "true", &[], &["Child"]);
        write_subsystem(root.path(), &["Zeta"], "Child", "true", &[], &[]);
        write_subsystem(root.path(), &[], "Alpha", "true", &[], &[]);
        fs::write(root.path().join("Subsystems/Unregistered.xml"), b"ignored").unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert_eq!(
            topology.dependency_paths(),
            [
                Path::new("Configuration.xml"),
                Path::new("Subsystems/Zeta.xml"),
                Path::new("Subsystems/Zeta/Subsystems/Child.xml"),
                Path::new("Subsystems/Alpha.xml"),
            ]
        );
    }

    #[test]
    fn content_references_are_typed_and_match_both_descriptor_identities() {
        let root = tempfile::tempdir().unwrap();
        let descriptor_uuid = Uuid::parse_str("11111111-2222-4333-8444-555555555555").unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &[
                "InformationRegister.Ledger",
                "11111111-2222-4333-8444-555555555555",
            ],
            &[],
        );
        let source_root = root.path().canonicalize().unwrap();

        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();
        let metadata_address = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "InformationRegister.Ledger",
        )
        .unwrap();
        assert_eq!(
            topology.roots[0].content,
            vec![
                ContentReference::MetadataAddress(metadata_address.clone()),
                ContentReference::Uuid(descriptor_uuid),
            ]
        );

        let target = MetadataObjectIdentity {
            address: metadata_address,
            uuid: descriptor_uuid,
        };
        assert_eq!(
            topology
                .interface_memberships_for(&target)
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Sales"]
        );

        let different_target = MetadataObjectIdentity {
            address: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "InformationRegister.Other",
            )
            .unwrap(),
            uuid: Uuid::parse_str("aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee").unwrap(),
        };
        assert!(topology
            .interface_memberships_for(&different_target)
            .is_empty());
    }

    #[test]
    fn arbitrary_nonempty_content_reference_rejects_the_topology() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &["broken-reference"],
            &[],
        );
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("an arbitrary string cannot become a third Content reference kind");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn unregistered_files_do_not_define_or_break_the_topology() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        fs::write(
            root.path().join("Subsystems/Unregistered.xml"),
            "not XML at all",
        )
        .unwrap();

        let source_root = root.path().canonicalize().unwrap();
        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert_eq!(
            topology
                .interface_addresses()
                .iter()
                .map(|address| address.as_str())
                .collect::<Vec<_>>(),
            ["Registered"]
        );
    }

    #[test]
    fn unregistered_oversized_xml_does_not_spend_the_topology_byte_budget() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        let source_root = root.path().canonicalize().unwrap();
        let expected = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();
        let unrelated = fs::File::create(root.path().join("Subsystems/Unregistered.xml")).unwrap();
        unrelated
            .set_len((SUBSYSTEM_SCAN_MAX_BYTES as u64) + 1)
            .unwrap();

        let actual = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect("an unregistered XML must not spend the registered byte budget");

        assert_eq!(actual, expected);
    }

    #[test]
    fn unregistered_file_symlink_does_not_affect_the_topology() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        fs::write(external.path().join("Unregistered.xml"), b"outside").unwrap();
        let source_root = root.path().canonicalize().unwrap();
        let expected = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();
        let outcome = create_file_link_fixture_for_test(
            external.path().join("Unregistered.xml"),
            root.path().join("Subsystems/Unregistered.xml"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a file link needs the symlink privilege",
            );
            return;
        }

        let actual = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect("an unregistered file symlink must not be classified");

        assert_eq!(actual, expected);
    }

    #[test]
    fn unregistered_directory_symlink_branch_does_not_affect_the_topology() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        fs::write(external.path().join("Outside.xml"), b"outside").unwrap();
        let source_root = root.path().canonicalize().unwrap();
        let expected = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();
        match create_directory_link_fixture_for_test(
            external.path(),
            root.path().join("Subsystems/Unregistered"),
        )
        .unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                unica_process::test_support::skip_without_host_capability(
                    unica_process::test_support::HostCapability::Symlink,
                    "creating a directory link needs the symlink privilege",
                );
                return;
            }
        }

        let actual = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect("an unregistered directory symlink branch must not be classified");

        assert_eq!(actual, expected);
    }

    #[test]
    fn registered_oversized_descriptor_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        fs::create_dir_all(root.path().join("Subsystems")).unwrap();
        let descriptor = fs::File::create(root.path().join("Subsystems/Registered.xml")).unwrap();
        descriptor
            .set_len((SUBSYSTEM_SCAN_MAX_BYTES as u64) + 1)
            .unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a registered descriptor must spend the registered byte budget");

        assert!(error.to_string().contains("byte"), "{error}");
    }

    #[test]
    fn registered_name_mismatch_is_not_complete_evidence() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        let descriptor = root.path().join("Subsystems/Registered.xml");
        let text = fs::read_to_string(&descriptor)
            .unwrap()
            .replace("<Name>Registered</Name>", "<Name>Other</Name>");
        fs::write(descriptor, text).unwrap();

        let source_root = root.path().canonicalize().unwrap();
        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("instead of `Registered`"),
            "{error}"
        );
    }

    #[test]
    fn registered_content_requires_the_platform_item_shape() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor)
            .unwrap()
            .replace("<xr:Item ", "<Item ")
            .replace("</xr:Item>", "</Item>");
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a foreign Content item cannot prove membership");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn registered_content_resolves_the_xsi_type_qname_namespace() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor).unwrap().replace(
            r#"<xr:Item xsi:type="xr:MDObjectRef">InformationRegister.Ledger</xr:Item>"#,
            r#"<readable:Item xmlns:readable="http://v8.1c.ru/8.3/xcf/readable" xmlns:xr="urn:foreign" xsi:type="xr:MDObjectRef">InformationRegister.Ledger</readable:Item>"#,
        );
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a foreign QName cannot prove typed metadata membership");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn registered_content_rejects_nested_or_mixed_value_nodes() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(
            root.path(),
            &[],
            "Sales",
            "true",
            &["InformationRegister.Ledger"],
            &[],
        );
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor).unwrap().replace(
            "InformationRegister.Ledger</xr:Item>",
            r#"InformationRegister.Ledger<foreign:Decoy xmlns:foreign="urn:foreign"/>.Ignored</xr:Item>"#,
        );
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("nested Content nodes cannot be collapsed into a scalar membership");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn configuration_registration_rejects_nested_or_mixed_value_nodes() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(root.path(), &[], "Sales", "true", &[], &[]);
        let configuration = root.path().join("Configuration.xml");
        let malformed = fs::read_to_string(&configuration).unwrap().replace(
            "<Subsystem>Sales</Subsystem>",
            r#"<Subsystem>Sales<foreign:Decoy xmlns:foreign="urn:foreign"/></Subsystem>"#,
        );
        fs::write(&configuration, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a registration must be one closed scalar value");

        assert!(error.to_string().contains("registration"), "{error}");
    }

    #[test]
    fn required_subsystem_property_rejects_nested_or_mixed_value_nodes() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(root.path(), &[], "Sales", "true", &[], &[]);
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor).unwrap().replace(
            "<Name>Sales</Name>",
            r#"<Name>Sales<foreign:Decoy xmlns:foreign="urn:foreign"/></Name>"#,
        );
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a required property must be one closed scalar value");

        assert!(error.to_string().contains("Name"), "{error}");
    }

    #[test]
    fn metadata_root_rejects_a_foreign_direct_artifact() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(root.path(), &[], "Sales", "true", &[], &[]);
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor).unwrap().replace(
            "</MetaDataObject>",
            r#"<foreign:Decoy xmlns:foreign="urn:foreign"/></MetaDataObject>"#,
        );
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("a foreign direct artifact makes the object identity ambiguous");

        assert!(error.to_string().contains("exactly one"), "{error}");
    }

    #[test]
    fn content_container_rejects_non_whitespace_direct_text() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(root.path(), &[], "Sales", "true", &[], &[]);
        let descriptor = root.path().join("Subsystems/Sales.xml");
        let malformed = fs::read_to_string(&descriptor)
            .unwrap()
            .replace("<Content></Content>", "<Content>Catalog.Goods</Content>");
        fs::write(&descriptor, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("raw Content text is not a typed metadata reference");

        assert!(error.to_string().contains("Content"), "{error}");
    }

    #[test]
    fn child_objects_container_rejects_non_whitespace_direct_text() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Sales"]);
        write_subsystem(root.path(), &[], "Sales", "true", &[], &[]);
        let configuration = root.path().join("Configuration.xml");
        let malformed = fs::read_to_string(&configuration)
            .unwrap()
            .replace("<Subsystem>Sales</Subsystem>", "Sales");
        fs::write(&configuration, malformed).unwrap();
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint)
            .expect_err("raw ChildObjects text is not a subsystem registration");

        assert!(error.to_string().contains("ChildObjects"), "{error}");
    }

    #[test]
    fn registered_boolean_must_be_present_once_and_canonical() {
        for (case, replacement) in [
            ("missing", ""),
            (
                "duplicate",
                "<IncludeInCommandInterface>true</IncludeInCommandInterface><IncludeInCommandInterface>false</IncludeInCommandInterface>",
            ),
            (
                "invalid",
                "<IncludeInCommandInterface>True</IncludeInCommandInterface>",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            write_configuration(root.path(), &["Registered"]);
            write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
            let descriptor = root.path().join("Subsystems/Registered.xml");
            let text = fs::read_to_string(&descriptor).unwrap().replace(
                "<IncludeInCommandInterface>true</IncludeInCommandInterface>",
                replacement,
            );
            fs::write(descriptor, text).unwrap();

            let source_root = root.path().canonicalize().unwrap();
            let error = capture_registered_subsystem_topology(&source_root, checkpoint)
                .expect_err(case);
            assert!(
                error.to_string().contains("IncludeInCommandInterface"),
                "{case}: {error}"
            );
        }
    }

    #[test]
    fn missing_malformed_and_duplicate_registered_nodes_are_rejected() {
        let missing = tempfile::tempdir().unwrap();
        write_configuration(missing.path(), &["Missing"]);
        let source_root = missing.path().canonicalize().unwrap();
        assert!(capture_registered_subsystem_topology(&source_root, checkpoint).is_err());

        let malformed = tempfile::tempdir().unwrap();
        write_configuration(malformed.path(), &["Broken"]);
        fs::create_dir_all(malformed.path().join("Subsystems")).unwrap();
        fs::write(malformed.path().join("Subsystems/Broken.xml"), "<broken>").unwrap();
        let source_root = malformed.path().canonicalize().unwrap();
        assert!(capture_registered_subsystem_topology(&source_root, checkpoint).is_err());

        let duplicate = tempfile::tempdir().unwrap();
        write_configuration(duplicate.path(), &["Same", "Same"]);
        write_subsystem(duplicate.path(), &[], "Same", "true", &[], &[]);
        let source_root = duplicate.path().canonicalize().unwrap();
        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();
        assert!(error
            .to_string()
            .contains("duplicate subsystem registration"));
    }

    #[test]
    fn empty_registration_proves_an_empty_topology_without_a_subsystems_directory() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &[]);
        let source_root = root.path().canonicalize().unwrap();

        let topology = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap();

        assert!(topology.roots.is_empty());
        assert!(topology.functional_addresses().is_empty());
        assert!(topology.interface_addresses().is_empty());
    }

    #[test]
    fn ninth_registered_level_exceeds_the_shared_address_budget() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["A"]);
        let names = ["A", "B", "C", "D", "E", "F", "G", "H", "I"];
        for (index, name) in names.iter().enumerate() {
            let children = names
                .get(index + 1)
                .copied()
                .into_iter()
                .collect::<Vec<_>>();
            write_subsystem(root.path(), &names[..index], name, "true", &[], &children);
        }
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("depth") || error.to_string().contains("1 to 8"),
            "{error}"
        );
    }

    #[test]
    fn complete_result_requires_a_checkpoint_after_secure_capture_and_parsing() {
        let root = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Registered"]);
        write_subsystem(root.path(), &[], "Registered", "true", &[], &[]);
        let source_root = root.path().canonicalize().unwrap();
        let identity_proofs_completed = Rc::new(Cell::new(false));
        let phase_flag = Rc::clone(&identity_proofs_completed);

        let error = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterFinalIdentityProofs {
                    phase_flag.set(true);
                }
            },
            || {
                capture_registered_subsystem_topology(&source_root, || {
                    if identity_proofs_completed.get() {
                        Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                    } else {
                        Ok(())
                    }
                })
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("cancelled"), "{error}");
    }

    #[test]
    fn registered_descriptor_symlink_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        write_configuration(root.path(), &["Linked"]);
        write_subsystem(external.path(), &[], "Linked", "true", &[], &[]);
        fs::create_dir_all(root.path().join("Subsystems")).unwrap();
        let outcome = create_file_link_fixture_for_test(
            external.path().join("Subsystems/Linked.xml"),
            root.path().join("Subsystems/Linked.xml"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a file link needs the symlink privilege",
            );
            return;
        }
        let source_root = root.path().canonicalize().unwrap();

        let error = capture_registered_subsystem_topology(&source_root, checkpoint).unwrap_err();

        assert!(
            error.to_string().contains("symbolic links")
                || error.to_string().contains("reparse point")
                || error.to_string().contains("missing"),
            "{error}"
        );
    }

    fn capture_registered_subsystem_topology(
        source_root: &Path,
        mut checkpoint: impl FnMut() -> io::Result<()>,
    ) -> Result<SubsystemTopology, SubsystemTopologyError> {
        let reader = RetainedRootSecureRead::open(
            source_root,
            SecureTreeCaptureLimits {
                maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2,
                maximum_entries: SUBSYSTEM_SCAN_MAX_ENTRIES,
                maximum_files: SUBSYSTEM_SCAN_MAX_FILES,
                maximum_bytes: SUBSYSTEM_SCAN_MAX_BYTES,
            },
            &mut checkpoint,
        )?;
        read_registered_subsystem_topology(Box::new(reader), &mut checkpoint)
    }
}
