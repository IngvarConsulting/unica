use super::test_support::*;
use super::*;
use crate::platform::testing;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub fn canonical_registrations_accumulate_and_preserve_source_bytes(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("canonical");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    assert_eq!(
        transaction
            .register_canonical_child(&config, "Role", "Reader", registrar)
            .expect("role registration must plan"),
        RegistrationStatus::Added
    );
    assert_eq!(
        transaction
            .register_canonical_child(&config, "Subsystem", "Core", registrar)
            .expect("subsystem registration must plan"),
        RegistrationStatus::Added
    );
    assert_eq!(
        transaction
            .register_canonical_child(&config, "Role", "Reader", registrar)
            .expect("duplicate must be detected"),
        RegistrationStatus::AlreadyPresent
    );

    let diffs = transaction.registration_diffs();
    assert_eq!(diffs.len(), 1);
    let diff = &diffs[0];
    let mut reconstructed = original.clone();
    reconstructed.splice(diff.byte_range.clone(), diff.after.clone());
    assert_eq!(transaction.planned_updated_paths(), vec![config.clone()]);

    let report = transaction.commit().expect("transaction must commit");
    assert!(report.created.is_empty());
    assert_eq!(report.updated, vec![config.clone()]);
    assert!(report.cleanup_warnings.is_empty());
    let actual = fs::read(&config).expect("configuration must remain readable");
    assert_eq!(actual, reconstructed);
    assert!(actual.starts_with(UTF8_BOM));
    assert_no_bare_lf(&actual);
    let text = String::from_utf8(actual).expect("configuration must be UTF-8");
    assert!(text.contains("xmlns:cfg=\"urn:kept-only-as-qname\""));
    assert!(text.contains("xsi:type=\"cfg:MetaDataObject\""));
    assert!(text.ends_with("</MetaDataObject><!--tail stays exact-->"));
    let subsystem = text.find("<Subsystem>Core</Subsystem>").unwrap();
    let role = text.find("<Role>Reader</Role>").unwrap();
    let catalog = text.find("<Catalog>Items</Catalog>").unwrap();
    assert!(subsystem < role && role < catalog);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn canonical_registrations_accumulate_across_path_aliases(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("canonical-path-alias");
    let detour = root.join("detour");
    fs::create_dir(&detour).expect("detour directory must be created");
    let config = root.join("Configuration.xml");
    let config_alias = detour.join("..").join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    assert_eq!(
        transaction
            .register_canonical_child(&config_alias, "Role", "Reader", registrar)
            .expect("aliased role registration must plan"),
        RegistrationStatus::Added
    );
    assert_eq!(
        transaction
            .register_canonical_child(&config, "Subsystem", "Core", registrar)
            .expect("canonical subsystem registration must join the same plan"),
        RegistrationStatus::Added
    );
    assert_eq!(transaction.registration_diffs().len(), 1);

    transaction
        .commit()
        .expect("aliased transaction must commit");
    let published = fs::read_to_string(&config).expect("published config must be readable");
    assert!(published.contains("<Role>Reader</Role>"), "{published}");
    assert!(
        published.contains("<Subsystem>Core</Subsystem>"),
        "{published}"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn missing_registration_target_is_explicit_and_guards_normalized_absence(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("missing-target");
    let detour = root.join("detour");
    fs::create_dir(&detour).expect("detour directory must be created");
    let target = root.join("Configuration.xml");
    let target_alias = detour.join("..").join("Configuration.xml");
    let mut transaction = CompileTransaction::new();
    assert_eq!(
        transaction
            .register_canonical_child(&target_alias, "Role", "Reader", registrar)
            .expect("missing target is an allowed status"),
        RegistrationStatus::MissingTarget
    );
    assert!(!transaction.is_empty());
    assert!(transaction
        .protects_path(&target)
        .expect("absence identity must normalize"));
    assert!(transaction.registration_diffs().is_empty());
    let report = transaction
        .commit()
        .expect("unchanged absent target must satisfy the guard");
    assert!(report.created.is_empty());
    assert!(report.updated.is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn missing_registration_target_appearance_rolls_back_created_outputs(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("missing-target-appears");
    let config = root.join("Configuration.xml");
    let output = root.join("Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_utf8_bom_text(
            &output,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .expect("role output must plan");
    assert_eq!(
        transaction
            .register_canonical_child(&config, "Role", "Reader", registrar)
            .expect("missing target is an allowed status"),
        RegistrationStatus::MissingTarget
    );
    let config_for_hook = config.clone();
    let supported_config = configuration_bytes();

    let error = with_before_commit_hook(
        move |_| fs::write(&config_for_hook, &supported_config).unwrap(),
        || transaction.commit(),
    )
    .expect_err("late owner appearance must abort the transaction");

    assert!(error.contains("absence guard"), "{error}");
    assert!(config.is_file());
    assert!(!output.exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn self_closing_registration_keeps_a_bom_free_lf_source_bom_free(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("bom-free-self-closing");
    let config = root.join("Configuration.xml");
    let original = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>"
    );
    fs::write(&config, original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    assert_eq!(
        transaction
            .register_canonical_child(&config, "Role", "Reader", registrar)
            .expect("registration must plan"),
        RegistrationStatus::Added
    );
    transaction.commit().expect("transaction must commit");

    let actual = fs::read(&config).expect("configuration must remain readable");
    assert!(!actual.starts_with(UTF8_BOM));
    assert!(String::from_utf8(actual)
        .unwrap()
        .contains("<ChildObjects>\n\t<Role>Reader</Role>\n</ChildObjects>"));
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn appended_registration_preserves_cr_only_line_boundaries(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("cr-only-append");
    let config = root.join("Configuration.xml");
    fs::write(
        &config,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r",
            "<MetaDataObject>\r",
            "\t<Configuration>\r",
            "\t\t<ChildObjects>\r",
            "\t\t\t<Catalog>Items</Catalog>\r",
            "\t\t</ChildObjects>\r",
            "\t</Configuration>\r",
            "</MetaDataObject>"
        ),
    )
    .expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    assert_eq!(
        transaction
            .register_canonical_child(&config, "Document", "Orders", registrar)
            .expect("registration must plan"),
        RegistrationStatus::Added
    );
    transaction.commit().expect("transaction must commit");

    let actual = String::from_utf8(fs::read(&config).expect("configuration must be readable"))
        .expect("configuration must remain UTF-8");
    assert!(
        actual.contains(concat!(
            "\t\t\t<Catalog>Items</Catalog>\r",
            "\t\t\t<Document>Orders</Document>\r",
            "\t\t</ChildObjects>"
        )),
        "{actual:?}"
    );
    assert!(!actual.contains("\r\r\t\t</ChildObjects>"), "{actual:?}");
    assert!(!actual.contains('\n'));
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn already_present_registration_commits_as_a_byte_for_byte_noop(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("registration-noop");
    let config = root.join("Configuration.xml");
    let original = concat!(
        "<?xml version=\"1.0\"?>\r\n",
        "<MetaDataObject><Configuration><ChildObjects>\r\n",
        "\t<Role>Reader</Role>\r\n",
        "</ChildObjects></Configuration></MetaDataObject>"
    )
    .as_bytes()
    .to_vec();
    fs::write(&config, &original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    assert_eq!(
        transaction
            .register_canonical_child(&config, "Role", "Reader", registrar)
            .expect("duplicate registration must plan"),
        RegistrationStatus::AlreadyPresent
    );
    assert!(transaction.is_empty());
    let report = transaction.commit().expect("no-op transaction must commit");

    assert!(report.created.is_empty());
    assert!(report.updated.is_empty());
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn existing_target_without_child_objects_is_rejected_without_mutation(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("missing-child-objects");
    let config = root.join("Configuration.xml");
    let original = b"<?xml version=\"1.0\"?><MetaDataObject><Configuration/></MetaDataObject>";
    fs::write(&config, original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect_err("missing ChildObjects must fail");

    assert!(error.contains("No <ChildObjects>"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), original);
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn commit_creates_bom_text_and_updates_registration_once(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("success");
    let config = root.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let object = root.join("Roles/Reader.xml");
    let rights = root.join("Roles/Reader/Ext/Rights.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_utf8_bom_text(
            &object,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction
        .create_utf8_bom_text(
            &rights,
            "<?xml version=\"1.0\"?><Rights xmlns=\"http://v8.1c.ru/8.2/roles\"/>",
        )
        .unwrap();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .unwrap();

    let report = transaction.commit().expect("transaction must commit");

    assert_eq!(report.created, vec![object.clone(), rights.clone()]);
    assert_eq!(report.updated, vec![config.clone()]);
    assert!(fs::read(&object).unwrap().starts_with(UTF8_BOM));
    assert!(fs::read(&rights).unwrap().starts_with(UTF8_BOM));
    assert!(fs::read_to_string(&config)
        .unwrap()
        .contains("<Role>Reader</Role>"));
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn after_object_files_failure_removes_files_and_created_directories(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-objects");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let object = root.join("Deep/Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_utf8_bom_text(
            &object,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .unwrap();

    let error = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || transaction.commit())
        .expect_err("failpoint must abort commit");

    assert!(error.contains("after object files"), "{error}");
    assert!(!object.exists());
    assert!(!root.join("Deep").exists());
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn post_write_validation_failure_restores_exact_registration_bytes(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-validation");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let object = root.join("Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_utf8_bom_text(
            &object,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .unwrap();

    let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        transaction.commit()
    })
    .expect_err("failpoint must abort commit");

    assert!(error.contains("post-write validation"), "{error}");
    assert!(!object.exists());
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn post_validation_rollback_restores_bytes_and_unix_mode_0600(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-mode");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    if !testing::set_unix_mode_for_test(&config, 0o600).expect("mode fixture must be configurable")
    {
        fs::remove_dir_all(root).expect("temporary root must be removed");
        return;
    }
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        transaction.commit()
    })
    .expect_err("post-validation failpoint must roll the transaction back");

    assert!(error.contains("post-write validation"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), original);
    assert_eq!(
        testing::unix_mode_for_test(&config).expect("mode must remain readable"),
        Some(0o600)
    );
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn registration_target_remains_present_after_backup_preparation(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("recovery-keeps-target-present");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let (recovery_ready_sender, recovery_ready_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let commit_thread = thread::spawn(move || {
        with_registration_recovery_pause(recovery_ready_sender, release_receiver, || {
            transaction.commit()
        })
    });

    recovery_ready_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("transaction must reach recovery preparation without hanging");
    let target_present = fs::symlink_metadata(&config).is_ok();
    let bytes_during_recovery = fs::read(&config);
    release_sender
        .send(())
        .expect("commit thread must remain available for release");

    let commit_result = commit_thread.join().expect("commit thread must not panic");
    assert!(
        target_present,
        "the target entry must remain present while recovery is ready"
    );
    assert_eq!(bytes_during_recovery.unwrap(), original);
    let report = commit_result.expect("transaction must commit after the pause");
    assert_eq!(report.updated, vec![config.clone()]);
    assert!(fs::read_to_string(&config)
        .unwrap()
        .contains("<Role>Reader</Role>"));
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn pending_registration_cleanup_preserves_same_name_decoys_after_parent_swap(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    if !testing::can_rename_parent_with_retained_cleanup_child_for_test() {
        return;
    }
    let root = temp_root("pending-recovery-parent-swap");
    let active_parent = root.join("active");
    let preserved_parent = root.join("preserved");
    fs::create_dir(&active_parent).expect("active parent must be created");
    let config = active_parent.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let (recovery_ready_sender, recovery_ready_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let commit_thread = thread::spawn(move || {
        with_commit_failpoint(CommitFailpoint::AfterRegistrationBackup, || {
            with_registration_recovery_pause(recovery_ready_sender, release_receiver, || {
                transaction.commit()
            })
        })
    });

    recovery_ready_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("transaction must reach recovery preparation without hanging");
    let entries = fs::read_dir(&active_parent)
        .expect("prepared publication entries must be readable")
        .map(|entry| entry.expect("prepared entry must be readable").file_name())
        .collect::<Vec<_>>();
    let stage_name = entries
        .iter()
        .find(|name| name.to_string_lossy().contains(".unica-stage-"))
        .expect("prepared stage must exist")
        .clone();
    let recovery_name = entries
        .iter()
        .find(|name| name.to_string_lossy().contains(".unica-recovery-"))
        .expect("prepared recovery directory must exist")
        .clone();

    fs::rename(&active_parent, &preserved_parent).expect("prepared parent must be displaced");
    fs::create_dir(&active_parent).expect("replacement parent must be created");
    let decoy_stage = active_parent.join(&stage_name);
    let decoy_recovery_directory = active_parent.join(&recovery_name);
    let decoy_recovery = decoy_recovery_directory.join("original");
    fs::write(&decoy_stage, b"same-name stage decoy").expect("stage decoy must be written");
    fs::create_dir(&decoy_recovery_directory).expect("recovery decoy directory must be created");
    fs::write(&decoy_recovery, b"same-name recovery decoy")
        .expect("recovery decoy must be written");
    release_sender
        .send(())
        .expect("commit thread must remain available for release");

    let error = commit_thread
        .join()
        .expect("commit thread must not panic")
        .expect_err("backup failpoint must abort publication");

    assert!(error.contains("after registration backup"), "{error}");
    assert!(error.contains("identity changed"), "{error}");
    assert!(
        error.contains(&stage_name.to_string_lossy().to_string()),
        "stage cleanup warning is missing: {error}"
    );
    assert!(
        error.contains(&recovery_name.to_string_lossy().to_string()),
        "recovery cleanup warning is missing: {error}"
    );
    assert_eq!(
        fs::read(&decoy_stage).expect("cleanup must preserve the stage decoy"),
        b"same-name stage decoy"
    );
    assert_eq!(
        fs::read(&decoy_recovery).expect("cleanup must preserve the recovery decoy"),
        b"same-name recovery decoy"
    );
    assert!(preserved_parent.join(&stage_name).is_file());
    assert!(preserved_parent
        .join(&recovery_name)
        .join("original")
        .is_file());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn precommit_retry_finishes_the_recovery_directory_after_file_cleanup_recovers(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("pending-recovery-retry-finishes-directory");
    let config = root.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let displaced_recovery = Arc::new(Mutex::new(None));
    let displaced_for_first_cleanup = Arc::clone(&displaced_recovery);
    let displaced_for_retry = Arc::clone(&displaced_recovery);
    let root_for_first_cleanup = root.clone();

    let error = with_before_cleanup_retry_hook(
        move || {
            let (recovery_path, displaced_path) = displaced_for_retry
                .lock()
                .expect("recovery-path state must remain readable")
                .take()
                .expect("the first cleanup must displace the recovery file");
            assert_eq!(
                fs::read(&recovery_path).expect("same-name decoy must remain readable"),
                b"same-name recovery decoy"
            );
            fs::remove_file(&recovery_path).expect("same-name decoy must be removed");
            fs::rename(&displaced_path, &recovery_path)
                .expect("the proven recovery route must be restored before automatic retry");
        },
        || {
            with_before_bound_cleanup_hook(
                move |stage_path| {
                    assert!(
                        stage_path
                            .file_name()
                            .is_some_and(|name| name.to_string_lossy().contains(".unica-stage-")),
                        "the first bound cleanup must discard the prepared stage: {}",
                        stage_path.display()
                    );
                    let recovery_directory = fs::read_dir(&root_for_first_cleanup)
                        .expect("prepared recovery directory must be readable")
                        .filter_map(Result::ok)
                        .find(|entry| {
                            entry
                                .file_name()
                                .to_string_lossy()
                                .contains(".unica-recovery-")
                        })
                        .map(|entry| entry.path())
                        .expect("prepared recovery directory must exist");
                    let recovery_path = recovery_directory.join("original");
                    let displaced_path = recovery_directory.join("displaced-original");
                    fs::rename(&recovery_path, &displaced_path)
                        .expect("the proven recovery must be displaced");
                    fs::write(&recovery_path, b"same-name recovery decoy")
                        .expect("same-name recovery decoy must be written");
                    *displaced_for_first_cleanup
                        .lock()
                        .expect("recovery-path state must remain writable") =
                        Some((recovery_path, displaced_path));
                },
                || {
                    with_commit_failpoint(CommitFailpoint::AfterRegistrationBackup, || {
                        transaction.commit()
                    })
                },
            )
        },
    )
    .expect_err("backup failpoint must abort publication");

    assert!(error.contains("after registration backup"), "{error}");
    assert!(
        error.contains("failed to remove pending registration recovery"),
        "the first identity mismatch must remain visible: {error}"
    );
    assert_eq!(fs::read(&config).unwrap(), configuration_bytes());
    assert!(
            transaction_debris(&root).is_empty(),
            "automatic file cleanup retry must retain enough guard state to remove the now-empty recovery directory: {:?}",
            transaction_debris(&root)
        );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    if !testing::can_rename_parent_with_retained_cleanup_child_for_test() {
        return;
    }
    let root = temp_root("finalize-recovery-parent-swap");
    let active_parent = root.join("active");
    let preserved_parent = root.join("preserved");
    fs::create_dir(&active_parent).expect("active parent must be created");
    let config = active_parent.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let active_for_validation = active_parent.clone();
    let preserved_for_validation = preserved_parent.clone();
    let (recovery_sender, recovery_receiver) = mpsc::channel();

    let report = transaction
        .commit_with_post_validation(move || {
            let recovery_name = fs::read_dir(&active_for_validation)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .find(|name| name.to_string_lossy().contains(".unica-recovery-"))
                .ok_or_else(|| "prepared recovery directory is missing".to_string())?;
            fs::rename(&active_for_validation, &preserved_for_validation)
                .map_err(|error| error.to_string())?;
            fs::create_dir(&active_for_validation).map_err(|error| error.to_string())?;
            let decoy_directory = active_for_validation.join(&recovery_name);
            fs::create_dir(&decoy_directory).map_err(|error| error.to_string())?;
            fs::write(decoy_directory.join("original"), b"finalize recovery decoy")
                .map_err(|error| error.to_string())?;
            recovery_sender
                .send(recovery_name)
                .map_err(|error| error.to_string())?;
            Ok(())
        })
        .expect("published bytes remain valid despite deferred cleanup warning");
    let recovery_name = recovery_receiver
        .recv()
        .expect("post-validation must report its recovery name");
    let decoy_recovery = active_parent.join(&recovery_name).join("original");

    assert!(
        report
            .cleanup_warnings
            .iter()
            .any(|warning| warning.contains("identity changed")
                && warning.contains(&recovery_name.to_string_lossy().to_string())),
        "{:?}",
        report.cleanup_warnings
    );
    assert!(
        fs::read_to_string(preserved_parent.join("Configuration.xml"))
            .expect("published configuration must remain readable")
            .contains("<Role>Reader</Role>"),
        "finalize cleanup failure must not roll back committed bytes"
    );
    assert_eq!(
        fs::read(&decoy_recovery).expect("finalize must preserve the recovery decoy"),
        b"finalize recovery decoy"
    );
    assert!(
        preserved_parent
            .join(recovery_name)
            .join("original")
            .is_file(),
        "the identity-bound recovery must remain in its displaced parent"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn successful_registration_finalize_does_not_recreate_recovery_after_directory_cleanup_failure(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("finalize-partial-recovery-cleanup");
    let config = root.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let root_for_validation = root.clone();
    let (recovery_sender, recovery_receiver) = mpsc::channel();

    let report = transaction
        .commit_with_post_validation(move || {
            let recovery_directory = fs::read_dir(&root_for_validation)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains(".unica-recovery-")
                })
                .map(|entry| entry.path())
                .ok_or_else(|| "prepared recovery directory is missing".to_string())?;
            fs::write(recovery_directory.join("blocker"), b"retain directory")
                .map_err(|error| error.to_string())?;
            recovery_sender
                .send(recovery_directory)
                .map_err(|error| error.to_string())?;
            Ok(())
        })
        .expect("directory cleanup failure must remain a post-commit warning");
    let recovery_directory = recovery_receiver
        .recv()
        .expect("post-validation must report the recovery directory");

    assert!(
        report
            .cleanup_warnings
            .iter()
            .any(|warning| warning.contains("recovery directory")),
        "{:?}",
        report.cleanup_warnings
    );
    assert_eq!(
        fs::read(recovery_directory.join("blocker")).unwrap(),
        b"retain directory"
    );
    assert!(
        !recovery_directory.join("original").exists(),
        "a committed transaction must not recreate a removed rollback file"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn compile_transaction_rejects_readonly_registration_without_partial_creates(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("readonly-preflight");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    if !testing::set_unix_mode_for_test(&config, 0o400).expect("mode fixture must be configurable")
    {
        let mut permissions = fs::metadata(&config).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&config, permissions).unwrap();
    }
    let original_mode = testing::unix_mode_for_test(&config).unwrap();
    let object = root.join("Deep/Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_text(&object, "<Object/>")
        .expect("create must plan");
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let error = transaction
        .commit()
        .expect_err("read-only registration must reject the complete transaction");

    assert!(error.contains("read-only"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), original);
    assert_eq!(testing::unix_mode_for_test(&config).unwrap(), original_mode);
    assert!(!object.exists());
    assert!(!root.join("Deep").exists());
    assert!(transaction_debris(&root).is_empty());
    prepare_file_for_removal(&config).expect("fixture must be removable");
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn compile_transaction_rejects_hard_linked_registration_without_mutation(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("hard-link-preflight");
    let config = root.join("Configuration.xml");
    let alias = root.join("Configuration.alias.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    fs::hard_link(&config, &alias).expect("hard-link fixture must be created");
    let object = root.join("Deep/Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_text(&object, "<Object/>")
        .expect("create must plan");
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");

    let error = transaction
        .commit()
        .expect_err("hard-linked registration must reject the complete transaction");

    assert!(error.contains("hard links"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), original);
    assert_eq!(fs::read(&alias).unwrap(), original);
    assert_eq!(
        crate::platform::filesystem::hard_link_count(&fs::File::open(&config).unwrap()).unwrap(),
        2
    );
    assert!(!object.exists());
    assert!(!root.join("Deep").exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn post_validation_failure_rolls_back_two_registrations_and_one_create(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-two-registrations");
    let config_a = root.join("Configuration.xml");
    let config_b = root.join("Subsystems/Core.xml");
    fs::create_dir_all(config_b.parent().unwrap()).expect("fixture parent must be created");
    let original_a = configuration_bytes();
    let original_b = configuration_bytes();
    fs::write(&config_a, &original_a).expect("first fixture must be written");
    fs::write(&config_b, &original_b).expect("second fixture must be written");
    let modes_supported = testing::set_unix_mode_for_test(&config_a, 0o600)
        .and_then(|supported| {
            testing::set_unix_mode_for_test(&config_b, 0o640)
                .map(|second_supported| supported && second_supported)
        })
        .expect("mode fixtures must be configurable");
    let object = root.join("Deep/Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_text(&object, "<Object/>")
        .expect("create must plan");
    transaction
        .register_canonical_child(&config_a, "Role", "Reader", registrar)
        .expect("first registration must plan");
    transaction
        .register_canonical_child(&config_b, "Catalog", "Orders", registrar)
        .expect("second registration must plan");

    let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        transaction.commit()
    })
    .expect_err("post-validation failure must roll every publication back");

    assert!(error.contains("post-write validation"), "{error}");
    assert_eq!(fs::read(&config_a).unwrap(), original_a);
    assert_eq!(fs::read(&config_b).unwrap(), original_b);
    if modes_supported {
        assert_eq!(testing::unix_mode_for_test(&config_a).unwrap(), Some(0o600));
        assert_eq!(testing::unix_mode_for_test(&config_b).unwrap(), Some(0o640));
    }
    assert!(!object.exists());
    assert!(!root.join("Deep").exists());
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn rollback_preserves_concurrent_registration_replacement_and_original_recovery(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-registration-concurrent-replacement");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    let guard = root.join("Decision.bsl");
    fs::write(&config, &original).unwrap();
    fs::write(&guard, b"stable").unwrap();
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .unwrap();
    let diff = transaction.registration_diffs().pop().unwrap();
    let mut concurrent = original.clone();
    concurrent.splice(diff.byte_range, diff.after);
    transaction.guard_exact_preimage(&guard, b"stable").unwrap();
    let config_for_hook = config.clone();
    let root_for_hook = root.clone();
    let concurrent_for_hook = concurrent.clone();

    let error = with_before_rollback_mutation_hook(
        move |path| {
            assert_eq!(path, config_for_hook);
            let external = root_for_hook.join("external-registration.xml");
            fs::write(&external, &concurrent_for_hook).unwrap();
            replace_file_atomically(&external, &config_for_hook).unwrap();
        },
        || {
            transaction.commit_with_post_validation(|| {
                fs::write(&guard, b"changed").unwrap();
                Ok(())
            })
        },
    )
    .expect_err("late guard failure must not overwrite a concurrent replacement");

    assert!(error.contains("read guard"), "{error}");
    assert!(error.contains("rollback conflict"), "{error}");
    assert!(error.contains("recovery is preserved"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), concurrent);
    let recovery_directory = transaction_debris(&root)
        .into_iter()
        .find(|path| path.is_dir())
        .expect("unsafe registration rollback must preserve its recovery directory");
    assert_eq!(
        fs::read(recovery_directory.join("original")).unwrap(),
        original
    );
    fs::remove_dir_all(root).unwrap();
}

pub fn registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    if !testing::can_rename_parent_with_retained_cleanup_child_for_test() {
        return;
    }
    let root = temp_root("rollback-recovery-parent-swap");
    let active_parent = root.join("active");
    let preserved_parent = root.join("preserved");
    fs::create_dir(&active_parent).expect("active parent must be created");
    let config = active_parent.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let active_for_hook = active_parent.clone();
    let preserved_for_hook = preserved_parent.clone();
    let config_for_hook = config.clone();
    let original_for_hook = original.clone();
    let (recovery_sender, recovery_receiver) = mpsc::channel();

    let error = with_before_rollback_mutation_hook(
        move |path| {
            assert_eq!(path, config_for_hook);
            let recovery_name = fs::read_dir(&active_for_hook)
                .expect("published entries must be readable")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .find(|name| name.to_string_lossy().contains(".unica-recovery-"))
                .expect("published recovery directory must exist");
            fs::rename(&active_for_hook, &preserved_for_hook)
                .expect("published parent must be displaced");
            fs::create_dir(&active_for_hook).expect("replacement parent must be created");
            fs::hard_link(
                preserved_for_hook.join("Configuration.xml"),
                active_for_hook.join("Configuration.xml"),
            )
            .expect("published identity must be rebound through the old route");
            fs::remove_file(preserved_for_hook.join("Configuration.xml"))
                .expect("old published name must be released");
            let decoy_directory = active_for_hook.join(&recovery_name);
            fs::create_dir(&decoy_directory).expect("recovery decoy must be created");
            fs::write(decoy_directory.join("original"), &original_for_hook)
                .expect("same-name recovery decoy must be written");
            recovery_sender
                .send(recovery_name)
                .expect("recovery name must be reported");
        },
        || {
            with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                transaction.commit()
            })
        },
    )
    .expect_err("post-validation failure must roll publication back");
    let recovery_name = recovery_receiver
        .recv()
        .expect("rollback hook must report its recovery name");
    let decoy_recovery = active_parent.join(&recovery_name).join("original");

    assert!(error.contains("rollback encountered:"), "{error}");
    assert!(error.contains("identity changed"), "{error}");
    assert!(
        fs::read_to_string(&config)
            .expect("published target must remain readable")
            .contains("<Role>Reader</Role>"),
        "rollback must not mutate the published target after recovery identity loss"
    );
    assert_eq!(
        fs::read(&decoy_recovery).expect("rollback must preserve the recovery decoy"),
        original
    );
    assert!(
        preserved_parent
            .join(recovery_name)
            .join("original")
            .is_file(),
        "the proven recovery must remain in the displaced parent"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn registration_rollback_cleanup_preserves_same_name_quarantine_decoy(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-quarantine-identity-swap");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let (paths_sender, paths_receiver) = mpsc::channel();

    let error = with_before_rollback_quarantine_cleanup_hook(
        move |quarantined| {
            let displaced = quarantined.with_file_name("displaced-published");
            fs::rename(quarantined, &displaced)
                .expect("owned quarantine must be displaced before cleanup");
            fs::write(quarantined, b"same-name quarantine decoy")
                .expect("same-name quarantine decoy must be written");
            paths_sender
                .send((quarantined.to_path_buf(), displaced))
                .expect("quarantine paths must be reported");
        },
        || {
            with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                transaction.commit()
            })
        },
    )
    .expect_err("post-validation failure must roll publication back");
    let (quarantined, displaced) = paths_receiver
        .recv()
        .expect("rollback cleanup hook must report quarantine paths");

    assert!(error.contains("post-write validation"), "{error}");
    assert_eq!(fs::read(&config).unwrap(), original);
    assert_eq!(
        fs::read(&quarantined).expect("cleanup must preserve the quarantine decoy"),
        b"same-name quarantine decoy"
    );
    assert!(fs::read_to_string(&displaced)
        .expect("cleanup must preserve the displaced published bytes")
        .contains("<Role>Reader</Role>"));
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn registration_rollback_restore_cleanup_preserves_same_name_quarantine_decoy(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("registration-rollback-restore-cleanup-identity-swap");
    let config = root.join("Configuration.xml");
    let guard = root.join("Decision.bsl");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("registration fixture must be written");
    fs::write(&guard, b"stable").expect("guard fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let diff = transaction
        .registration_diffs()
        .pop()
        .expect("registration diff must exist");
    let mut concurrent = original.clone();
    concurrent.splice(diff.byte_range, diff.after);
    transaction
        .guard_exact_preimage(&guard, b"stable")
        .expect("guard must plan");
    let config_for_mutation = config.clone();
    let root_for_mutation = root.clone();
    let concurrent_for_mutation = concurrent.clone();
    let (paths_sender, paths_receiver) = mpsc::channel();

    let error = with_before_rollback_quarantine_restore_rename_hook(
        move |quarantined, _target| {
            let displaced = quarantined.with_file_name("displaced-concurrent-registration");
            fs::rename(quarantined, &displaced)
                .expect("owned quarantine link must be displaced before cleanup");
            fs::write(quarantined, b"same-name quarantine decoy")
                .expect("same-name quarantine decoy must be written");
            paths_sender
                .send((quarantined.to_path_buf(), displaced))
                .expect("quarantine paths must be reported");
        },
        || {
            with_before_rollback_mutation_hook(
                move |path| {
                    assert_eq!(path, config_for_mutation);
                    let external = root_for_mutation.join("external-registration.xml");
                    fs::write(&external, &concurrent_for_mutation)
                        .expect("concurrent registration must be written");
                    replace_file_atomically(&external, &config_for_mutation)
                        .expect("concurrent registration must replace the published target");
                },
                || {
                    transaction.commit_with_post_validation(|| {
                        fs::write(&guard, b"changed")
                            .expect("guard mutation must trigger rollback");
                        Ok(())
                    })
                },
            )
        },
    )
    .expect_err("late guard failure must roll the registration back");
    let (quarantined, displaced) = paths_receiver
        .recv()
        .expect("rollback restore hook must report quarantine paths");

    assert!(error.contains("rollback conflict"), "{error}");
    assert!(error.contains("regular child identity changed"), "{error}");
    assert!(
        !config.exists(),
        "identity loss must not publish the same-name quarantine decoy"
    );
    assert_eq!(
        fs::read(&quarantined).expect("cleanup must preserve the quarantine decoy"),
        b"same-name quarantine decoy"
    );
    assert_eq!(
        fs::read(&displaced).expect("cleanup must preserve the displaced concurrent link"),
        concurrent
    );
    let recovery_directory = transaction_debris(&root)
        .into_iter()
        .find(|path| path.is_dir())
        .expect("unsafe registration rollback must preserve its recovery directory");
    assert_eq!(
        fs::read(recovery_directory.join("original")).unwrap(),
        original
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn registration_rollback_validation_reports_preserved_quarantine(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("registration-rollback-validation-quarantine-diagnostic");
    let config = root.join("Configuration.xml");
    let guard = root.join("Decision.bsl");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("registration fixture must be written");
    fs::write(&guard, b"stable").expect("guard fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    transaction
        .guard_exact_preimage(&guard, b"stable")
        .expect("guard must plan");
    let root_for_hook = root.clone();
    let (path_sender, path_receiver) = mpsc::channel();

    let error = with_after_registration_rollback_restore_hook(
        move |target, quarantined| {
            let replacement = root_for_hook.join("unexpected-restored-target.xml");
            fs::write(&replacement, b"unexpected restored bytes")
                .expect("replacement target must be written");
            replace_file_atomically(&replacement, target)
                .expect("restored target must be replaced before validation");
            path_sender
                .send(quarantined.to_path_buf())
                .expect("quarantine path must be reported");
        },
        || {
            transaction.commit_with_post_validation(|| {
                fs::write(&guard, b"changed").expect("guard must trigger rollback");
                Ok(())
            })
        },
    )
    .expect_err("restoration validation failure must be reported");
    let quarantined = path_receiver
        .recv()
        .expect("rollback hook must report the quarantine path");

    assert!(
        error.contains(&format!(
            "quarantined published bytes are preserved at {}",
            quarantined.display()
        )),
        "{error}"
    );
    assert!(fs::read_to_string(&quarantined)
        .expect("quarantined published bytes must remain readable")
        .contains("<Role>Reader</Role>"));
    let recovery_directory = quarantined
        .parent()
        .expect("quarantine must remain inside recovery");
    assert_eq!(
        fs::read(recovery_directory.join("original")).unwrap(),
        original
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn registration_rollback_restore_uses_retained_parent_handles_after_route_swap(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    if !testing::can_rename_parent_with_retained_cleanup_child_for_test() {
        return;
    }
    let root = temp_root("registration-rollback-restore-parent-route-swap");
    let active_parent = root.join("active");
    let displaced_parent = root.join("displaced-active");
    fs::create_dir(&active_parent).expect("active parent must be created");
    let config = active_parent.join("Configuration.xml");
    let guard = root.join("Decision.bsl");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("registration fixture must be written");
    fs::write(&guard, b"stable").expect("guard fixture must be written");
    let mut transaction = CompileTransaction::new();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .expect("registration must plan");
    let diff = transaction
        .registration_diffs()
        .pop()
        .expect("registration diff must exist");
    let mut concurrent = original.clone();
    concurrent.splice(diff.byte_range, diff.after);
    transaction
        .guard_exact_preimage(&guard, b"stable")
        .expect("guard must plan");
    let config_for_mutation = config.clone();
    let root_for_mutation = root.clone();
    let concurrent_for_mutation = concurrent.clone();
    let active_for_swap = active_parent.clone();
    let displaced_for_swap = displaced_parent.clone();
    let config_for_swap = config.clone();
    let (paths_sender, paths_receiver) = mpsc::channel();

    let error = with_before_rollback_quarantine_restore_rename_hook(
        move |quarantined, target| {
            assert_eq!(target, config_for_swap);
            let recovery_name = quarantined
                .parent()
                .and_then(Path::file_name)
                .expect("quarantine recovery name must exist")
                .to_os_string();
            fs::rename(&active_for_swap, &displaced_for_swap)
                .expect("verified rollback parents must be displaced");
            fs::create_dir(&active_for_swap).expect("replacement target parent must exist");
            let replacement_recovery = active_for_swap.join(&recovery_name);
            fs::create_dir(&replacement_recovery)
                .expect("replacement quarantine parent must exist");
            let replacement_quarantine = replacement_recovery.join("published");
            fs::write(&replacement_quarantine, b"same-name route decoy")
                .expect("same-name route decoy must be written");
            paths_sender
                .send((
                    replacement_quarantine,
                    active_for_swap.join("Configuration.xml"),
                    displaced_for_swap.join("Configuration.xml"),
                    displaced_for_swap.join(recovery_name).join("published"),
                ))
                .expect("swapped rollback paths must be reported");
        },
        || {
            with_before_rollback_mutation_hook(
                move |path| {
                    assert_eq!(path, config_for_mutation);
                    let external = root_for_mutation.join("external-registration.xml");
                    fs::write(&external, &concurrent_for_mutation)
                        .expect("concurrent registration must be written");
                    replace_file_atomically(&external, &config_for_mutation)
                        .expect("concurrent registration must replace the published target");
                },
                || {
                    transaction.commit_with_post_validation(|| {
                        fs::write(&guard, b"changed")
                            .expect("guard mutation must trigger rollback");
                        Ok(())
                    })
                },
            )
        },
    )
    .expect_err("late guard failure must roll the registration back");
    let (replacement_quarantine, replacement_target, restored_target, retained_quarantine) =
        paths_receiver
            .recv()
            .expect("rollback route-swap hook must report paths");

    assert!(error.contains("rollback conflict"), "{error}");
    assert_eq!(
        fs::read(&replacement_quarantine)
            .expect("descriptor-relative restore must preserve the route decoy"),
        b"same-name route decoy"
    );
    assert!(
        !replacement_target.exists(),
        "descriptor-relative restore must not publish through a replacement parent route"
    );
    assert_eq!(
        fs::read(&restored_target)
            .expect("concurrent target must be restored through its retained parent"),
        concurrent
    );
    assert!(
        !retained_quarantine.exists(),
        "the retained quarantine link must move to the retained target parent"
    );
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn after_registration_backup_failure_restores_exact_bytes_and_removes_debris(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("rollback-registration-backup");
    let config = root.join("Configuration.xml");
    let original = configuration_bytes();
    fs::write(&config, &original).expect("fixture must be written");
    let object = root.join("Roles/Reader.xml");
    let mut transaction = CompileTransaction::new();
    transaction
        .create_utf8_bom_text(
            &object,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction
        .register_canonical_child(&config, "Role", "Reader", registrar)
        .unwrap();

    let error = with_commit_failpoint(CommitFailpoint::AfterRegistrationBackup, || {
        transaction.commit()
    })
    .expect_err("failpoint must abort between registration renames");

    assert!(error.contains("after registration backup"), "{error}");
    assert!(!object.exists());
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(transaction_debris(&root).is_empty());
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("unica-compile.lock")
    }));
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn concurrent_registration_commits_serialize_before_preflight(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("concurrent-registration");
    let config = root.join("Configuration.xml");
    fs::write(&config, configuration_bytes()).expect("fixture must be written");
    let object_a = root.join("Roles/ReaderA.xml");
    let object_b = root.join("Roles/ReaderB.xml");

    let mut transaction_a = CompileTransaction::new();
    transaction_a
        .create_utf8_bom_text(
            &object_a,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction_a
        .register_canonical_child(&config, "Role", "ReaderA", registrar)
        .unwrap();

    let mut transaction_b = CompileTransaction::new();
    transaction_b
        .create_utf8_bom_text(
            &object_b,
            "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
        )
        .unwrap();
    transaction_b
        .register_canonical_child(&config, "Role", "ReaderB", registrar)
        .unwrap();

    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_a = acquired.clone();
    let release_a = release.clone();
    let thread_a = thread::spawn(move || {
        with_publication_lock_pause(acquired_by_a, release_a, || transaction_a.commit())
    });
    acquired.wait();

    let (contended_sender, contended_receiver) = mpsc::channel();
    let thread_b = thread::spawn(move || {
        with_publication_lock_contention_signal(contended_sender, || transaction_b.commit())
    });
    let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
    release.wait();

    let result_a = thread_a.join().expect("first commit thread must not panic");
    let result_b = thread_b
        .join()
        .expect("second commit thread must not panic");
    contention.expect("second thread must contend on the in-process registration lock");
    let report_a = result_a.expect("first transaction must commit");
    let error_b = result_b.expect_err("stale second plan must fail after acquiring the lock");

    assert_eq!(report_a.created, vec![object_a.clone()]);
    assert!(error_b.contains("changed after planning"), "{error_b}");
    assert!(object_a.is_file());
    assert!(!object_b.exists());
    let actual = fs::read_to_string(&config).unwrap();
    assert!(actual.contains("<Role>ReaderA</Role>"));
    assert!(!actual.contains("<Role>ReaderB</Role>"));
    assert!(transaction_debris(&root).is_empty());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}

pub fn symlink_targets_are_rejected(
    registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
) {
    let root = temp_root("symlink");
    let real = root.join("real.xml");
    let link = root.join("Configuration.xml");
    fs::write(&real, configuration_bytes()).expect("fixture must be written");
    let Some(symlink) = testing::create_file_symlink_for_test(&real, &link) else {
        fs::remove_dir_all(root).expect("temporary root must be removed");
        return;
    };
    symlink.expect("symlink must be created");
    let mut transaction = CompileTransaction::new();

    let error = transaction
        .register_canonical_child(&link, "Role", "Reader", registrar)
        .expect_err("symlink registration target must be rejected");

    assert!(error.contains("symbolic link"), "{error}");
    assert_eq!(fs::read(&real).unwrap(), configuration_bytes());
    fs::remove_dir_all(root).expect("temporary root must be removed");
}
