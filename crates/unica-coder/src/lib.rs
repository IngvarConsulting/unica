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

// Репетиция двух полос: ошибка только под macOS должна пройти PR-гейт на
// ubuntu и покраснеть в очереди слияния, где идёт полная матрица.
#[cfg(all(test, target_os = "macos"))]
compile_error!("репетиция двух полос: macOS краснеет в очереди");
