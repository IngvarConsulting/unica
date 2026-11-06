//! rule: INV.PKG.STARTUP-FAILURE-DIAGNOSIS

//! rule: INV.PKG.PREFETCH-EXIT-STATUS

#[cfg_attr(
    not(target_os = "linux"),
    ignore = "host capability SSL_CERT_FILE trust override: macOS and Windows use their OS certificate stores"
)]
#[test]
fn trusted_prefetch_cli_reports_checksum_failure_for_core_and_engine() {
    super::assert_trusted_cli_tls_failure(false);
}

#[cfg_attr(
    not(target_os = "linux"),
    ignore = "host capability SSL_CERT_FILE trust override: macOS and Windows use their OS certificate stores"
)]
#[test]
fn trusted_prefetch_cli_reports_body_wait_expiry_for_core_and_engine() {
    super::assert_trusted_cli_tls_failure(true);
}
