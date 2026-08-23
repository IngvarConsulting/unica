use crate::application::{tools, ToolHandler};
use std::collections::BTreeMap;

/// The public platform-XML mutation surface is closed here by domain family. A
/// new native or typed metadata mutator cannot inherit rewrite or preimage
/// claims merely by being registered: both aggregate falsifiers below must be
/// extended with real family evidence.
fn public_platform_xml_mutator_inventory() -> BTreeMap<&'static str, &'static str> {
    let expected = BTreeMap::from([
        ("unica.cf.edit", "configuration"),
        ("unica.cf.init", "configuration"),
        ("unica.cfe.borrow", "extension"),
        ("unica.cfe.init", "extension"),
        ("unica.cfe.patch_method", "extension"),
        ("unica.code.patch", "code"),
        ("unica.dcs.compile", "dcs"),
        ("unica.dcs.edit", "dcs"),
        ("unica.epf.init", "external"),
        ("unica.erf.init", "external"),
        ("unica.form.add", "form"),
        ("unica.form.compile", "form"),
        ("unica.form.edit", "form"),
        ("unica.form.remove", "form"),
        ("unica.interface.edit", "interface"),
        ("unica.meta.add", "metadata"),
        ("unica.meta.edit", "metadata"),
        ("unica.meta.remove", "metadata"),
        ("unica.mxl.compile", "mxl"),
        ("unica.role.compile", "role"),
        ("unica.role.edit", "role"),
        ("unica.subsystem.compile", "subsystem"),
        ("unica.subsystem.edit", "subsystem"),
        ("unica.support.edit", "support"),
        ("unica.xdto.edit", "xdto"),
    ]);
    let actual = tools()
        .into_iter()
        .filter(|tool| tool.execution.is_mutating())
        .filter(|tool| {
            matches!(
                tool.handler,
                ToolHandler::NativeOperation { .. } | ToolHandler::Metadata { .. }
            )
        })
        .map(|tool| {
            let family = expected.get(tool.name).unwrap_or_else(|| {
                panic!(
                    "public platform-XML mutator {} has no source contract family",
                    tool.name
                )
            });
            (tool.name, *family)
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual, expected);
    actual
}

#[test]
fn verified_public_mutator_idempotence_cases_are_exact() {
    let cases: [(&str, fn()); 12] = [
        ("unica.cf.edit", crate::application::tests::cf_edit_equal_serialized_result_is_a_public_noop_and_preserves_identity),
        ("unica.cfe.borrow", super::cfe::tests::borrow_cfe_preserves_object_and_file_identity_on_repeated_borrow),
        ("unica.code.patch", super::code::tests::applied_patch_returns_typed_data_and_repeated_apply_is_noop_with_stable_identity),
        ("unica.dcs.edit", super::dcs::tests::native_dcs_edit_noop_leaves_file_bytes_and_identity_untouched),
        ("unica.form.edit", super::form::tests::edit_form_identical_event_is_byte_and_identity_exact_idempotent_noop),
        ("unica.interface.edit", super::interface::tests::repeated_interface_edit_preserves_identity_but_reports_attempted_update),
        ("unica.meta.edit", super::meta::typed_resource_noop_and_identity_contract_is_complete),
        ("unica.mxl.compile", super::mxl::tests::repeated_mxl_compile_preserves_identity_but_reports_attempted_update),
        ("unica.role.edit", super::role::role_edit_contract_tests::role_edit_without_vendor_support_is_logically_addressed_and_preserves_identity),
        ("unica.subsystem.compile", super::subsystem::tests::repeated_subsystem_compile_preserves_file_identities_and_reports_no_changes),
        ("unica.support.edit", super::support::tests::repeated_support_edit_is_a_byte_and_identity_exact_noop),
        ("unica.xdto.edit", super::xdto::tests::xdto_events_and_file_identity_follow_changed_plan_and_exact_noop),
    ];
    let registered = public_platform_xml_mutator_inventory();
    for (tool, evidence) in cases {
        assert!(
            registered.contains_key(tool),
            "verified mutator {tool} left the public surface"
        );
        evidence();
    }
}

#[test]
fn typed_platform_resource_noop_emits_no_effects() {
    super::meta::typed_resource_noop_and_identity_contract_is_complete();
    super::role::role_edit_contract_tests::role_edit_without_vendor_support_is_logically_addressed_and_preserves_identity();
    super::xdto::tests::xdto_events_and_file_identity_follow_changed_plan_and_exact_noop();
}

#[test]
fn public_platform_xml_mutator_preimage_contract_is_complete() {
    let cases: [(&str, fn()); 25] = [
        ("unica.cf.edit", super::cf::cf_edit_transaction_tests::cf_edit_external_only_change_rejects_concurrent_format_owner_change),
        ("unica.cf.init", super::cf::cf_init_transaction_tests::cf_init_reauthorizes_containing_owner_immediately_before_publication),
        ("unica.cfe.borrow", super::cfe::tests::borrow_cfe_rejects_concurrent_base_format_owner_change),
        ("unica.cfe.init", super::cfe::tests::cfe_init_rejects_concurrent_base_format_owner_change),
        ("unica.cfe.patch_method", super::cfe::tests::cfe_patch_method_binds_exact_existing_bsl_preimage),
        ("unica.code.patch", super::code::tests::code_patch_rolls_back_if_owner_descriptor_changes_before_commit),
        ("unica.dcs.compile", super::dcs::tests::dcs_compile_rolls_back_if_format_owner_changes_during_publication),
        ("unica.dcs.edit", super::dcs::tests::dcs_edit_preserves_a_concurrent_replacement_instead_of_overwriting_it),
        ("unica.epf.init", super::external::tests::external_init_reauthorizes_containing_owner_immediately_before_publication),
        ("unica.erf.init", super::external::tests::external_init_reauthorizes_containing_owner_immediately_before_publication),
        ("unica.form.add", super::form::tests::add_form_rejects_scaffold_member_created_after_planning),
        ("unica.form.compile", super::form::tests::form_compile_rolls_back_if_unchanged_parent_owner_changes_during_publication),
        ("unica.form.edit", super::form::tests::edit_form_rejects_stale_preimage_without_overwriting_concurrent_change),
        ("unica.form.remove", super::form::tests::remove_form_rejects_payload_directory_that_appears_after_absent_probe),
        ("unica.interface.edit", super::interface::tests::interface_edit_rolls_back_if_unchanged_metadata_owner_changes_during_publication),
        ("unica.meta.add", super::meta::typed_resource_preimage_contract_is_complete),
        ("unica.meta.edit", crate::infrastructure::metadata_operations::tests::typed_edit_concurrency_and_rollback_preserve_exact_external_or_preimage_bytes),
        ("unica.meta.remove", crate::infrastructure::metadata_operations::tests::meta_remove_publish_honors_cancellation_and_owner_exact_preimages),
        ("unica.mxl.compile", super::mxl::tests::mxl_compile_rolls_back_if_format_owner_changes_during_publication),
        ("unica.role.compile", super::role::role_compile_contract_tests::role_compile_rolls_back_if_supported_format_owner_appears_during_publication),
        ("unica.role.edit", super::role::role_edit_contract_tests::rights_drift_in_the_staging_window_is_classified_as_concurrent),
        ("unica.subsystem.compile", super::subsystem::tests::subsystem_compile_exact_binds_a_reused_existing_child),
        ("unica.subsystem.edit", super::subsystem::tests::subsystem_edit_exact_binds_a_reused_existing_child),
        ("unica.support.edit", super::support::tests::support_edit_rejects_a_concurrent_configuration_owner_change),
        ("unica.xdto.edit", super::xdto::tests::xdto_guard_rejects_descriptor_identity_drift_before_commit),
    ];
    let inventory = public_platform_xml_mutator_inventory();
    let actual = cases
        .iter()
        .map(|(tool, _)| *tool)
        .collect::<std::collections::BTreeSet<_>>();
    let expected = inventory
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected);
    for (_, evidence) in cases {
        evidence();
    }
}

