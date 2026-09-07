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

// Репетиция двух полос: тест падает только на macOS и без `cfg` по ОС —
// страж границы платформ не терпит таких условий вне фасада `platform/`.
// Ожидание: PR-гейт на ubuntu зелёный, очередь с полной матрицей красная.
#[cfg(test)]
mod two_lane_rehearsal {
    #[test]
    fn fails_only_on_macos() {
        assert_ne!(std::env::consts::OS, "macos", "репетиция двух полос: macOS краснеет в очереди");
    }
}
