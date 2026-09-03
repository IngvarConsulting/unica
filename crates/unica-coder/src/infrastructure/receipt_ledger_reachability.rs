//! Feature-only declarations for ReceiptLedger scenario operations whose
//! production owners do not exist yet.
//!
//! These probes intentionally return a harness error. They must not open an
//! unrelated empty ledger or mint boundary evidence: only the future concrete
//! transition/capacity/identity/reconciliation owner may do that after it has
//! processed the typed scenario input.

use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReceiptLedgerReachabilityError {
    boundary: &'static str,
    operation: &'static str,
}

impl ReceiptLedgerReachabilityError {
    const fn missing_owner(boundary: &'static str, operation: &'static str) -> Self {
        Self {
            boundary,
            operation,
        }
    }
}

impl fmt::Display for ReceiptLedgerReachabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "production owner for {} operation {} is not implemented",
            self.boundary, self.operation
        )
    }
}

impl std::error::Error for ReceiptLedgerReachabilityError {}

type ProbeResult = Result<ProductionMissingTransitionEvidence, ReceiptLedgerReachabilityError>;

fn missing_owner(boundary: &'static str, operation: &'static str) -> ProbeResult {
    Err(ReceiptLedgerReachabilityError::missing_owner(
        boundary, operation,
    ))
}

macro_rules! missing_probe {
    ($name:ident, $boundary:literal, $operation:literal) => {
        pub(crate) fn $name() -> ProbeResult {
            missing_owner($boundary, $operation)
        }
    };
}

missing_probe!(
    run_recover_reachability_probe_for_test,
    "receipt_transition",
    "recover"
);
missing_probe!(
    run_acknowledge_reachability_probe_for_test,
    "receipt_transition",
    "acknowledge"
);
missing_probe!(
    run_cancel_reachability_probe_for_test,
    "receipt_transition",
    "cancel"
);
missing_probe!(
    run_spawn_cancel_reachability_probe_for_test,
    "receipt_transition",
    "spawn_cancel"
);
missing_probe!(
    run_seed_receipt_reachability_probe_for_test,
    "receipt_transition",
    "seed_receipt"
);
missing_probe!(
    run_inject_store_fault_reachability_probe_for_test,
    "receipt_transition",
    "inject_store_fault"
);
missing_probe!(
    run_fill_receipt_pool_reachability_probe_for_test,
    "capacity_coordination",
    "fill_receipt_pool"
);
missing_probe!(
    run_fill_task_links_reachability_probe_for_test,
    "capacity_coordination",
    "fill_task_links"
);
missing_probe!(
    run_fill_task_links_leaving_one_reservation_slot_reachability_probe_for_test,
    "capacity_coordination",
    "fill_task_links_leaving_one_reservation_slot"
);
missing_probe!(
    run_fill_tombstones_reachability_probe_for_test,
    "capacity_coordination",
    "fill_tombstones"
);
missing_probe!(
    run_attempt_task_store_bind_under_gate_reachability_probe_for_test,
    "capacity_coordination",
    "attempt_task_store_bind_under_gate"
);
missing_probe!(
    run_compare_client_server_identity_reachability_probe_for_test,
    "receipt_identity",
    "compare_client_server_identity"
);
missing_probe!(
    run_inject_persisted_identity_collision_reachability_probe_for_test,
    "receipt_identity",
    "inject_persisted_identity_collision"
);
missing_probe!(
    run_cross_store_crash_workload_reachability_probe_for_test,
    "cross_store_reconciliation",
    "run_cross_store_crash_workload"
);
missing_probe!(
    run_task_retirement_workload_reachability_probe_for_test,
    "cross_store_reconciliation",
    "run_task_retirement_workload"
);
missing_probe!(
    run_open_task_store_inspect_only_reachability_probe_for_test,
    "cross_store_reconciliation",
    "open_task_store_inspect_only"
);
missing_probe!(
    run_reconcile_startup_reachability_probe_for_test,
    "cross_store_reconciliation",
    "reconcile_startup"
);

#[cfg(test)]
mod tests {
    use super::*;

    type Probe = fn() -> ProbeResult;

    #[test]
    fn unavailable_routes_report_the_exact_missing_production_owner() {
        let cases: &[(&str, &str, Probe)] = &[
            (
                "receipt_transition",
                "recover",
                run_recover_reachability_probe_for_test,
            ),
            (
                "receipt_transition",
                "acknowledge",
                run_acknowledge_reachability_probe_for_test,
            ),
            (
                "receipt_transition",
                "cancel",
                run_cancel_reachability_probe_for_test,
            ),
            (
                "receipt_transition",
                "spawn_cancel",
                run_spawn_cancel_reachability_probe_for_test,
            ),
            (
                "receipt_transition",
                "seed_receipt",
                run_seed_receipt_reachability_probe_for_test,
            ),
            (
                "receipt_transition",
                "inject_store_fault",
                run_inject_store_fault_reachability_probe_for_test,
            ),
            (
                "capacity_coordination",
                "fill_receipt_pool",
                run_fill_receipt_pool_reachability_probe_for_test,
            ),
            (
                "capacity_coordination",
                "fill_task_links",
                run_fill_task_links_reachability_probe_for_test,
            ),
            (
                "capacity_coordination",
                "fill_task_links_leaving_one_reservation_slot",
                run_fill_task_links_leaving_one_reservation_slot_reachability_probe_for_test,
            ),
            (
                "capacity_coordination",
                "fill_tombstones",
                run_fill_tombstones_reachability_probe_for_test,
            ),
            (
                "capacity_coordination",
                "attempt_task_store_bind_under_gate",
                run_attempt_task_store_bind_under_gate_reachability_probe_for_test,
            ),
            (
                "receipt_identity",
                "compare_client_server_identity",
                run_compare_client_server_identity_reachability_probe_for_test,
            ),
            (
                "receipt_identity",
                "inject_persisted_identity_collision",
                run_inject_persisted_identity_collision_reachability_probe_for_test,
            ),
            (
                "cross_store_reconciliation",
                "run_cross_store_crash_workload",
                run_cross_store_crash_workload_reachability_probe_for_test,
            ),
            (
                "cross_store_reconciliation",
                "run_task_retirement_workload",
                run_task_retirement_workload_reachability_probe_for_test,
            ),
            (
                "cross_store_reconciliation",
                "open_task_store_inspect_only",
                run_open_task_store_inspect_only_reachability_probe_for_test,
            ),
            (
                "cross_store_reconciliation",
                "reconcile_startup",
                run_reconcile_startup_reachability_probe_for_test,
            ),
        ];

        for (boundary, operation, probe) in cases {
            let error = match probe() {
                Ok(_) => panic!("{boundary}/{operation} must not mint evidence without an owner"),
                Err(error) => error,
            };
            assert_eq!(
                error,
                ReceiptLedgerReachabilityError::missing_owner(boundary, operation)
            );
        }
    }
}
