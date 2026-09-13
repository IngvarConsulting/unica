pub mod application;
mod composition;
pub mod domain;
pub(crate) mod infrastructure;
pub mod interfaces;

#[cfg(feature = "receipt-ledger-test-support")]
#[doc(hidden)]
pub mod receipt_ledger_test_support;

#[cfg(test)]
pub(crate) mod test_support;

pub use infrastructure::platform::run_platform_main;
