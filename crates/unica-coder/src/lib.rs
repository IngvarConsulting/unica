pub mod application;
mod composition;
pub mod domain;
pub(crate) mod infrastructure;
pub mod interfaces;

#[cfg(feature = "receipt-ledger-test-support")]
#[doc(hidden)]
pub mod receipt_ledger_test_support;

#[cfg(feature = "receipt-ledger-test-support")]
#[doc(hidden)]
pub mod frontend_restart_test_support;

// Входы проверок обвязки сценариев. Не фасад: вход без аргументов только
// запускает проверку, `#[test]` к ним приставляет бинарь контракта.
#[cfg(feature = "receipt-ledger-test-support")]
#[doc(hidden)]
pub use infrastructure::receipt_scenario::harness_checks::entry as receipt_scenario_harness_checks;

pub use infrastructure::platform::run_platform_main;

/// Имена операций `unica.apply` из реестра, по которому словарь `can`
/// публикует операции узла. Вход для проверок, которые видят только
/// публичный API.
#[doc(hidden)]
pub fn registered_apply_operation_names() -> Vec<&'static str> {
    domain::apply::operation_registry().names()
}
