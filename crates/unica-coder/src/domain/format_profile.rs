pub use unica_format_core::ports::FormatCompatibility;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatProfile {
    pub platform_line: &'static str,
    pub export_format: &'static str,
}

pub const ACTIVE_FORMAT_PROFILE: FormatProfile = FormatProfile {
    // Task 8 moves these writer-serialization constants into the adapter.
    platform_line: "8.3.27",
    export_format: "2.20",
};

#[cfg(test)]
mod tests {
    use super::ACTIVE_FORMAT_PROFILE;

    #[test]
    fn active_profile_identity_remains_const_usable() {
        const IDENTITY: (&str, &str) = (
            ACTIVE_FORMAT_PROFILE.platform_line,
            ACTIVE_FORMAT_PROFILE.export_format,
        );

        assert_eq!(IDENTITY, ("8.3.27", "2.20"));
    }
}
