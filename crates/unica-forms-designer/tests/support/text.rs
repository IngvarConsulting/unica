pub fn assert_platform_text_uses_crlf_without_bare_lf(text: &str) {
    assert!(text.contains("\r\n"));
    assert!(!text.replace("\r\n", "").contains('\n'));
}
