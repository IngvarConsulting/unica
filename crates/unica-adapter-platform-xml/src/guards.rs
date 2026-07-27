use sha2::{Digest, Sha256};
use unica_format_core::{
    ports::{
        AuthorabilityPort, AuthorabilityRequest, AuthorabilityResult, CompatibilityIssueKind,
        CompatibilityPort, CompatibilityRequest, CompatibilityResult, FormatDiagnostic,
        FormatDiagnosticCode, FormatDiagnosticDetail, OperationalEvidenceRevision,
        SourceCompatibilityEvidence, SourceCompatibilityPort, SourceCompatibilityRequest,
        SourceCompatibilityResult,
    },
    source::SourceAdapterError,
};

use crate::versions::v2_20;

pub(crate) struct PlatformXmlGuards;

impl CompatibilityPort for PlatformXmlGuards {
    fn inspect(
        &self,
        request: &CompatibilityRequest,
    ) -> Result<CompatibilityResult, SourceAdapterError> {
        let mut older = None;
        let mut malformed = None;
        let mut newer = None;
        let mut evidence = Sha256::new();
        evidence.update(b"unica:platform-xml:compatibility-request:v1\0");
        for handle in request.sessions() {
            let session = v2_20::operations::session_from_handle(handle)?;
            let result = v2_20::operations::compatibility(session);
            evidence.update(result.evidence_revision().digest());
            match result.issue().map(|issue| issue.kind()) {
                None => {}
                Some(CompatibilityIssueKind::Newer) if newer.is_none() => {
                    newer = result.issue().cloned()
                }
                Some(CompatibilityIssueKind::Newer) => {}
                Some(CompatibilityIssueKind::Malformed) if malformed.is_none() => {
                    malformed = result.issue().cloned()
                }
                Some(CompatibilityIssueKind::Malformed) => {}
                Some(CompatibilityIssueKind::Older) if older.is_none() => {
                    older = result.issue().cloned()
                }
                Some(CompatibilityIssueKind::Older) => {}
            }
        }
        let evidence = OperationalEvidenceRevision::from_digest(evidence.finalize().into());
        Ok(match newer.or(malformed).or(older) {
            Some(issue) => CompatibilityResult::incompatible(issue, evidence),
            None => CompatibilityResult::compatible(evidence),
        })
    }
}

impl SourceCompatibilityPort for PlatformXmlGuards {
    fn inspect_source(
        &self,
        request: &SourceCompatibilityRequest,
    ) -> Result<SourceCompatibilityResult, SourceAdapterError> {
        Ok(match request.evidence() {
            SourceCompatibilityEvidence::Compatible => SourceCompatibilityResult::compatible(),
            SourceCompatibilityEvidence::AlternateFamily => {
                SourceCompatibilityResult::incompatible(
                    FormatDiagnostic::new(
                        FormatDiagnosticCode::SourceFamilyIncompatible,
                        FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Malformed),
                    )
                    .expect("source-family diagnostic is closed"),
                )
            }
            SourceCompatibilityEvidence::Ambiguous => SourceCompatibilityResult::incompatible(
                FormatDiagnostic::new(
                    FormatDiagnosticCode::SourceFamilyIncompatible,
                    FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Malformed),
                )
                .expect("source-family diagnostic is closed"),
            ),
            SourceCompatibilityEvidence::UnsupportedDeclaration => {
                SourceCompatibilityResult::incompatible(
                    FormatDiagnostic::new(
                        FormatDiagnosticCode::SourceFamilyIncompatible,
                        FormatDiagnosticDetail::Compatibility(CompatibilityIssueKind::Malformed),
                    )
                    .expect("source-family diagnostic is closed"),
                )
            }
        })
    }
}

impl AuthorabilityPort for PlatformXmlGuards {
    fn inspect(
        &self,
        request: &AuthorabilityRequest,
    ) -> Result<AuthorabilityResult, SourceAdapterError> {
        let session = v2_20::operations::session_from_handle(request.session())?;
        Ok(v2_20::operations::authorability(
            session,
            request.requirement(),
        ))
    }
}
