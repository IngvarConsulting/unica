use unica_bsl::profile::PlatformProfile;

/// Selects the semantic profile from the exact actor-issued source identity.
/// Legacy and test-only identities never gain a writable supported profile.
pub(crate) fn bsl_profile_for_source(
    profile: unica_source_contract::SourceProfile,
) -> Option<PlatformProfile> {
    if profile != unica_source_contract::SourceProfile::platform_xml_8_3_27_format_2_20() {
        return None;
    }
    debug_assert_eq!(
        profile.canonical_id(),
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20
    );
    debug_assert_eq!(
        profile.platform_line(),
        Some(PlatformProfile::v8_3_27().id())
    );
    debug_assert_eq!(
        profile.serialization_format(),
        Some(unica_source_contract::ACTIVE_SERIALIZATION_FORMAT)
    );
    Some(PlatformProfile::v8_3_27())
}
