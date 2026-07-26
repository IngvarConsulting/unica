use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
pub use unica_format_core::ports::AdapterFormatProfile as FormatProfile;
pub use unica_format_core::ports::FormatCompatibility;

pub const ACTIVE_FORMAT_PROFILE: FormatProfile = PlatformXmlAdapterFactory::profile();

#[cfg(test)]
mod tests {
    use super::ACTIVE_FORMAT_PROFILE;

    #[test]
    fn active_profile_is_platform_8_3_27_format_2_20() {
        assert_eq!(ACTIVE_FORMAT_PROFILE.platform_line, "8.3.27");
        assert_eq!(ACTIVE_FORMAT_PROFILE.export_format.to_string(), "2.20");
    }
}
