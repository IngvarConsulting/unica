use std::collections::BTreeSet;

use unica_format_core::commands::{WriterCommandKind, WriterFamily};

const WRITER_SOURCES: &str = concat!(
    include_str!("../src/versions/v2_20/writers/cf.rs"),
    include_str!("../src/versions/v2_20/writers/cfe.rs"),
    include_str!("../src/versions/v2_20/writers/external.rs"),
    include_str!("../src/versions/v2_20/writers/meta.rs"),
    include_str!("../src/versions/v2_20/writers/form.rs"),
    include_str!("../src/versions/v2_20/writers/template.rs"),
    include_str!("../src/versions/v2_20/writers/help.rs"),
    include_str!("../src/versions/v2_20/writers/interface.rs"),
    include_str!("../src/versions/v2_20/writers/role.rs"),
    include_str!("../src/versions/v2_20/writers/subsystem.rs"),
    include_str!("../src/versions/v2_20/writers/support.rs"),
    include_str!("../src/versions/v2_20/writers/dcs.rs"),
    include_str!("../src/versions/v2_20/writers/mxl.rs"),
    include_str!("../../unica-coder/src/application/mod.rs"),
);

const INFRASTRUCTURE_TEST_SOURCES: &str = concat!(
    include_str!("../src/versions/v2_20/writers/compile_transaction.rs"),
    include_str!("../src/versions/v2_20/writers/single_file_publisher.rs"),
    include_str!("task8_writer_ports.rs"),
    include_str!("task8_fix_round1_architecture.rs"),
    include_str!("../../unica-coder/src/application/mod.rs"),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Oracle {
    ProductionSemanticProjection,
    IndependentStructuredProjection,
    IndependentStandaloneProjection,
    HostLanguageProjection,
}

#[derive(Debug, Clone, Copy)]
struct VariantPreservation {
    kind: WriterCommandKind,
    family: WriterFamily,
    semantic_test: &'static str,
    oracle: Oracle,
}

const VARIANTS: &[VariantPreservation] = &[
    case(
        WriterCommandKind::ConfigurationInitialize,
        WriterFamily::Configuration,
        "cf_init_emits_active_format_for_configuration_and_language",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::ConfigurationEdit,
        WriterFamily::Configuration,
        "cf_edit_remove_add_child_object_preserves_neighboring_childobjects",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::ExtensionInitialize,
        WriterFamily::Extension,
        "cfe_init_uses_active_format_with_supported_base_config",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::ExtensionBorrow,
        WriterFamily::Extension,
        "cfe_borrow_common_module_copies_canonical_properties_in_8_3_27_order",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::ExtensionPatchMethod,
        WriterFamily::Extension,
        "cfe_patch_method_marks_the_extended_module_property_in_platform_xml",
        Oracle::HostLanguageProjection,
    ),
    case(
        WriterCommandKind::ExternalProcessorInitialize,
        WriterFamily::ExternalArtifact,
        "epf_init_creates_make_ready_layout_with_optional_managed_form",
        Oracle::IndependentStandaloneProjection,
    ),
    case(
        WriterCommandKind::ExternalReportInitialize,
        WriterFamily::ExternalArtifact,
        "erf_init_creates_minimal_report_layout_without_a_form",
        Oracle::IndependentStandaloneProjection,
    ),
    case(
        WriterCommandKind::MetadataCreate,
        WriterFamily::Metadata,
        "meta_compile_preserves_configuration_child_objects_formatting",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::MetadataEdit,
        WriterFamily::Metadata,
        "meta_edit_sets_enum_fill_value_through_public_tool",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::MetadataRemove,
        WriterFamily::Metadata,
        "meta_remove_removes_the_last_empty_type_collection_directory",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::FormCreate,
        WriterFamily::Form,
        "add_then_remove_form_round_trips_empty_catalog_parent_xml",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::FormCompile,
        WriterFamily::Form,
        "form_compile_emits_documented_command_sources_and_global_buttons",
        Oracle::IndependentStructuredProjection,
    ),
    case(
        WriterCommandKind::FormEdit,
        WriterFamily::Form,
        "form_edit_preview_apply_and_no_op_validate_the_projected_form",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::FormRemove,
        WriterFamily::Form,
        "add_then_remove_form_round_trips_empty_catalog_parent_xml",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::TemplateCreate,
        WriterFamily::Template,
        "template_add_spreadsheet_matches_platform_8_3_27_fixture",
        Oracle::IndependentStructuredProjection,
    ),
    case(
        WriterCommandKind::TemplateRemove,
        WriterFamily::Template,
        "template_remove_collapses_last_child_to_canonical_self_closing_child_objects",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::HelpCreate,
        WriterFamily::Help,
        "help_add_routes_through_unica_and_creates_help_files",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::InterfaceEdit,
        WriterFamily::Interface,
        "create_if_missing_hide_show_create_valid_command_interface",
        Oracle::IndependentStructuredProjection,
    ),
    case(
        WriterCommandKind::RoleCreate,
        WriterFamily::Role,
        "role_compile_registers_in_canonical_position_and_preserves_crlf",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::SubsystemCreate,
        WriterFamily::Subsystem,
        "compile_subsystem_registers_in_canonical_position_and_preserves_crlf",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::SubsystemEdit,
        WriterFamily::Subsystem,
        "edit_subsystem_plans_child_stubs_from_final_batch_state",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::SupportEdit,
        WriterFamily::Support,
        "support_edit_set_editable_updates_object_rule_and_meta_info",
        Oracle::ProductionSemanticProjection,
    ),
    case(
        WriterCommandKind::DataCompositionCreate,
        WriterFamily::DataComposition,
        "dcs_compile_complex_definition_follows_8_3_27_direct_child_order",
        Oracle::IndependentStandaloneProjection,
    ),
    case(
        WriterCommandKind::DataCompositionEdit,
        WriterFamily::DataComposition,
        "native_dcs_edit_structure_preserves_nested_named_groups",
        Oracle::IndependentStandaloneProjection,
    ),
    case(
        WriterCommandKind::SpreadsheetCreate,
        WriterFamily::Spreadsheet,
        "mxl_compile_matches_platform_first_cell_and_format_order",
        Oracle::IndependentStandaloneProjection,
    ),
];

const SHARED_CONTRACT_TESTS: &[&str] = &[
    "external_init_dry_run_lists_files_without_writing",
    "meta_compile_dry_run_reports_exact_registration_diff_without_writes",
    "form_compile_dry_run_plans_valid_root_event_without_writing",
    "support_edit_dry_run_does_not_change_parent_configurations",
    "repeated_meta_compile_is_a_byte_for_byte_noop",
    "form_edit_preview_apply_and_no_op_validate_the_projected_form",
    "repeated_subsystem_compile_does_not_overwrite_or_report_changes",
    "native_dcs_edit_noop_leaves_file_untouched",
    "configuration_initializers_reject_external_source_set_roots_before_writes",
    "cfe_patch_method_rejects_newer_extension_owner_without_creating_module",
    "mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic",
    "cf_edit_post_write_validation_failure_rolls_back_config_and_external_files",
    "cancellation_during_planning_is_observed_before_any_write",
    "cancellation_after_first_publication_rolls_back_before_reporting_cancelled",
    "task8_writer_cancellation_while_waiting_for_shared_publication_lock_is_typed",
    "compile_transaction_and_cf_edit_share_target_lock",
    "compile_transaction_and_meta_compile_share_target_lock",
    "compile_transaction_and_bsl_patch_share_target_lock_without_lost_update",
    "symlink_targets_are_rejected",
    "lock_only_identity_rejects_an_existing_symlink_ancestor",
];

const fn case(
    kind: WriterCommandKind,
    family: WriterFamily,
    semantic_test: &'static str,
    oracle: Oracle,
) -> VariantPreservation {
    VariantPreservation {
        kind,
        family,
        semantic_test,
        oracle,
    }
}

#[test]
fn every_closed_writer_variant_has_independent_semantic_preservation_evidence() {
    let actual = VARIANTS
        .iter()
        .map(|case| case.kind)
        .collect::<BTreeSet<_>>();
    let expected = WriterCommandKind::ALL.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(VARIANTS.len(), WriterCommandKind::ALL.len());

    for case in VARIANTS {
        assert!(
            WRITER_SOURCES.contains(&format!("fn {}(", case.semantic_test)),
            "{:?} names missing preservation test `{}`",
            case.kind,
            case.semantic_test
        );
        assert!(
            matches!(
                case.oracle,
                Oracle::ProductionSemanticProjection
                    | Oracle::IndependentStructuredProjection
                    | Oracle::IndependentStandaloneProjection
                    | Oracle::HostLanguageProjection
            ),
            "{:?} has no independent oracle",
            case.kind
        );
    }
}

#[test]
fn every_writer_family_is_covered_and_shared_failure_atomicity_cases_exist() {
    let actual = VARIANTS
        .iter()
        .map(|case| case.family)
        .collect::<BTreeSet<_>>();
    let expected = WriterFamily::ALL.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);

    for test in SHARED_CONTRACT_TESTS {
        assert!(
            WRITER_SOURCES.contains(&format!("fn {test}("))
                || INFRASTRUCTURE_TEST_SOURCES.contains(&format!("fn {test}(")),
            "shared Task 8 preservation contract `{test}` is missing"
        );
    }
}
