//! Real discovery coupled to retained workspace assertion contracts.
use std::sync::Arc;
use unica_source_designer::project_sources::ActorSourceMapDiscovery;

#[test]
fn aggregate_selection_directory_handles_cross_former_128_and_keep_finality() {
    unica_workspace::selection_evidence::contract_tests::aggregate_selection_directory_handles_cross_former_128_and_keep_finality(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn aggregate_selection_exact_bytes_cross_former_32mib_without_source_walk() {
    unica_workspace::selection_evidence::contract_tests::aggregate_selection_exact_bytes_cross_former_32mib_without_source_walk(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn aggregate_selection_membership_crosses_former_16384_across_sources() {
    unica_workspace::selection_evidence::contract_tests::aggregate_selection_membership_crosses_former_16384_across_sources(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn published_replacement_of_a_retained_source_map_file_passes_the_final_gate() {
    unica_workspace::selection_evidence::contract_tests::published_replacement_of_a_retained_source_map_file_passes_the_final_gate(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_rejects_aggregate_exact_byte_budget() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_rejects_aggregate_exact_byte_budget(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_charges_repeated_exact_work_before_second_read() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_charges_repeated_exact_work_before_second_read(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_bounds_unique_retained_directories_without_ulimit() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_bounds_unique_retained_directories_without_ulimit(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_accepts_external_processor_and_report_with_non_evidence_members() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_accepts_external_processor_and_report_with_non_evidence_members(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_bounds_global_membership_across_external_source_sets() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_bounds_global_membership_across_external_source_sets(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_counts_repeated_membership_enumeration_globally() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_counts_repeated_membership_enumeration_globally(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_rejects_total_evidence_record_budget() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_rejects_total_evidence_record_budget(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_rejects_route_and_name_byte_budget() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_rejects_route_and_name_byte_budget(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_comparison_honors_cancellation() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_comparison_honors_cancellation(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}

#[test]
fn actor_admission_comparison_honors_deadline() {
    unica_workspace::selection_evidence::contract_tests::actor_admission_comparison_honors_deadline(
        Arc::new(ActorSourceMapDiscovery::new(
            unica_workspace::paths::host_path_text,
        )),
    );
}

#[test]
fn retained_selection_pass_rejects_inconsistent_regular_repeat() {
    unica_workspace::selection_evidence::contract_tests::retained_selection_pass_rejects_inconsistent_regular_repeat(Arc::new(ActorSourceMapDiscovery::new(unica_workspace::paths::host_path_text)));
}