#[test]
fn repeated_interface_and_mxl_mutations_preserve_file_identity_but_report_attempted_updates() {
    super::interface::tests::repeated_interface_edit_preserves_identity_but_reports_attempted_update();
    super::mxl::tests::repeated_mxl_compile_preserves_identity_but_reports_attempted_update();
}

#[test]
fn mutation_idempotence_scope_decision_is_fully_realized() {
    verified_public_mutator_idempotence_cases_are_exact();
    repeated_interface_and_mxl_mutations_preserve_file_identity_but_report_attempted_updates();
}

/// One registry-facing falsifier for response parity across the complete
/// subject-reader bridge. Each family keeps its focused fixture below its
/// handler; this aggregate prevents a partial family list from grounding the
/// cross-family invariant.
#[test]
fn bridged_reader_outputs_are_identical_for_logical_and_physical_selectors() {
    use crate::application::tool_contracts::{
        authoritative_reader_migration_inventory, ReaderMigrationMode,
    };
    use std::collections::BTreeSet;

    let cases: [(&str, fn()); 13] = [
        ("unica.cf.info", super::cf::cf_read_selector_bridge_tests::cf_info_answers_identically_for_a_source_set_and_a_config_path),
        ("unica.cf.validate", super::cf::cf_read_selector_bridge_tests::cf_validate_answers_identically_for_a_source_set_and_a_config_path),
        ("unica.form.info", super::form::form_read_selector_bridge_tests::form_info_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.form.validate", super::form::form_read_selector_bridge_tests::form_validate_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.role.info", super::role::role_info_typed_result_tests::role_info_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.role.validate", super::role::role_info_typed_result_tests::role_validate_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.mxl.info", super::mxl::mxl_read_selector_bridge_tests::mxl_info_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.mxl.validate", super::mxl::mxl_read_selector_bridge_tests::mxl_validate_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.mxl.decompile", super::mxl::mxl_read_selector_bridge_tests::mxl_decompile_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.dcs.info", super::mxl::mxl_read_selector_bridge_tests::dcs_info_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.dcs.validate", super::mxl::mxl_read_selector_bridge_tests::dcs_validate_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.subsystem.info", super::subsystem::subsystem_read_selector_bridge_tests::subsystem_info_answers_identically_for_a_logical_and_a_physical_selector),
        ("unica.subsystem.validate", super::subsystem::subsystem_read_selector_bridge_tests::subsystem_validate_answers_identically_for_a_logical_and_a_physical_selector),
    ];
    let expected = authoritative_reader_migration_inventory()
        .filter_map(|(name, mode)| (mode == ReaderMigrationMode::Bridge).then_some(name))
        .collect::<BTreeSet<_>>();
    let actual = cases.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    for (_, parity_test) in cases {
        parity_test();
    }
}

/// One registry-facing falsifier for every clause of selector-free tail
/// insertion, whose public schema and write behavior live on opposite sides of
/// the application/infrastructure boundary.
#[test]
fn tail_insert_public_and_write_contract_is_complete() {
    crate::application::tool_contracts::tests::code_patch_tail_insert_public_contract_is_closed();
    super::code::tests::code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat();
    super::code::tests::code_patch_writes_the_first_body_of_an_empty_or_bom_only_module();
    super::code::tests::code_patch_creates_a_module_file_the_platform_never_exported();
    super::code::tests::code_patch_refuses_a_module_role_the_metadata_kind_never_owns();
}
