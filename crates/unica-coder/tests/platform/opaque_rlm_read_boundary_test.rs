#[cfg_attr(
    not(target_os = "linux"),
    ignore = "host capability private-index-open-observer: inotify is Linux-only"
)]
/// rule: INV.APP.PROVIDER-PRIVATE-STATE
#[test]
#[should_panic(expected = "consumer opened private index")]
fn canonical_analysis_detects_an_ignored_consumer_private_read() {
    check_opaque_format(0, true);
}
