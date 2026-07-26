use std::sync::Arc;

use unica_format_core::{
    ports::{CapturePort, CaptureResult, FormatReadRequest, ProbePort, ProbeResult, ReadPort},
    source::{AdapterManifest, SourceAdapterError, SourceContext},
};

use crate::versions::v2_20;

pub struct PlatformXmlAdapterRegistration {
    pub manifest: AdapterManifest,
    pub capture: Arc<dyn CapturePort>,
    pub probe: Arc<dyn ProbePort>,
    pub read: Arc<dyn ReadPort>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformXmlAdapterFactory;

impl PlatformXmlAdapterFactory {
    pub const fn new() -> Self {
        Self
    }

    pub fn registration(self) -> PlatformXmlAdapterRegistration {
        let adapter = Arc::new(PlatformXmlAdapter);
        PlatformXmlAdapterRegistration {
            manifest: v2_20::manifest(),
            capture: adapter.clone(),
            probe: adapter.clone(),
            read: adapter,
        }
    }

    pub const fn platform_line() -> &'static str {
        v2_20::PLATFORM_LINE
    }

    pub const fn export_format() -> &'static str {
        v2_20::EXPORT_FORMAT
    }

    pub const fn legacy_metadata_classes() -> &'static [&'static str] {
        v2_20::metadata_classes()
    }

    pub fn support_decision(
        bin_path: &std::path::Path,
        object_uuid: &str,
    ) -> Result<(&'static str, unica_format_core::navigation::Authorability), (String, Option<usize>)>
    {
        v2_20::support_decision(bin_path, object_uuid)
    }

    pub fn support_summary_lines(bin_path: &std::path::Path, is_extension: bool) -> Vec<String> {
        v2_20::support_summary_lines(bin_path, is_extension)
    }

    pub fn support_status(bin_path: &std::path::Path, object_uuid: &str) -> String {
        v2_20::support_status(bin_path, object_uuid)
    }

    pub fn support_header(text: &str) -> Option<(u8, usize)> {
        v2_20::support_header(text)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_owners(
        source: &SourceContext,
        expected_root: Option<(&str, &str)>,
        existing_only: bool,
        mut on_owner: impl FnMut(&str, &std::path::Path, Option<&str>, &[u8]),
        mut on_exact_candidate: impl FnMut(&std::path::Path, &[u8]),
        mut on_absent_candidate: impl FnMut(&std::path::Path),
        mut on_directory_membership: impl FnMut(&std::path::Path, &[std::ffi::OsString]),
    ) -> Result<(), SourceAdapterError> {
        let resolution = crate::owner::resolve(source, expected_root, existing_only)?;
        for owner in resolution.owners {
            on_owner(
                owner.kind.label(),
                &owner.path,
                owner.version.as_deref(),
                &owner.raw,
            );
        }
        for (path, candidate) in resolution.provenance.candidates {
            match candidate {
                crate::owner::CandidateInput::Exact(raw) => on_exact_candidate(&path, &raw),
                crate::owner::CandidateInput::Absent => on_absent_candidate(&path),
            }
        }
        for (directory, names) in resolution.provenance.directory_memberships {
            on_directory_membership(&directory, &names);
        }
        Ok(())
    }
}

struct PlatformXmlAdapter;

impl CapturePort for PlatformXmlAdapter {
    fn capture(&self, source: &SourceContext) -> Result<CaptureResult, SourceAdapterError> {
        v2_20::capture(source)
    }
}

impl ProbePort for PlatformXmlAdapter {
    fn probe(&self, source: &SourceContext) -> Result<ProbeResult, SourceAdapterError> {
        v2_20::probe(source)
    }
}

impl ReadPort for PlatformXmlAdapter {
    fn read(
        &self,
        request: &FormatReadRequest,
    ) -> Result<unica_format_core::navigation::NavigationEnvelope, SourceAdapterError> {
        let _ = &request.query;
        v2_20::read(&request.source, &request.snapshot)
    }
}
