//! Демон Unica и его двери: сервер, службы v0.13, MCP, LSP и вход демона.
//! Нижний слой — разбор, чтение, правка и платформа — в `unica-coder`.

// Код демона пишет пути нижнего слоя как `crate::application`,
// `crate::infrastructure::…`: корень и `infrastructure` повторяют
// пространство имён `unica-coder`, свои модули стоят рядом.
#[cfg(test)]
pub(crate) use unica_coder::composition;
pub use unica_coder::{application, domain};

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

#[doc(hidden)]
pub use unica_coder::registered_apply_operation_names;
pub use unica_coder::run_platform_main;
