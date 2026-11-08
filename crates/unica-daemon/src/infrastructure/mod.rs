//! Нижний слой целиком и свои модули демона рядом с ним.
pub(crate) use unica_coder::infrastructure::*;

pub(crate) mod daemon;
#[cfg(feature = "receipt-ledger-test-support")]
#[path = "../../tests/support/receipt_scenario.rs"]
pub(crate) mod receipt_scenario;
