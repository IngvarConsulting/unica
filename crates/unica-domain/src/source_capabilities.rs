use unica_bsl::profile::PlatformProfile;

/// Selects the semantic profile from the actor-issued source identity: the
/// platform line of its known format decides it. Legacy and test-only
/// identities name no known format and never gain a supported profile.
pub fn bsl_profile_for_source(
    profile: unica_source_contract::SourceProfile,
) -> Option<PlatformProfile> {
    profile.semantic_line().and_then(PlatformProfile::for_line)
}

#[cfg(test)]
mod tests {
    use super::bsl_profile_for_source;
    use unica_bsl::profile::PlatformProfile;
    use unica_source_contract::format_identity::PlatformLine;
    use unica_source_contract::SourceProfile;

    /// rule: INV.SOURCE.FORMAT-IDENTITY
    #[test]
    fn the_semantic_profile_follows_the_line_of_a_known_format() {
        let active = SourceProfile::platform_xml_8_3_27_format_2_20();
        assert_eq!(active.semantic_line(), Some(PlatformLine::new(8, 3, 27)));
        assert_eq!(
            bsl_profile_for_source(active),
            Some(PlatformProfile::v8_3_27())
        );
        for unknown in [
            SourceProfile::legacy_workspace_service_compatibility(),
            SourceProfile::TestPlatform8_3_28Format2_20,
            SourceProfile::TestPlatform8_3_27Format2_21,
        ] {
            assert_eq!(unknown.semantic_line(), None, "{unknown:?}");
            assert_eq!(bsl_profile_for_source(unknown), None, "{unknown:?}");
        }
        // A line without described capabilities has no semantic profile yet.
        assert_eq!(PlatformProfile::for_line(PlatformLine::new(8, 5, 4)), None);
        assert_eq!(
            PlatformProfile::for_line(PlatformLine::new(8, 3, 27)).map(PlatformProfile::line),
            Some(PlatformLine::new(8, 3, 27))
        );
    }
}
