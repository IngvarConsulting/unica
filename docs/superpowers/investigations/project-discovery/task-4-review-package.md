# Review package: 0fce5d9..HEAD

## Commits
c544dc2 fix: закрыть финальные snapshot races
89f390b feat: добавить content source snapshots

## Files changed
 Cargo.lock                                         |    1 +
 crates/unica-coder/Cargo.toml                      |   11 +
 .../unica-coder/src/application/discovery/mod.rs   |   72 +-
 .../unica-coder/src/application/discovery/model.rs |   38 +-
 .../unica-coder/src/application/discovery/ports.rs |  512 ++++-
 .../src/application/discovery/use_case.rs          |  834 +++++--
 crates/unica-coder/src/application/mod.rs          |   18 +-
 .../unica-coder/src/application/tool_contracts.rs  |    3 +-
 .../unica-coder/src/domain/discovery_registry.rs   |   22 +
 crates/unica-coder/src/domain/project_sources.rs   |  555 +----
 crates/unica-coder/src/domain/source_snapshot.rs   |  648 ++++--
 .../unica-coder/src/infrastructure/contained_fs.rs |  650 ++++++
 crates/unica-coder/src/infrastructure/mod.rs       |    4 +
 .../unica-coder/src/infrastructure/platform_xml.rs |  267 +++
 .../src/infrastructure/project_sources.rs          |  945 ++++++++
 .../src/infrastructure/source_snapshot.rs          | 2389 ++++++++++++++++++++
 16 files changed, 6115 insertions(+), 854 deletions(-)

## Diff
diff --git a/Cargo.lock b/Cargo.lock
index e60169e..021fdb8 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -707,20 +707,21 @@ dependencies = [
  "fs2",
  "libc",
  "roxmltree",
  "rusqlite",
  "serde",
  "serde_json",
  "serde_yaml",
  "sha2",
  "ureq",
  "uuid",
+ "windows-sys",
 ]
 
 [[package]]
 name = "unicode-ident"
 version = "1.0.24"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75"
 
 [[package]]
 name = "unsafe-libyaml"
diff --git a/crates/unica-coder/Cargo.toml b/crates/unica-coder/Cargo.toml
index 1650bde..58e6924 100644
--- a/crates/unica-coder/Cargo.toml
+++ b/crates/unica-coder/Cargo.toml
@@ -19,10 +19,21 @@ serde.workspace = true
 serde_json.workspace = true
 serde_yaml.workspace = true
 sha2.workspace = true
 roxmltree.workspace = true
 rusqlite.workspace = true
 ureq.workspace = true
 uuid.workspace = true
 
 [target.'cfg(unix)'.dependencies]
 libc = "0.2"
+
+[target.'cfg(windows)'.dependencies]
+windows-sys = { version = "0.52", features = [
+    "Wdk_Foundation",
+    "Wdk_Storage_FileSystem",
+    "Win32_Foundation",
+    "Win32_Globalization",
+    "Win32_Storage_FileSystem",
+    "Win32_System_IO",
+    "Win32_System_Kernel",
+] }
diff --git a/crates/unica-coder/src/application/discovery/mod.rs b/crates/unica-coder/src/application/discovery/mod.rs
index b028bf3..43addfc 100644
--- a/crates/unica-coder/src/application/discovery/mod.rs
+++ b/crates/unica-coder/src/application/discovery/mod.rs
@@ -1189,42 +1189,112 @@ mod tests {
             CheckOutcome::Inconclusive,
             Coverage::Unknown,
             CheckSeverity::Blocking,
             vec![],
             "index_building",
             true,
             vec!["x".repeat(513)],
             vec![],
         )
         .is_err());
+        assert!(Check::new(
+            "source_readiness",
+            "DefinitionPort",
+            CheckState::Skipped,
+            CheckOutcome::Inconclusive,
+            Coverage::Unknown,
+            CheckSeverity::Blocking,
+            vec![],
+            "unsupported_source_format",
+            false,
+            vec![],
+            vec![],
+        )
+        .is_err());
     }
 
     #[test]
-    fn checks_accept_exactly_the_six_evidence_port_names() {
+    fn checks_accept_evidence_ports_and_the_source_resolver_orchestration_port() {
         for port in EvidencePort::ALL {
             let name = port.wire_name();
             assert_eq!(EvidencePort::parse_wire_name(name), Some(port));
             assert!(Check::new(
                 "provider_contract",
                 name,
                 CheckState::Passed,
                 CheckOutcome::Satisfied,
                 Coverage::Complete,
                 CheckSeverity::Info,
                 vec![],
                 "ok",
                 false,
                 vec![],
                 vec![],
             )
             .is_ok());
         }
+        assert!(Check::new(
+            "source_readiness",
+            "ProjectSourceResolverPort",
+            CheckState::Skipped,
+            CheckOutcome::Inconclusive,
+            Coverage::Unknown,
+            CheckSeverity::Blocking,
+            vec![],
+            "unsupported_source_format",
+            false,
+            vec![],
+            vec![],
+        )
+        .is_ok());
+        assert!(Check::new(
+            "provider_contract",
+            "ProjectSourceResolverPort",
+            CheckState::Skipped,
+            CheckOutcome::Inconclusive,
+            Coverage::Unknown,
+            CheckSeverity::Blocking,
+            vec![],
+            "unsupported_source_format",
+            false,
+            vec![],
+            vec![],
+        )
+        .is_err());
+        assert!(Check::new(
+            "source_readiness",
+            "ProjectSourceResolverPort",
+            CheckState::Failed,
+            CheckOutcome::Inconclusive,
+            Coverage::Unknown,
+            CheckSeverity::Blocking,
+            vec!["proposal:p".into()],
+            "unsupported_source_format",
+            false,
+            vec![],
+            vec![],
+        )
+        .is_err());
+        assert!(Check::new(
+            "source_readiness",
+            "ProjectSourceResolverPort",
+            CheckState::Skipped,
+            CheckOutcome::Inconclusive,
+            Coverage::Unknown,
+            CheckSeverity::Blocking,
+            vec!["candidate:not-allowed".into()],
+            "unsupported_source_format",
+            false,
+            vec![],
+            vec![],
+        )
+        .is_err());
         for provider in ["SyntheticProvider", "definitionport", "DefinitionPortTypo"] {
             assert!(Check::new(
                 "provider_contract",
                 provider,
                 CheckState::Passed,
                 CheckOutcome::Satisfied,
                 Coverage::Complete,
                 CheckSeverity::Info,
                 vec![],
                 "ok",
diff --git a/crates/unica-coder/src/application/discovery/model.rs b/crates/unica-coder/src/application/discovery/model.rs
index 04c5793..a556868 100644
--- a/crates/unica-coder/src/application/discovery/model.rs
+++ b/crates/unica-coder/src/application/discovery/model.rs
@@ -1027,22 +1027,56 @@ impl Check {
             retryable,
             details,
             evidence_ids,
         };
         check.validate()?;
         Ok(check)
     }
 
     pub(crate) fn validate(&self) -> Result<(), String> {
         stable_code(&self.code, "check.code")?;
-        if EvidencePort::parse_wire_name(&self.provider).is_none() {
-            return Err("check.provider must name one of the six evidence ports".to_string());
+        let evidence_provider = EvidencePort::parse_wire_name(&self.provider).is_some()
+            && self.code != "source_readiness";
+        let source_readiness_provider =
+            self.provider == "ProjectSourceResolverPort" && self.code == "source_readiness";
+        if !evidence_provider && !source_readiness_provider {
+            return Err(
+                "check.provider must name an evidence port or ProjectSourceResolverPort"
+                    .to_string(),
+            );
+        }
+        if source_readiness_provider {
+            let exact_state = self.state == CheckState::Skipped
+                && self.outcome == CheckOutcome::Inconclusive
+                && self.coverage == Coverage::Unknown
+                && self.severity == CheckSeverity::Blocking
+                && self.reason_code == "unsupported_source_format"
+                && !self.retryable
+                && self.details.is_empty()
+                && self.evidence_ids.is_empty();
+            if !exact_state {
+                return Err(
+                    "source_readiness resolver check must use the canonical skipped tuple"
+                        .to_string(),
+                );
+            }
+            if self
+                .affects
+                .iter()
+                .any(|target| target.strip_prefix("proposal:").is_none_or(str::is_empty))
+                || self.affects.windows(2).any(|pair| pair[0] >= pair[1])
+            {
+                return Err(
+                    "source_readiness resolver check affects must be canonical proposal ids"
+                        .to_string(),
+                );
+            }
         }
         validate_bounded_list(self.affects.clone(), "check.affects", 128, 256)?;
         stable_code(&self.reason_code, "check.reasonCode")?;
         if self.details.len() > 32 {
             return Err("check details must contain at most 32 entries".to_string());
         }
         for detail in &self.details {
             stable_component(detail, "check.details", 512)?;
         }
         validate_bounded_list(self.evidence_ids.clone(), "check.evidenceIds", 2000, 80)?;
diff --git a/crates/unica-coder/src/application/discovery/ports.rs b/crates/unica-coder/src/application/discovery/ports.rs
index 0b60e9f..495867b 100644
--- a/crates/unica-coder/src/application/discovery/ports.rs
+++ b/crates/unica-coder/src/application/discovery/ports.rs
@@ -1,29 +1,274 @@
 use super::contract::{DiscoverRequest, Proposal};
 use super::determinism::evidence_record_digest;
 use super::model::{
     CheckState, Coverage, EvidencePort, EvidenceProvider, EvidenceRecord, ProviderFact,
     ProviderOutcomeSnapshot, ProviderReadiness, ReceiptEligibility,
 };
-use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSnapshot};
+use crate::domain::source_snapshot::{
+    ResolvedSourceSelection, ResolvedSourceSet, SourceReadError, SourceSetSnapshot, SourceSnapshot,
+};
 use std::fmt;
 
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub(crate) enum DiscoveryError {
     Operation(String),
+    SourceReadiness(SourceReadinessError),
+    SnapshotCapture(SnapshotCaptureError),
     ProviderContractViolation { provider: String, reason: String },
 }
 
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub(crate) enum SnapshotCaptureReason {
+    SourceChangedDuringCapture,
+    UnsafeSourceTopology,
+    SnapshotDeadlineExceeded,
+    TransientSourceIo,
+    MalformedSourceMaterial,
+    UnsupportedSourceLayout,
+    InvalidSourcePath,
+    SnapshotResourceLimit,
+    SnapshotInvariantViolation,
+}
+
+impl SnapshotCaptureReason {
+    pub(crate) fn reason_code(self) -> &'static str {
+        match self {
+            Self::SourceChangedDuringCapture => "source_changed_during_capture",
+            Self::UnsafeSourceTopology => "unsafe_source_topology",
+            Self::SnapshotDeadlineExceeded => "source_snapshot_deadline",
+            Self::TransientSourceIo => "source_io_unavailable",
+            Self::MalformedSourceMaterial => "malformed_source_material",
+            Self::UnsupportedSourceLayout => "unsupported_source_layout",
+            Self::InvalidSourcePath => "invalid_source_path",
+            Self::SnapshotResourceLimit => "source_snapshot_resource_limit",
+            Self::SnapshotInvariantViolation => "source_snapshot_invariant_violation",
+        }
+    }
+
+    fn retryable(self) -> bool {
+        matches!(
+            self,
+            Self::SourceChangedDuringCapture
+                | Self::SnapshotDeadlineExceeded
+                | Self::TransientSourceIo
+        )
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct SnapshotCaptureError {
+    pub(crate) reason: SnapshotCaptureReason,
+    pub(crate) retryable: bool,
+    pub(crate) detail: String,
+}
+
+impl SnapshotCaptureError {
+    pub(crate) fn new(reason: SnapshotCaptureReason, detail: impl Into<String>) -> Self {
+        Self {
+            reason,
+            retryable: reason.retryable(),
+            detail: detail.into(),
+        }
+    }
+
+    pub(crate) fn classify(detail: impl Into<String>) -> Self {
+        let detail = detail.into();
+        let reason = if has_any_prefix(
+            &detail,
+            &["source_mapping_changed:", "source_snapshot_unavailable:"],
+        ) {
+            SnapshotCaptureReason::SourceChangedDuringCapture
+        } else if detail.starts_with("source_snapshot_deadline:") {
+            SnapshotCaptureReason::SnapshotDeadlineExceeded
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "source_snapshot_file_limit:",
+                "source_snapshot_byte_limit:",
+                "source_snapshot_traversal_limit:",
+                "source_snapshot_traversal_depth:",
+                "source_map_config_too_large:",
+            ],
+        ) {
+            SnapshotCaptureReason::SnapshotResourceLimit
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "source_root_symlink:",
+                "source_root_escape:",
+                "symlink_or_reparse_escape:",
+                "material_file_not_regular:",
+                "material_subtree_not_directory:",
+                "file_identity_unavailable:",
+                "source_map_config_not_regular:",
+                "symlink_or_reparse_marker:",
+            ],
+        ) {
+            SnapshotCaptureReason::UnsafeSourceTopology
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "malformed_registration:",
+                "malformed_registered_object:",
+                "malformed_descriptor:",
+                "duplicate_registration:",
+                "duplicate_nested_registration:",
+                "invalid_registration_value:",
+                "registered_object_identity_mismatch:",
+                "registered_material_missing:",
+                "unknown_registration_kind:",
+            ],
+        ) {
+            SnapshotCaptureReason::MalformedSourceMaterial
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "empty_configured_path:",
+                "absolute_source_root:",
+                "invalid_configured_path:",
+                "empty_path_component:",
+                "path_traversal:",
+                "embedded_current_dir:",
+                "path_escape:",
+                "invalid_path_component:",
+                "invalid_material_path:",
+                "non_utf8_material_path:",
+            ],
+        ) {
+            SnapshotCaptureReason::InvalidSourcePath
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "unsupported_source_format:",
+                "source_root_not_directory:",
+                "workspace_root_not_directory:",
+            ],
+        ) {
+            SnapshotCaptureReason::UnsupportedSourceLayout
+        } else if has_any_prefix(
+            &detail,
+            &[
+                "workspace_root_unavailable:",
+                "source_root_unavailable:",
+                "source_root_unreadable:",
+                "source_map_config_unavailable:",
+                "marker_unavailable:",
+                "material_file_unavailable:",
+                "material_file_unreadable:",
+                "material_subtree_unavailable:",
+                "material_subtree_unreadable:",
+                "path_unavailable:",
+            ],
+        ) {
+            SnapshotCaptureReason::TransientSourceIo
+        } else {
+            SnapshotCaptureReason::SnapshotInvariantViolation
+        };
+        Self::new(reason, detail)
+    }
+
+    pub(crate) fn source_changed(detail: impl Into<String>) -> Self {
+        Self::new(SnapshotCaptureReason::SourceChangedDuringCapture, detail)
+    }
+
+    pub(crate) fn reason_code(&self) -> &'static str {
+        self.reason.reason_code()
+    }
+
+    pub(crate) fn retryable(&self) -> bool {
+        self.retryable
+    }
+}
+
+fn has_any_prefix(value: &str, prefixes: &[&str]) -> bool {
+    prefixes.iter().any(|prefix| value.starts_with(prefix))
+}
+
+impl From<SnapshotCaptureError> for DiscoveryError {
+    fn from(error: SnapshotCaptureError) -> Self {
+        Self::SnapshotCapture(error)
+    }
+}
+
+impl From<String> for SnapshotCaptureError {
+    fn from(detail: String) -> Self {
+        Self::classify(detail)
+    }
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub(crate) enum SourceReadinessReason {
+    UnknownSourceFormat,
+    InvalidSourceFormat,
+    UnsupportedSourceKind,
+    UnsupportedSourceFormat,
+    UnsupportedDestinationKind,
+    UnsupportedDestinationFormat,
+}
+
+impl SourceReadinessReason {
+    pub(crate) fn reason_code(self) -> &'static str {
+        match self {
+            Self::UnknownSourceFormat => "unknown_source_format",
+            Self::InvalidSourceFormat => "invalid_source_format",
+            Self::UnsupportedSourceKind => "unsupported_source_kind",
+            Self::UnsupportedSourceFormat => "unsupported_source_format",
+            Self::UnsupportedDestinationKind => "unsupported_destination_kind",
+            Self::UnsupportedDestinationFormat => "unsupported_destination_format",
+        }
+    }
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub(crate) enum SourceRole {
+    Analysis,
+    Destination,
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct SourceReadinessError {
+    pub(crate) reason: SourceReadinessReason,
+    pub(crate) role: SourceRole,
+    pub(crate) source_set: String,
+    pub(crate) retryable: bool,
+}
+
+impl SourceReadinessError {
+    pub(crate) fn new(reason: SourceReadinessReason, role: SourceRole, source_set: &str) -> Self {
+        Self {
+            reason,
+            role,
+            source_set: source_set.to_string(),
+            retryable: false,
+        }
+    }
+
+    pub(crate) fn reason_code(&self) -> &'static str {
+        self.reason.reason_code()
+    }
+
+    pub(crate) fn retryable(&self) -> bool {
+        self.retryable
+    }
+}
+
 impl fmt::Display for DiscoveryError {
     fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
         match self {
             Self::Operation(message) => formatter.write_str(message),
+            Self::SourceReadiness(error) => {
+                write!(formatter, "{}: {}", error.reason_code(), error.source_set)
+            }
+            Self::SnapshotCapture(error) => {
+                write!(formatter, "{}: {}", error.reason_code(), error.detail)
+            }
             Self::ProviderContractViolation { provider, reason } => {
                 write!(formatter, "{provider} contract violation: {reason}")
             }
         }
     }
 }
 
 impl std::error::Error for DiscoveryError {}
 
 #[derive(Debug, Clone, PartialEq, Eq)]
@@ -154,20 +399,33 @@ impl ProviderOutcome<EvidenceRecord> {
             ),
             Self::Failed(issue) => collect_issue(
                 expected_port,
                 issue,
                 CheckState::Failed,
                 ProviderReadiness::Failed,
             ),
             Self::ContractViolation(reason) => Err(contract_error(expected_port, reason)),
         }
     }
+
+    pub(crate) fn collect_for_snapshot(
+        self,
+        expected_port: EvidencePort,
+        snapshot: &SourceSnapshot,
+    ) -> Result<CollectedProviderOutcome, DiscoveryError> {
+        let mut collected = self.collect(expected_port)?;
+        validate_freshness_against_snapshot(expected_port, &collected.records, snapshot)?;
+        for record in &mut collected.records {
+            record.freshness.workspace_epoch = snapshot.workspace_epoch;
+        }
+        Ok(collected)
+    }
 }
 
 #[derive(Debug, Clone)]
 pub(crate) struct CollectedProviderOutcome {
     pub(crate) port: EvidencePort,
     pub(crate) provider: EvidenceProvider,
     pub(crate) records: Vec<EvidenceRecord>,
     pub(crate) state: CheckState,
     pub(crate) coverage: Coverage,
     pub(crate) reason_code: Option<String>,
@@ -272,20 +530,42 @@ fn validate_expected_batch(
     }
     for record in &batch.records {
         validate_fact_for_port(expected_port, &record.fact)
             .map_err(|reason| contract_error(expected_port, reason))?;
         evidence_record_digest(record)
             .map_err(|error| contract_error(expected_port, error.to_string()))?;
     }
     Ok(())
 }
 
+fn validate_freshness_against_snapshot(
+    expected_port: EvidencePort,
+    records: &[EvidenceRecord],
+    snapshot: &SourceSnapshot,
+) -> Result<(), DiscoveryError> {
+    for record in records {
+        let Some(linked) = snapshot.snapshot_named(&record.freshness.source_set) else {
+            return Err(contract_error(
+                expected_port,
+                "evidence freshness names a source set outside the captured snapshot".into(),
+            ));
+        };
+        if linked.source_fingerprint != record.freshness.source_fingerprint {
+            return Err(contract_error(
+                expected_port,
+                "evidence freshness does not match the captured source identity".into(),
+            ));
+        }
+    }
+    Ok(())
+}
+
 fn validate_fact_for_port(port: EvidencePort, fact: &ProviderFact) -> Result<(), String> {
     let allowed = match port {
         EvidencePort::MetadataCatalog => matches!(
             fact,
             ProviderFact::MetadataPresent { .. }
                 | ProviderFact::MetadataAbsent { .. }
                 | ProviderFact::PlatformCallback { .. }
                 | ProviderFact::Binding { .. }
         ),
         EvidencePort::CodeSearch => matches!(fact, ProviderFact::CodeOccurrence { .. }),
@@ -359,48 +639,76 @@ fn contract_error(port: EvidencePort, reason: String) -> DiscoveryError {
         reason,
     }
 }
 
 macro_rules! evidence_port {
     ($name:ident, $method:ident) => {
         pub(crate) trait $name {
             fn $method(
                 &self,
                 plan: &DiscoveryQueryPlan,
-                context: &DiscoveryExecutionContext,
+                context: &EvidenceExecutionContext<'_>,
             ) -> ProviderOutcome<EvidenceRecord>;
         }
     };
 }
 
 evidence_port!(MetadataCatalogPort, metadata);
 evidence_port!(CodeSearchPort, search);
 evidence_port!(DefinitionPort, definitions);
 evidence_port!(CallGraphPort, calls);
 evidence_port!(FormInspectionPort, forms);
 evidence_port!(SupportStatePort, support);
 
+pub(crate) struct EvidenceExecutionContext<'a> {
+    pub(crate) workspace: &'a DiscoveryExecutionContext,
+    pub(crate) snapshot: &'a SourceSnapshot,
+    pub(crate) source_reader: &'a dyn SourceSnapshotPort,
+}
+
 pub(crate) trait ProjectSourceResolverPort {
-    fn resolve(
+    fn resolve_all(
         &self,
         context: &DiscoveryExecutionContext,
-        requested_source_set: Option<&str>,
-    ) -> Result<ResolvedSourceSet, DiscoveryError>;
+        requested_analysis: Option<&str>,
+        requested_mutations: &[String],
+    ) -> Result<ResolvedSourceSelection, DiscoveryError>;
 }
 
 pub(crate) trait SourceSnapshotPort {
     fn capture(
         &self,
         analysis: &ResolvedSourceSet,
         mutation_sources: &[ResolvedSourceSet],
         workspace_epoch: u64,
-    ) -> Result<SourceSnapshot, DiscoveryError>;
+    ) -> Result<SourceSnapshot, SnapshotCaptureError>;
+
+    fn read_verified(
+        &self,
+        snapshot: &SourceSetSnapshot,
+        workspace_relative_path: &str,
+    ) -> Result<Vec<u8>, SourceReadError> {
+        let _ = snapshot;
+        Err(SourceReadError::SnapshotUnavailable {
+            path: workspace_relative_path.to_string(),
+            detail: "snapshot reader is not implemented".into(),
+        })
+    }
+
+    fn read_optional_verified(
+        &self,
+        snapshot: &SourceSetSnapshot,
+        workspace_relative_path: &str,
+    ) -> Result<Option<Vec<u8>>, SourceReadError> {
+        self.read_verified(snapshot, workspace_relative_path)
+            .map(Some)
+    }
 }
 
 pub(crate) struct ReceiptIssuanceRequest<'a> {
     pub(crate) proposals: &'a [Proposal],
     pub(crate) snapshot: &'a SourceSnapshot,
 }
 
 pub(crate) trait ReceiptIssuerPort {
     fn assess(
         &self,
@@ -419,27 +727,128 @@ impl ReceiptIssuerPort for NoopReceiptIssuer {
             eligible: false,
             blockers: vec!["receipt_store_not_implemented".to_string()],
         })
     }
 }
 
 #[cfg(test)]
 mod tests {
     use super::*;
     use crate::application::discovery::contract::{ArtifactKind, ArtifactRef, ExecutionContext};
+    use crate::application::discovery::determinism::canonicalize_evidence;
     use crate::application::discovery::model::{
         BindingDetails, FlowKind, Freshness, HttpVerb, ProviderFact,
     };
+    use crate::domain::project_sources::{SourceFormat, SourceSetKind};
+    use crate::domain::source_snapshot::{
+        ManifestEntry, MaterialFile, ResolvedSourceSet, SourceManifest, SourceSetSnapshot,
+        SourceSnapshot,
+    };
+    use std::collections::BTreeMap;
 
     const FINGERPRINT: &str =
         "sha256:1111111111111111111111111111111111111111111111111111111111111111";
 
+    #[test]
+    fn snapshot_capture_retry_matrix_is_stable_and_typed() {
+        let reasons = [
+            SnapshotCaptureReason::SourceChangedDuringCapture,
+            SnapshotCaptureReason::UnsafeSourceTopology,
+            SnapshotCaptureReason::SnapshotDeadlineExceeded,
+            SnapshotCaptureReason::TransientSourceIo,
+            SnapshotCaptureReason::MalformedSourceMaterial,
+            SnapshotCaptureReason::UnsupportedSourceLayout,
+            SnapshotCaptureReason::InvalidSourcePath,
+            SnapshotCaptureReason::SnapshotResourceLimit,
+            SnapshotCaptureReason::SnapshotInvariantViolation,
+        ];
+        assert_eq!(
+            reasons.map(SnapshotCaptureReason::reason_code),
+            [
+                "source_changed_during_capture",
+                "unsafe_source_topology",
+                "source_snapshot_deadline",
+                "source_io_unavailable",
+                "malformed_source_material",
+                "unsupported_source_layout",
+                "invalid_source_path",
+                "source_snapshot_resource_limit",
+                "source_snapshot_invariant_violation",
+            ]
+        );
+        assert_eq!(
+            reasons.map(SnapshotCaptureReason::retryable),
+            [true, false, true, true, false, false, false, false, false]
+        );
+        for (detail, reason, retryable) in [
+            (
+                "source_mapping_changed: source map changed during resolution",
+                SnapshotCaptureReason::SourceChangedDuringCapture,
+                true,
+            ),
+            (
+                "source_snapshot_unavailable: concurrent mutation",
+                SnapshotCaptureReason::SourceChangedDuringCapture,
+                true,
+            ),
+            (
+                "source_snapshot_deadline: authoritative snapshot discarded",
+                SnapshotCaptureReason::SnapshotDeadlineExceeded,
+                true,
+            ),
+            (
+                "material_file_unreadable: transient read failure",
+                SnapshotCaptureReason::TransientSourceIo,
+                true,
+            ),
+            (
+                "source_snapshot_file_limit: authoritative snapshot discarded",
+                SnapshotCaptureReason::SnapshotResourceLimit,
+                false,
+            ),
+            (
+                "source_snapshot_byte_limit: authoritative snapshot discarded",
+                SnapshotCaptureReason::SnapshotResourceLimit,
+                false,
+            ),
+            (
+                "source_snapshot_traversal_limit: authoritative snapshot discarded",
+                SnapshotCaptureReason::SnapshotResourceLimit,
+                false,
+            ),
+            (
+                "source_snapshot_traversal_depth: authoritative snapshot discarded",
+                SnapshotCaptureReason::SnapshotResourceLimit,
+                false,
+            ),
+            (
+                "symlink_or_reparse_escape: stable component",
+                SnapshotCaptureReason::UnsafeSourceTopology,
+                false,
+            ),
+            (
+                "malformed_registered_object: invalid XML",
+                SnapshotCaptureReason::MalformedSourceMaterial,
+                false,
+            ),
+            (
+                "unknown_registration_kind: FutureObject",
+                SnapshotCaptureReason::MalformedSourceMaterial,
+                false,
+            ),
+        ] {
+            let error = SnapshotCaptureError::classify(detail);
+            assert_eq!(error.reason, reason, "{detail}");
+            assert_eq!(error.retryable(), retryable, "{detail}");
+        }
+    }
+
     fn binding_outcome(
         port: EvidencePort,
         relation: FlowKind,
         details: BindingDetails,
     ) -> ProviderOutcome<EvidenceRecord> {
         let provider =
             EvidenceProvider::new(port, &format!("test-{}", port.wire_name()), "1").unwrap();
         let subject = ArtifactRef::parse(ArtifactKind::Module, "CommonModule.Entry").unwrap();
         let object = ArtifactRef::parse(ArtifactKind::Method, "CommonModule.Flow.Run").unwrap();
         ProviderOutcome::complete(
@@ -453,20 +862,111 @@ mod tests {
                 },
                 None,
                 provider,
                 Coverage::Complete,
                 Freshness::new("main", FINGERPRINT, 1).unwrap(),
             )],
         )
         .unwrap()
     }
 
+    fn captured_snapshot() -> SourceSnapshot {
+        let source = ResolvedSourceSet::new(
+            "main".into(),
+            SourceSetKind::Configuration,
+            "src".into(),
+            SourceFormat::PlatformXml,
+            format!("sha256:{}", "a".repeat(64)),
+        )
+        .unwrap();
+        let manifest = SourceManifest::new(BTreeMap::from([(
+            "src/Configuration.xml".into(),
+            ManifestEntry::Present(
+                MaterialFile::new(1, format!("sha256:{}", "b".repeat(64))).unwrap(),
+            ),
+        )]))
+        .unwrap();
+        SourceSnapshot::new(
+            SourceSetSnapshot::from_manifest(source, manifest).unwrap(),
+            vec![],
+            9,
+        )
+        .unwrap()
+    }
+
+    #[test]
+    fn freshness_binds_source_identity_but_epoch_is_diagnostic_only() {
+        let snapshot = captured_snapshot();
+        let fingerprint = snapshot.analysis.source_fingerprint.clone();
+        let mut older_epoch = binding_outcome(
+            EvidencePort::MetadataCatalog,
+            FlowKind::Contains,
+            BindingDetails::Structural,
+        );
+        let ProviderOutcome::Complete(batch) = &mut older_epoch else {
+            unreachable!()
+        };
+        batch.records[0].freshness = Freshness::new("main", &fingerprint, 1).unwrap();
+        assert!(older_epoch
+            .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
+            .is_ok());
+
+        let ProviderOutcome::Complete(mut batch) = binding_outcome(
+            EvidencePort::MetadataCatalog,
+            FlowKind::Contains,
+            BindingDetails::Structural,
+        ) else {
+            unreachable!()
+        };
+        batch.records[0].freshness = Freshness::new("main", &fingerprint, 1).unwrap();
+        let mut current = batch.records[0].clone();
+        current.freshness.workspace_epoch = snapshot.workspace_epoch;
+        let provider = batch.provider.clone();
+        let forward = ProviderOutcome::complete(
+            provider.clone(),
+            vec![batch.records[0].clone(), current.clone()],
+        )
+        .unwrap()
+        .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
+        .unwrap();
+        let reverse = ProviderOutcome::complete(provider, vec![current, batch.records.remove(0)])
+            .unwrap()
+            .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)
+            .unwrap();
+        let forward = canonicalize_evidence(forward.records).unwrap();
+        let reverse = canonicalize_evidence(reverse.records).unwrap();
+        assert_eq!(forward, reverse);
+        assert_eq!(forward.len(), 1);
+        assert_eq!(
+            forward[0].freshness.workspace_epoch,
+            snapshot.workspace_epoch
+        );
+
+        for (source_set, source_fingerprint) in
+            [("other", fingerprint.as_str()), ("main", FINGERPRINT)]
+        {
+            let mut invalid = binding_outcome(
+                EvidencePort::MetadataCatalog,
+                FlowKind::Contains,
+                BindingDetails::Structural,
+            );
+            let ProviderOutcome::Complete(batch) = &mut invalid else {
+                unreachable!()
+            };
+            batch.records[0].freshness = Freshness::new(source_set, source_fingerprint, 9).unwrap();
+            assert!(matches!(
+                invalid.collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot),
+                Err(DiscoveryError::ProviderContractViolation { .. })
+            ));
+        }
+    }
+
     fn binding_cases() -> Vec<(BindingDetails, Vec<(EvidencePort, FlowKind)>)> {
         vec![
             (
                 BindingDetails::Structural,
                 vec![
                     (EvidencePort::MetadataCatalog, FlowKind::Contains),
                     (EvidencePort::MetadataCatalog, FlowKind::Defines),
                 ],
             ),
             (
diff --git a/crates/unica-coder/src/application/discovery/use_case.rs b/crates/unica-coder/src/application/discovery/use_case.rs
index e1fffd9..9d95fe3 100644
--- a/crates/unica-coder/src/application/discovery/use_case.rs
+++ b/crates/unica-coder/src/application/discovery/use_case.rs
@@ -1,25 +1,28 @@
 use super::contract::{DiscoverMode, DiscoverRequest, MutationIntent};
 use super::determinism::{analysis_id, canonicalize_evidence, evidence_id};
 use super::evidence_graph::EvidenceGraph;
 use super::model::{
-    Check, CheckOutcome, CheckSeverity, DiscoveryReport, DiscoverySource, DiscoveryStatus,
-    EvidencePort, LinkedSourceSnapshot, ReceiptEligibility, SourceSnapshotRole, Verdict,
+    Check, CheckOutcome, CheckSeverity, CheckState, Coverage, DiscoveryReport, DiscoverySource,
+    DiscoveryStatus, EvidencePort, FactAnswer, LinkedSourceSnapshot, ProposalFacts,
+    ProposalVerdict, ReceiptEligibility, SourceSnapshotRole, SupportState, Verdict,
 };
 use super::ports::{
     CallGraphPort, CodeSearchPort, CollectedProviderOutcome, DefinitionPort, DiscoveryError,
-    DiscoveryExecutionContext, DiscoveryQueryPlan, FormInspectionPort, MetadataCatalogPort,
-    ProjectSourceResolverPort, ReceiptIssuanceRequest, ReceiptIssuerPort, SourceSnapshotPort,
-    SupportStatePort,
+    DiscoveryExecutionContext, DiscoveryQueryPlan, EvidenceExecutionContext, FormInspectionPort,
+    MetadataCatalogPort, ProjectSourceResolverPort, ReceiptIssuanceRequest, ReceiptIssuerPort,
+    SnapshotCaptureError, SnapshotCaptureReason, SourceReadinessError, SourceReadinessReason,
+    SourceRole, SourceSnapshotPort, SupportStatePort,
 };
 use super::proposal_validator::{ProposalValidation, ProposalValidator};
-use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSnapshot};
+use crate::domain::project_sources::{SourceFormat, SourceSetKind};
+use crate::domain::source_snapshot::{ResolvedSourceSelection, ResolvedSourceSet, SourceSnapshot};
 use std::collections::BTreeSet;
 
 const ANALYSIS_CONTRACT_VERSION: &str = "project-discovery-v1";
 
 pub(crate) struct DiscoverExtensionPointsUseCase<'a> {
     source_resolver: &'a dyn ProjectSourceResolverPort,
     snapshot_port: &'a dyn SourceSnapshotPort,
     metadata_catalog: &'a dyn MetadataCatalogPort,
     code_search: &'a dyn CodeSearchPort,
     definitions: &'a dyn DefinitionPort,
@@ -58,60 +61,78 @@ impl<'a> DiscoverExtensionPointsUseCase<'a> {
     pub(crate) fn execute(
         &self,
         context: DiscoveryExecutionContext,
         request: DiscoverRequest,
     ) -> Result<DiscoveryReport, DiscoveryError> {
         if context.workspace_root.trim().is_empty() {
             return Err(DiscoveryError::Operation(
                 "workspace root must not be blank".to_string(),
             ));
         }
-        let analysis_source = self
-            .source_resolver
-            .resolve(&context, request.source_set.as_deref())?;
+        let mutation_names = mutation_source_names(&request);
+        let resolved_sources = self.source_resolver.resolve_all(
+            &context,
+            request.source_set.as_deref(),
+            &mutation_names,
+        )?;
+        validate_resolved_source_roles(&resolved_sources)?;
+        let analysis_source = resolved_sources.analysis;
         analysis_source
             .validate()
             .map_err(DiscoveryError::Operation)?;
-        let mutation_sources = self.resolve_mutation_sources(&context, &request)?;
-        let snapshot = self.snapshot_port.capture(
-            &analysis_source,
-            &mutation_sources,
-            context.workspace_epoch,
-        )?;
-        snapshot.validate().map_err(DiscoveryError::Operation)?;
-        validate_captured_snapshot(
-            &snapshot,
+        let mutation_sources = resolved_sources.mutations;
+        let captured_mutation_sources = if analysis_source.source_format
+            == crate::domain::project_sources::SourceFormat::PlatformXml
+        {
+            mutation_sources.as_slice()
+        } else {
+            &[]
+        };
+        let mut snapshot = self.snapshot_port.capture(
             &analysis_source,
-            &mutation_sources,
+            captured_mutation_sources,
             context.workspace_epoch,
         )?;
+        snapshot.validate().map_err(snapshot_invariant_error)?;
+        validate_captured_snapshot(&snapshot, &analysis_source, captured_mutation_sources)?;
+        snapshot.workspace_epoch = context.workspace_epoch;
 
         let plan = DiscoveryQueryPlan::normalized(&request);
+        if snapshot.analysis.source_set.source_format
+            != crate::domain::project_sources::SourceFormat::PlatformXml
+        {
+            return unsupported_source_format_report(&plan, &snapshot);
+        }
+        let evidence_context = EvidenceExecutionContext {
+            workspace: &context,
+            snapshot: &snapshot,
+            source_reader: self.snapshot_port,
+        };
         let providers = vec![
             self.metadata_catalog
-                .metadata(&plan, &context)
-                .collect(EvidencePort::MetadataCatalog)?,
+                .metadata(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::MetadataCatalog, &snapshot)?,
             self.code_search
-                .search(&plan, &context)
-                .collect(EvidencePort::CodeSearch)?,
+                .search(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::CodeSearch, &snapshot)?,
             self.definitions
-                .definitions(&plan, &context)
-                .collect(EvidencePort::Definition)?,
+                .definitions(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::Definition, &snapshot)?,
             self.call_graph
-                .calls(&plan, &context)
-                .collect(EvidencePort::CallGraph)?,
+                .calls(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::CallGraph, &snapshot)?,
             self.form_inspection
-                .forms(&plan, &context)
-                .collect(EvidencePort::FormInspection)?,
+                .forms(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::FormInspection, &snapshot)?,
             self.support_state
-                .support(&plan, &context)
-                .collect(EvidencePort::SupportState)?,
+                .support(&plan, &evidence_context)
+                .collect_for_snapshot(EvidencePort::SupportState, &snapshot)?,
         ];
         let records = providers
             .iter()
             .flat_map(|provider| provider.records.iter().cloned())
             .collect::<Vec<_>>();
         let graph = EvidenceGraph::build(&records).map_err(DiscoveryError::Operation)?;
         let validation = ProposalValidator::validate(&plan.request.proposals, &graph, &providers)
             .map_err(DiscoveryError::Operation)?;
         let checks = build_checks(&providers, &graph, &validation)?;
         let status = report_status(plan.request.mode, &graph, &validation, &checks);
@@ -139,49 +160,35 @@ impl<'a> DiscoverExtensionPointsUseCase<'a> {
             graph.flow_edges,
             graph.candidates,
             validation.verdicts,
             evidence,
             checks,
             receipt_eligibility,
         )
         .map_err(DiscoveryError::Operation)
     }
 
-    fn resolve_mutation_sources(
-        &self,
-        context: &DiscoveryExecutionContext,
-        request: &DiscoverRequest,
-    ) -> Result<Vec<ResolvedSourceSet>, DiscoveryError> {
-        let names = request
-            .proposals
-            .iter()
-            .filter_map(|proposal| proposal.mutation_intent.as_ref())
-            .map(|intent| match intent {
-                MutationIntent::CfePatchMethod {
-                    destination_source_set,
-                    ..
-                } => destination_source_set.clone(),
-            })
-            .collect::<BTreeSet<_>>();
-        names
-            .iter()
-            .map(|name| self.source_resolver.resolve(context, Some(name)))
-            .collect()
-    }
-
     fn receipt_eligibility(
         &self,
         request: &DiscoverRequest,
         snapshot: &SourceSnapshot,
         validation: &ProposalValidation,
         checks: &[Check],
     ) -> Result<ReceiptEligibility, DiscoveryError> {
+        if snapshot.analysis.source_set.source_format
+            != crate::domain::project_sources::SourceFormat::PlatformXml
+        {
+            return Ok(ReceiptEligibility {
+                eligible: false,
+                blockers: vec!["unsupported_source_format".to_string()],
+            });
+        }
         let all_supported = request.mode == DiscoverMode::Validate
             && !validation.verdicts.is_empty()
             && validation.verdicts.iter().all(|verdict| {
                 verdict.verdict == Verdict::Supported
                     && verdict.coverage_gaps.is_empty()
                     && verdict.blockers.is_empty()
             });
         let material_blocker = checks.iter().any(|check| {
             check.severity == CheckSeverity::Blocking
                 && !matches!(
@@ -205,51 +212,188 @@ impl<'a> DiscoverExtensionPointsUseCase<'a> {
                 blockers: blockers.into_iter().collect(),
             });
         }
         self.receipt_issuer.assess(&ReceiptIssuanceRequest {
             proposals: &request.proposals,
             snapshot,
         })
     }
 }
 
+fn validate_resolved_source_roles(
+    selection: &ResolvedSourceSelection,
+) -> Result<(), DiscoveryError> {
+    selection.validate().map_err(DiscoveryError::Operation)?;
+    let analysis = &selection.analysis;
+    if !matches!(
+        analysis.kind,
+        SourceSetKind::Configuration | SourceSetKind::Extension
+    ) {
+        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnsupportedSourceKind,
+            SourceRole::Analysis,
+            &analysis.name,
+        )));
+    }
+    match analysis.source_format {
+        SourceFormat::PlatformXml => {}
+        SourceFormat::Edt if analysis.kind == SourceSetKind::Configuration => {}
+        SourceFormat::Edt => {
+            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+                SourceReadinessReason::UnsupportedSourceFormat,
+                SourceRole::Analysis,
+                &analysis.name,
+            )));
+        }
+        SourceFormat::Unknown => {
+            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+                SourceReadinessReason::UnknownSourceFormat,
+                SourceRole::Analysis,
+                &analysis.name,
+            )));
+        }
+        SourceFormat::Invalid => {
+            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+                SourceReadinessReason::InvalidSourceFormat,
+                SourceRole::Analysis,
+                &analysis.name,
+            )));
+        }
+    }
+    for mutation in &selection.mutations {
+        mutation.validate().map_err(DiscoveryError::Operation)?;
+        if mutation.kind != SourceSetKind::Extension {
+            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+                SourceReadinessReason::UnsupportedDestinationKind,
+                SourceRole::Destination,
+                &mutation.name,
+            )));
+        }
+        if mutation.source_format != SourceFormat::PlatformXml {
+            return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+                SourceReadinessReason::UnsupportedDestinationFormat,
+                SourceRole::Destination,
+                &mutation.name,
+            )));
+        }
+    }
+    Ok(())
+}
+
+fn mutation_source_names(request: &DiscoverRequest) -> Vec<String> {
+    request
+        .proposals
+        .iter()
+        .filter_map(|proposal| proposal.mutation_intent.as_ref())
+        .map(|intent| match intent {
+            MutationIntent::CfePatchMethod {
+                destination_source_set,
+                ..
+            } => destination_source_set.clone(),
+        })
+        .collect::<BTreeSet<_>>()
+        .into_iter()
+        .collect()
+}
+
+fn unsupported_source_format_report(
+    plan: &DiscoveryQueryPlan,
+    snapshot: &SourceSnapshot,
+) -> Result<DiscoveryReport, DiscoveryError> {
+    let source = discovery_source(snapshot);
+    let mut affects = plan
+        .request
+        .proposals
+        .iter()
+        .map(|proposal| format!("proposal:{}", proposal.id))
+        .collect::<Vec<_>>();
+    affects.sort();
+    let checks = vec![Check::new(
+        "source_readiness",
+        "ProjectSourceResolverPort",
+        CheckState::Skipped,
+        CheckOutcome::Inconclusive,
+        Coverage::Unknown,
+        CheckSeverity::Blocking,
+        affects,
+        "unsupported_source_format",
+        false,
+        Vec::new(),
+        Vec::new(),
+    )
+    .map_err(DiscoveryError::Operation)?];
+    let proposal_verdicts = plan
+        .request
+        .proposals
+        .iter()
+        .map(|proposal| ProposalVerdict {
+            proposal_id: proposal.id.clone(),
+            verdict: Verdict::Unknown,
+            facts: ProposalFacts {
+                exists: FactAnswer::Unknown,
+                runtime_reachable: FactAnswer::Unknown,
+                support: SupportState::Unknown,
+            },
+            evidence_ids: Vec::new(),
+            coverage_gaps: vec!["unsupported_source_format".into()],
+            blockers: vec!["unsupported_source_format".into()],
+        })
+        .collect::<Vec<_>>();
+    let analysis_id = analysis_id(&plan.request, ANALYSIS_CONTRACT_VERSION, &source, &[])
+        .map_err(|error| DiscoveryError::Operation(error.to_string()))?;
+    DiscoveryReport::new(
+        DiscoveryStatus::Insufficient,
+        analysis_id,
+        source,
+        Vec::new(),
+        Vec::new(),
+        Vec::new(),
+        proposal_verdicts,
+        Vec::new(),
+        checks,
+        ReceiptEligibility {
+            eligible: false,
+            blockers: vec!["unsupported_source_format".into()],
+        },
+    )
+    .map_err(DiscoveryError::Operation)
+}
+
 fn validate_captured_snapshot(
     snapshot: &SourceSnapshot,
     analysis_source: &ResolvedSourceSet,
     mutation_sources: &[ResolvedSourceSet],
-    workspace_epoch: u64,
-) -> Result<(), DiscoveryError> {
-    if snapshot.workspace_epoch != workspace_epoch {
-        return Err(DiscoveryError::Operation(
-            "captured snapshot workspace epoch differs from the requested epoch".to_string(),
-        ));
-    }
+) -> Result<(), SnapshotCaptureError> {
     if snapshot.analysis.source_set != *analysis_source {
-        return Err(DiscoveryError::Operation(
+        return Err(snapshot_invariant_error(
             "captured analysis snapshot identity differs from the resolved source".to_string(),
         ));
     }
     if snapshot.mutations.len() != mutation_sources.len()
         || mutation_sources.iter().any(|expected| {
             !snapshot
                 .mutations
                 .iter()
                 .any(|actual| actual.source_set == *expected)
         })
     {
-        return Err(DiscoveryError::Operation(
+        return Err(snapshot_invariant_error(
             "captured mutation snapshot identities differ from the resolved sources".to_string(),
         ));
     }
     Ok(())
 }
 
+fn snapshot_invariant_error(detail: String) -> SnapshotCaptureError {
+    SnapshotCaptureError::new(SnapshotCaptureReason::SnapshotInvariantViolation, detail)
+}
+
 fn discovery_source(snapshot: &SourceSnapshot) -> DiscoverySource {
     let mut linked_source_snapshots = vec![LinkedSourceSnapshot {
         source_set: snapshot.analysis.source_set.name.clone(),
         role: SourceSnapshotRole::Analysis,
         source_fingerprint: snapshot.analysis.source_fingerprint.clone(),
     }];
     linked_source_snapshots.extend(snapshot.mutations.iter().map(|mutation| {
         LinkedSourceSnapshot {
             source_set: mutation.source_set.name.clone(),
             role: SourceSnapshotRole::Mutation,
@@ -420,27 +564,65 @@ fn report_status(
 mod tests {
     use super::*;
     use crate::application::discovery::contract::{ArtifactKind, ArtifactRef, DiscoverRequest};
     use crate::application::discovery::model::{
         BindingDetails, Coverage, DiscoveryStatus, EvidenceLevel, EvidencePort, EvidenceProvider,
         EvidenceRecord, FactAnswer, FlowKind, Freshness, PlatformCallbackShape, ProviderFact,
         ReceiptEligibility, SourceLocation, SupportState, Verdict,
     };
     use crate::application::discovery::ports::*;
     use crate::domain::project_sources::{SourceFormat, SourceSetKind};
-    use crate::domain::source_snapshot::{ResolvedSourceSet, SourceSetSnapshot, SourceSnapshot};
+    use crate::domain::source_snapshot::{
+        ManifestEntry, MaterialFile, ResolvedSourceSelection, ResolvedSourceSet, SourceManifest,
+        SourceSetSnapshot, SourceSnapshot,
+    };
     use serde_json::json;
+    use std::collections::BTreeMap;
+    use std::sync::atomic::{AtomicUsize, Ordering};
+
+    fn fake_resolved(name: &str, mutation: bool) -> ResolvedSourceSet {
+        ResolvedSourceSet::new(
+            name.into(),
+            if mutation {
+                SourceSetKind::Extension
+            } else {
+                SourceSetKind::Configuration
+            },
+            if mutation {
+                "src-cfe".into()
+            } else {
+                "src".into()
+            },
+            SourceFormat::PlatformXml,
+            format!("sha256:{}", "a".repeat(64)),
+        )
+        .unwrap()
+    }
 
-    const FINGERPRINT: &str =
-        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
-    const COMPOSITE: &str =
-        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
+    fn fake_source_snapshot(source_set: ResolvedSourceSet) -> SourceSetSnapshot {
+        let path = if source_set.relative_root == "." {
+            "Configuration.xml".to_string()
+        } else {
+            format!("{}/Configuration.xml", source_set.relative_root)
+        };
+        SourceSetSnapshot::from_manifest(
+            source_set,
+            SourceManifest::new(BTreeMap::from([(
+                path,
+                ManifestEntry::Present(
+                    MaterialFile::new(1, format!("sha256:{}", "1".repeat(64))).unwrap(),
+                ),
+            )]))
+            .unwrap(),
+        )
+        .unwrap()
+    }
 
     fn artifact(kind: ArtifactKind, canonical_ref: &str) -> ArtifactRef {
         ArtifactRef::parse(kind, canonical_ref).unwrap()
     }
 
     fn target() -> ArtifactRef {
         artifact(ArtifactKind::Method, "CommonModule.Flow.Run")
     }
 
     fn owner() -> ArtifactRef {
@@ -506,21 +688,26 @@ mod tests {
     fn record_with_coverage(
         port: EvidencePort,
         fact: ProviderFact,
         coverage: Coverage,
     ) -> EvidenceRecord {
         EvidenceRecord::from_fact(
             fact,
             Some(SourceLocation::new("src/Flow.bsl", Some(1), Some(1)).unwrap()),
             EvidenceProvider::new(port, &format!("fake-{}", port.wire_name()), "1").unwrap(),
             coverage,
-            Freshness::new("main", FINGERPRINT, 7).unwrap(),
+            Freshness::new(
+                "main",
+                &fake_source_snapshot(fake_resolved("main", false)).source_fingerprint,
+                7,
+            )
+            .unwrap(),
         )
     }
 
     fn complete(
         port: EvidencePort,
         records: Vec<EvidenceRecord>,
     ) -> ProviderOutcome<EvidenceRecord> {
         ProviderOutcome::complete(
             EvidenceProvider::new(port, &format!("fake-{}", port.wire_name()), "1").unwrap(),
             records,
@@ -675,92 +862,115 @@ mod tests {
             }
         }
     }
 
     macro_rules! fake_port {
         ($trait_name:ident, $method:ident, $field:ident) => {
             impl $trait_name for FakeEvidencePorts {
                 fn $method(
                     &self,
                     _plan: &DiscoveryQueryPlan,
-                    _context: &DiscoveryExecutionContext,
+                    _context: &EvidenceExecutionContext<'_>,
                 ) -> ProviderOutcome<EvidenceRecord> {
                     self.$field.clone()
                 }
             }
         };
     }
 
     fake_port!(MetadataCatalogPort, metadata, metadata);
     fake_port!(CodeSearchPort, search, code);
     fake_port!(DefinitionPort, definitions, definitions);
     fake_port!(CallGraphPort, calls, calls);
     fake_port!(FormInspectionPort, forms, forms);
     fake_port!(SupportStatePort, support, support);
 
     struct FakeSourceResolver;
 
     impl ProjectSourceResolverPort for FakeSourceResolver {
-        fn resolve(
+        fn resolve_all(
             &self,
             _context: &DiscoveryExecutionContext,
-            requested_source_set: Option<&str>,
-        ) -> Result<ResolvedSourceSet, DiscoveryError> {
-            let name = requested_source_set.unwrap_or("main");
-            Ok(ResolvedSourceSet {
-                name: name.into(),
-                kind: if requested_source_set.is_some() {
-                    SourceSetKind::Extension
-                } else {
-                    SourceSetKind::Configuration
-                },
-                relative_root: if requested_source_set.is_some() {
-                    "src-cfe".into()
-                } else {
-                    "src".into()
-                },
-                source_format: SourceFormat::PlatformXml,
-                mapping_digest: if requested_source_set.is_some() {
-                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()
-                } else {
-                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()
-                },
-            })
+            requested_analysis: Option<&str>,
+            requested_mutations: &[String],
+        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
+            ResolvedSourceSelection::new(
+                fake_resolved(requested_analysis.unwrap_or("main"), false),
+                requested_mutations
+                    .iter()
+                    .map(|name| fake_resolved(name, true))
+                    .collect(),
+            )
+            .map_err(DiscoveryError::Operation)
+        }
+    }
+
+    struct FixedSourceResolver(ResolvedSourceSelection);
+
+    impl ProjectSourceResolverPort for FixedSourceResolver {
+        fn resolve_all(
+            &self,
+            _context: &DiscoveryExecutionContext,
+            _requested_analysis: Option<&str>,
+            _requested_mutations: &[String],
+        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
+            Ok(self.0.clone())
         }
     }
 
+    struct PanicSnapshotPort;
+
+    impl SourceSnapshotPort for PanicSnapshotPort {
+        fn capture(
+            &self,
+            _analysis: &ResolvedSourceSet,
+            _mutation_sources: &[ResolvedSourceSet],
+            _workspace_epoch: u64,
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+            panic!("invalid resolved source roles must be rejected before capture")
+        }
+    }
+
+    fn source_with_role_shape(
+        name: &str,
+        kind: SourceSetKind,
+        format: SourceFormat,
+    ) -> ResolvedSourceSet {
+        ResolvedSourceSet::new(
+            name.into(),
+            kind,
+            format!("src-{name}"),
+            format,
+            format!("sha256:{}", "a".repeat(64)),
+        )
+        .unwrap()
+    }
+
     struct FakeSnapshotPort;
 
     impl SourceSnapshotPort for FakeSnapshotPort {
         fn capture(
             &self,
             analysis: &ResolvedSourceSet,
             mutation_sources: &[ResolvedSourceSet],
             workspace_epoch: u64,
-        ) -> Result<SourceSnapshot, DiscoveryError> {
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
             SourceSnapshot::new(
-                SourceSetSnapshot {
-                    source_set: analysis.clone(),
-                    source_fingerprint: FINGERPRINT.into(),
-                },
+                fake_source_snapshot(analysis.clone()),
                 mutation_sources
                     .iter()
                     .cloned()
-                    .map(|source_set| SourceSetSnapshot {
-                        source_set,
-                        source_fingerprint: FINGERPRINT.into(),
-                    })
+                    .map(fake_source_snapshot)
                     .collect(),
-                COMPOSITE.into(),
                 workspace_epoch,
             )
-            .map_err(DiscoveryError::Operation)
+            .map_err(SnapshotCaptureError::classify)
         }
     }
 
     struct AllowReceiptIssuer;
 
     impl ReceiptIssuerPort for AllowReceiptIssuer {
         fn assess(
             &self,
             _request: &ReceiptIssuanceRequest<'_>,
         ) -> Result<ReceiptEligibility, DiscoveryError> {
@@ -1379,113 +1589,211 @@ mod tests {
             check.provider == "DefinitionPort"
                 && check.severity == crate::application::discovery::model::CheckSeverity::Blocking
                 && check.outcome == crate::application::discovery::model::CheckOutcome::Inconclusive
         }));
         assert_eq!(report.status, DiscoveryStatus::Insufficient);
         assert!(!report.receipt_eligibility.eligible);
     }
 
     struct AliasedAnalysisSnapshotPort;
 
+    struct ConcurrentCaptureFailure;
+
+    struct CorruptSnapshotPort;
+
+    struct OlderEpochSnapshotPort;
+
+    impl SourceSnapshotPort for OlderEpochSnapshotPort {
+        fn capture(
+            &self,
+            analysis: &ResolvedSourceSet,
+            mutation_sources: &[ResolvedSourceSet],
+            workspace_epoch: u64,
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+            FakeSnapshotPort.capture(
+                analysis,
+                mutation_sources,
+                workspace_epoch.saturating_sub(1),
+            )
+        }
+    }
+
+    impl SourceSnapshotPort for CorruptSnapshotPort {
+        fn capture(
+            &self,
+            analysis: &ResolvedSourceSet,
+            mutation_sources: &[ResolvedSourceSet],
+            workspace_epoch: u64,
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+            let mut snapshot =
+                FakeSnapshotPort.capture(analysis, mutation_sources, workspace_epoch)?;
+            snapshot.composite_fingerprint = format!("sha256:{}", "f".repeat(64));
+            Ok(snapshot)
+        }
+    }
+
+    impl SourceSnapshotPort for ConcurrentCaptureFailure {
+        fn capture(
+            &self,
+            _analysis: &ResolvedSourceSet,
+            _mutation_sources: &[ResolvedSourceSet],
+            _workspace_epoch: u64,
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+            Err(SnapshotCaptureError::source_changed(
+                "test source substitution",
+            ))
+        }
+    }
+
+    #[test]
+    fn snapshot_capture_error_type_survives_port_and_use_case_boundary() {
+        let result =
+            Fixture::positive().execute_with_snapshot(&ConcurrentCaptureFailure, method_proposal());
+
+        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
+            panic!("typed snapshot capture failure was not preserved");
+        };
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SourceChangedDuringCapture
+        );
+        assert_eq!(error.reason_code(), "source_changed_during_capture");
+        assert!(error.retryable());
+    }
+
+    #[test]
+    fn invalid_adapter_snapshot_is_a_typed_non_retryable_invariant_failure() {
+        let result =
+            Fixture::positive().execute_with_snapshot(&CorruptSnapshotPort, method_proposal());
+
+        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
+            panic!("invalid adapter snapshot was not a typed snapshot failure");
+        };
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SnapshotInvariantViolation
+        );
+        assert_eq!(error.reason_code(), "source_snapshot_invariant_violation");
+        assert!(!error.retryable());
+    }
+
+    #[test]
+    fn adapter_snapshot_epoch_is_normalized_as_diagnostic_metadata() {
+        let report = Fixture::positive()
+            .execute_with_snapshot(&OlderEpochSnapshotPort, method_proposal())
+            .unwrap();
+
+        assert_eq!(report.source.workspace_epoch, 7);
+        assert!(report
+            .evidence
+            .iter()
+            .all(|evidence| evidence.freshness.workspace_epoch == 7));
+    }
+
     impl SourceSnapshotPort for AliasedAnalysisSnapshotPort {
         fn capture(
             &self,
             analysis: &ResolvedSourceSet,
             _mutation_sources: &[ResolvedSourceSet],
             workspace_epoch: u64,
-        ) -> Result<SourceSnapshot, DiscoveryError> {
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
             let mut aliased_analysis = analysis.clone();
             aliased_analysis.mapping_digest =
                 "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                     .to_string();
             SourceSnapshot::new(
-                SourceSetSnapshot {
-                    source_set: aliased_analysis,
-                    source_fingerprint: FINGERPRINT.into(),
-                },
+                fake_source_snapshot(aliased_analysis),
                 Vec::new(),
-                COMPOSITE.into(),
                 workspace_epoch,
             )
-            .map_err(DiscoveryError::Operation)
+            .map_err(SnapshotCaptureError::classify)
         }
     }
 
     #[test]
     fn captured_analysis_snapshot_must_match_resolved_source_identity() {
         let result = Fixture::positive()
             .execute_with_snapshot(&AliasedAnalysisSnapshotPort, method_proposal());
 
-        assert!(matches!(result, Err(DiscoveryError::Operation(_))));
+        let Err(DiscoveryError::SnapshotCapture(error)) = result else {
+            panic!("analysis identity mismatch was not a typed snapshot failure");
+        };
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SnapshotInvariantViolation
+        );
+        assert!(!error.retryable());
     }
 
     #[derive(Clone, Copy)]
     enum MutationSnapshotMismatch {
         Omitted,
         Extra,
         Aliased,
     }
 
     impl SourceSnapshotPort for MutationSnapshotMismatch {
         fn capture(
             &self,
             analysis: &ResolvedSourceSet,
             mutation_sources: &[ResolvedSourceSet],
             workspace_epoch: u64,
-        ) -> Result<SourceSnapshot, DiscoveryError> {
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
             let mut mutations = mutation_sources
                 .iter()
                 .cloned()
-                .map(|source_set| SourceSetSnapshot {
-                    source_set,
-                    source_fingerprint: FINGERPRINT.into(),
-                })
+                .map(fake_source_snapshot)
                 .collect::<Vec<_>>();
             match self {
                 Self::Omitted => mutations.clear(),
-                Self::Extra => mutations.push(SourceSetSnapshot {
-                    source_set: ResolvedSourceSet {
-                        name: "extra".into(),
-                        kind: SourceSetKind::Extension,
-                        relative_root: "extra-src".into(),
-                        source_format: SourceFormat::PlatformXml,
-                        mapping_digest: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
+                Self::Extra => mutations.push(fake_source_snapshot(ResolvedSourceSet {
+                    name: "extra".into(),
+                    kind: SourceSetKind::Extension,
+                    relative_root: "extra-src".into(),
+                    source_format: SourceFormat::PlatformXml,
+                    mapping_digest:
+                        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                             .into(),
-                    },
-                    source_fingerprint: FINGERPRINT.into(),
-                }),
-                Self::Aliased => mutations[0].source_set.relative_root = "aliased-src".into(),
+                })),
+                Self::Aliased => {
+                    let mut source = mutations[0].source_set.clone();
+                    source.relative_root = "aliased-src".into();
+                    mutations[0] = fake_source_snapshot(source);
+                }
             }
             SourceSnapshot::new(
-                SourceSetSnapshot {
-                    source_set: analysis.clone(),
-                    source_fingerprint: FINGERPRINT.into(),
-                },
+                fake_source_snapshot(analysis.clone()),
                 mutations,
-                COMPOSITE.into(),
                 workspace_epoch,
             )
-            .map_err(DiscoveryError::Operation)
+            .map_err(SnapshotCaptureError::classify)
         }
     }
 
     #[test]
     fn captured_mutation_snapshots_must_match_resolved_sources_exactly() {
         for mismatch in [
             MutationSnapshotMismatch::Omitted,
             MutationSnapshotMismatch::Extra,
             MutationSnapshotMismatch::Aliased,
         ] {
             let result =
                 Fixture::positive().execute_with_snapshot(&mismatch, mutation_method_proposal());
 
-            assert!(matches!(result, Err(DiscoveryError::Operation(_))));
+            let Err(DiscoveryError::SnapshotCapture(error)) = result else {
+                panic!("mutation identity mismatch was not a typed snapshot failure");
+            };
+            assert_eq!(
+                error.reason,
+                SnapshotCaptureReason::SnapshotInvariantViolation
+            );
+            assert!(!error.retryable());
         }
     }
 
     #[test]
     fn no_actionable_result_is_insufficient_not_operation_error() {
         let ports = FakeEvidencePorts {
             metadata: complete(EvidencePort::MetadataCatalog, Vec::new()),
             code: complete(EvidencePort::CodeSearch, Vec::new()),
             definitions: complete(EvidencePort::Definition, Vec::new()),
             calls: complete(EvidencePort::CallGraph, Vec::new()),
@@ -1523,11 +1831,269 @@ mod tests {
                 method_proposal(),
             )
             .unwrap();
 
         assert!(!report.receipt_eligibility.eligible);
         assert_eq!(
             report.receipt_eligibility.blockers,
             ["receipt_store_not_implemented"]
         );
     }
+
+    #[test]
+    fn edt_analysis_is_independently_ineligible_for_receipt() {
+        let mut source = fake_resolved("main", false);
+        source.source_format = SourceFormat::Edt;
+        let snapshot = SourceSnapshot::new(fake_source_snapshot(source), Vec::new(), 7).unwrap();
+        let fixture = Fixture::positive();
+        let request = method_proposal();
+        let validation = ProposalValidation {
+            verdicts: Vec::new(),
+            material_ports: BTreeMap::new(),
+        };
+        let use_case = DiscoverExtensionPointsUseCase::new(
+            &FakeSourceResolver,
+            &FakeSnapshotPort,
+            &fixture.ports,
+            &fixture.ports,
+            &fixture.ports,
+            &fixture.ports,
+            &fixture.ports,
+            &fixture.ports,
+            &AllowReceiptIssuer,
+        );
+
+        let eligibility = use_case
+            .receipt_eligibility(&request, &snapshot, &validation, &[])
+            .unwrap();
+
+        assert!(!eligibility.eligible);
+        assert_eq!(eligibility.blockers, ["unsupported_source_format"]);
+    }
+
+    #[derive(Default)]
+    struct EdtSourceResolver {
+        resolved_mutation_count: AtomicUsize,
+    }
+
+    #[test]
+    fn application_revalidates_resolver_role_kind_and_format_contract() {
+        let valid_analysis = source_with_role_shape(
+            "analysis",
+            SourceSetKind::Configuration,
+            SourceFormat::PlatformXml,
+        );
+        let cases = [
+            (
+                ResolvedSourceSelection::new(
+                    source_with_role_shape(
+                        "external",
+                        SourceSetKind::ExternalProcessor,
+                        SourceFormat::PlatformXml,
+                    ),
+                    vec![],
+                )
+                .unwrap(),
+                "unsupported_source_kind",
+                SourceRole::Analysis,
+            ),
+            (
+                ResolvedSourceSelection::new(
+                    source_with_role_shape(
+                        "unknown",
+                        SourceSetKind::Configuration,
+                        SourceFormat::Unknown,
+                    ),
+                    vec![],
+                )
+                .unwrap(),
+                "unknown_source_format",
+                SourceRole::Analysis,
+            ),
+            (
+                ResolvedSourceSelection::new(
+                    source_with_role_shape(
+                        "edt-extension",
+                        SourceSetKind::Extension,
+                        SourceFormat::Edt,
+                    ),
+                    vec![],
+                )
+                .unwrap(),
+                "unsupported_source_format",
+                SourceRole::Analysis,
+            ),
+            (
+                ResolvedSourceSelection::new(
+                    valid_analysis.clone(),
+                    vec![source_with_role_shape(
+                        "destination-config",
+                        SourceSetKind::Configuration,
+                        SourceFormat::PlatformXml,
+                    )],
+                )
+                .unwrap(),
+                "unsupported_destination_kind",
+                SourceRole::Destination,
+            ),
+            (
+                ResolvedSourceSelection::new(
+                    valid_analysis,
+                    vec![source_with_role_shape(
+                        "destination-edt",
+                        SourceSetKind::Extension,
+                        SourceFormat::Edt,
+                    )],
+                )
+                .unwrap(),
+                "unsupported_destination_format",
+                SourceRole::Destination,
+            ),
+        ];
+        let providers = FakeEvidencePorts::positive();
+        for (selection, reason, role) in cases {
+            let resolver = FixedSourceResolver(selection);
+            let use_case = DiscoverExtensionPointsUseCase::new(
+                &resolver,
+                &PanicSnapshotPort,
+                &providers,
+                &providers,
+                &providers,
+                &providers,
+                &providers,
+                &providers,
+                &AllowReceiptIssuer,
+            );
+            let error = use_case
+                .execute(
+                    DiscoveryExecutionContext {
+                        workspace_root: "/workspace".into(),
+                        workspace_epoch: 7,
+                    },
+                    explore_request(),
+                )
+                .unwrap_err();
+            let DiscoveryError::SourceReadiness(error) = error else {
+                panic!("expected source-readiness error")
+            };
+            assert_eq!(error.reason_code(), reason);
+            assert_eq!(error.role, role);
+            assert!(!error.retryable());
+        }
+    }
+
+    impl ProjectSourceResolverPort for EdtSourceResolver {
+        fn resolve_all(
+            &self,
+            _context: &DiscoveryExecutionContext,
+            requested_analysis: Option<&str>,
+            requested_mutations: &[String],
+        ) -> Result<ResolvedSourceSelection, DiscoveryError> {
+            let mut source = fake_resolved(requested_analysis.unwrap_or("main"), false);
+            source.source_format = SourceFormat::Edt;
+            self.resolved_mutation_count
+                .store(requested_mutations.len(), Ordering::SeqCst);
+            ResolvedSourceSelection::new(
+                source,
+                requested_mutations
+                    .iter()
+                    .map(|name| fake_resolved(name, true))
+                    .collect(),
+            )
+            .map_err(DiscoveryError::Operation)
+        }
+    }
+
+    struct PanicEvidencePorts;
+
+    #[derive(Default)]
+    struct AnalysisOnlySnapshotPort {
+        mutation_count: AtomicUsize,
+    }
+
+    impl SourceSnapshotPort for AnalysisOnlySnapshotPort {
+        fn capture(
+            &self,
+            analysis: &ResolvedSourceSet,
+            mutation_sources: &[ResolvedSourceSet],
+            workspace_epoch: u64,
+        ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+            self.mutation_count
+                .store(mutation_sources.len(), Ordering::SeqCst);
+            FakeSnapshotPort.capture(analysis, mutation_sources, workspace_epoch)
+        }
+    }
+
+    macro_rules! panic_port {
+        ($trait_name:ident, $method:ident) => {
+            impl $trait_name for PanicEvidencePorts {
+                fn $method(
+                    &self,
+                    _plan: &DiscoveryQueryPlan,
+                    _context: &EvidenceExecutionContext<'_>,
+                ) -> ProviderOutcome<EvidenceRecord> {
+                    panic!("EDT readiness path must not invoke evidence providers")
+                }
+            }
+        };
+    }
+
+    panic_port!(MetadataCatalogPort, metadata);
+    panic_port!(CodeSearchPort, search);
+    panic_port!(DefinitionPort, definitions);
+    panic_port!(CallGraphPort, calls);
+    panic_port!(FormInspectionPort, forms);
+    panic_port!(SupportStatePort, support);
+
+    #[test]
+    fn edt_readiness_skips_providers_and_returns_typed_insufficient_report() {
+        let providers = PanicEvidencePorts;
+        let snapshots = AnalysisOnlySnapshotPort::default();
+        let resolver = EdtSourceResolver::default();
+        let use_case = DiscoverExtensionPointsUseCase::new(
+            &resolver,
+            &snapshots,
+            &providers,
+            &providers,
+            &providers,
+            &providers,
+            &providers,
+            &providers,
+            &AllowReceiptIssuer,
+        );
+
+        let report = use_case
+            .execute(
+                DiscoveryExecutionContext {
+                    workspace_root: "/workspace".into(),
+                    workspace_epoch: 7,
+                },
+                mutation_method_proposal(),
+            )
+            .unwrap();
+
+        assert_eq!(report.status, DiscoveryStatus::Insufficient);
+        assert_eq!(resolver.resolved_mutation_count.load(Ordering::SeqCst), 1);
+        assert_eq!(snapshots.mutation_count.load(Ordering::SeqCst), 0);
+        assert!(report.related_artifacts.is_empty());
+        assert!(report.flow_edges.is_empty());
+        assert!(report.extension_point_candidates.is_empty());
+        assert!(report.evidence.is_empty());
+        assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
+        assert_eq!(report.checks.len(), 1);
+        let check = &report.checks[0];
+        assert_eq!(check.code, "source_readiness");
+        assert_eq!(check.provider, "ProjectSourceResolverPort");
+        assert_eq!(check.state, CheckState::Skipped);
+        assert_eq!(check.outcome, CheckOutcome::Inconclusive);
+        assert_eq!(check.coverage, Coverage::Unknown);
+        assert_eq!(check.severity, CheckSeverity::Blocking);
+        assert_eq!(check.affects, ["proposal:method-hook"]);
+        assert_eq!(check.reason_code, "unsupported_source_format");
+        assert!(!check.retryable);
+        assert!(check.details.is_empty());
+        assert_eq!(
+            report.receipt_eligibility.blockers,
+            ["unsupported_source_format"]
+        );
+    }
 }
diff --git a/crates/unica-coder/src/application/mod.rs b/crates/unica-coder/src/application/mod.rs
index 4260f73..3cea3cd 100644
--- a/crates/unica-coder/src/application/mod.rs
+++ b/crates/unica-coder/src/application/mod.rs
@@ -1,20 +1,20 @@
 use crate::domain::cache::{CacheAccess, CacheReport};
 use crate::domain::events::{runtime_event_kind, DomainEvent, DomainEventKind};
-use crate::domain::project_sources::discover_project_source_map;
 use crate::domain::workspace::WorkspaceContext;
 use crate::infrastructure::internal_adapters::RuntimeJobAction;
 use crate::infrastructure::native_operations::common::{
     absolutize, path_arg, required_string, support_guard_violation, SupportGuardRequirement,
     SupportGuardViolation,
 };
 use crate::infrastructure::native_operations::{meta, template};
+use crate::infrastructure::project_sources::discover_project_source_map;
 use crate::infrastructure::AdapterOutcome;
 use operation_descriptors::SupportGuardPolicy;
 use ports::{ApplicationPorts, DefaultApplicationPorts};
 use serde::Serialize;
 use serde_json::{json, Map, Value};
 use std::env;
 use std::path::{Path, PathBuf};
 
 // The public MCP registration is intentionally deferred until the receipt and
 // guard slices are complete; intermediate discovery types are consumed by the
@@ -5737,20 +5737,23 @@ mod tests {
     }
 
     #[test]
     fn external_init_preview_is_path_guarded_and_source_set_typed() {
         let root = std::env::temp_dir().join(format!(
             "unica-external-init-contract-{}",
             std::process::id()
         ));
         let workspace = root.join("workspace");
         std::fs::create_dir_all(&workspace).unwrap();
+        for source_root in ["epf", "erf", "епф"] {
+            std::fs::create_dir_all(workspace.join(source_root)).unwrap();
+        }
         std::fs::write(
             workspace.join("v8project.yaml"),
             concat!(
                 "format: DESIGNER\n",
                 "source-set:\n",
                 "  - name: processors\n",
                 "    type: EXTERNAL_DATA_PROCESSORS\n",
                 "    path: epf\n",
                 "  - name: reports\n",
                 "    type: EXTERNAL_REPORTS\n",
@@ -5769,53 +5772,53 @@ mod tests {
         );
         args.insert("dryRun".to_string(), Value::Bool(true));
         args.insert("Name".to_string(), Value::String("Preview".to_string()));
         args.insert("OutputDir".to_string(), Value::String("epf".to_string()));
 
         let preview = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap();
         assert!(preview.ok, "{:?}", preview.errors);
         assert_eq!(preview.artifacts.len(), 2);
-        assert!(!workspace.join("epf").exists());
+        assert!(!workspace.join("epf/Preview.xml").exists());
 
         args.insert("OutputDir".to_string(), Value::String("EPF".to_string()));
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("exact source-set root"), "{error}");
-        assert!(!workspace.join("EPF").exists());
+        assert!(!workspace.join("epf/Preview.xml").exists());
 
         args.insert("OutputDir".to_string(), Value::String("ЕПФ".to_string()));
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("exact source-set root"), "{error}");
-        assert!(!workspace.join("ЕПФ").exists());
+        assert!(!workspace.join("епф/Preview.xml").exists());
 
         args.insert(
             "OutputDir".to_string(),
             Value::String("epf/nested".to_string()),
         );
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("source-set root"), "{error}");
-        assert!(!workspace.join("epf").exists());
+        assert!(!workspace.join("epf/nested").exists());
 
         args.insert("OutputDir".to_string(), Value::String("erf".to_string()));
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("source-set `reports`"), "{error}");
         assert!(error.contains("ExternalReport"), "{error}");
-        assert!(!workspace.join("erf").exists());
+        assert!(!workspace.join("erf/Preview.xml").exists());
 
         args.insert(
             "OutputDir".to_string(),
             Value::String("../outside".to_string()),
         );
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("outside workspace root"), "{error}");
         assert!(!root.join("outside").exists());
@@ -5866,26 +5869,27 @@ mod tests {
             workspace.join("v8project.yaml"),
             concat!(
                 "format: DESIGNER\n",
                 "source-set:\n",
                 "  - name: configuration\n",
                 "    type: CONFIGURATION\n",
                 "    path: src\n",
             ),
         )
         .unwrap();
+        std::fs::create_dir_all(workspace.join("src")).unwrap();
         args.insert("OutputDir".to_string(), Value::String("SRC".to_string()));
         let error = UnicaApplication::new()
             .call_tool("unica.epf.init", &args)
             .unwrap_err();
         assert!(error.contains("exact source-set root"), "{error}");
-        assert!(!workspace.join("SRC").exists());
+        assert!(!workspace.join("src/Preview.xml").exists());
 
         std::fs::write(
             workspace.join("v8project.yaml"),
             concat!(
                 "format: EDT\n",
                 "source-set:\n",
                 "  - name: processors\n",
                 "    type: EXTERNAL_DATA_PROCESSORS\n",
                 "    path: epf\n",
             ),
diff --git a/crates/unica-coder/src/application/tool_contracts.rs b/crates/unica-coder/src/application/tool_contracts.rs
index c10f8da..af5d473 100644
--- a/crates/unica-coder/src/application/tool_contracts.rs
+++ b/crates/unica-coder/src/application/tool_contracts.rs
@@ -1,15 +1,16 @@
 use super::operation_descriptors::native_operation_descriptor;
 use super::{ToolHandler, ToolSpec};
-use crate::domain::project_sources::{discover_project_source_map, SourceFormat, SourceSetKind};
+use crate::domain::project_sources::{SourceFormat, SourceSetKind};
 use crate::domain::workspace::WorkspaceContext;
 use crate::infrastructure::path_policy::WorkspacePathPolicy;
+use crate::infrastructure::project_sources::discover_project_source_map;
 use serde_json::{json, Map, Value};
 use std::collections::BTreeSet;
 use std::path::{Component, Path, PathBuf};
 use uuid::Uuid;
 
 const COMMON_ARGS: &[&str] = &["cwd", "dryRun", "confirm"];
 const RUNTIME_JOB_STATUS_ARGS: &[&str] = &["jobId"];
 const RUNTIME_JOB_WAIT_ARGS: &[&str] = &["jobId", "timeoutSeconds"];
 const RUNTIME_JOB_LOGS_ARGS: &[&str] = &["jobId", "tailChars"];
 
diff --git a/crates/unica-coder/src/domain/discovery_registry.rs b/crates/unica-coder/src/domain/discovery_registry.rs
index 121e385..4ecb613 100644
--- a/crates/unica-coder/src/domain/discovery_registry.rs
+++ b/crates/unica-coder/src/domain/discovery_registry.rs
@@ -90,20 +90,42 @@ metadata_kind_registry! {
 
 pub(crate) const MODULE_KIND_TAGS: &[&str] = &[
     "Module",
     "ObjectModule",
     "ManagerModule",
     "RecordSetModule",
     "ValueManagerModule",
     "CommandModule",
 ];
 
+/// Exact v1 filesystem artifacts. Snapshot selection and future providers use
+/// these registries instead of independently globbing Ext directories.
+pub(crate) const SOURCE_ROOT_EXT_ARTIFACTS_V1: &[&str] = &[
+    "ManagedApplicationModule.bsl",
+    "OrdinaryApplicationModule.bsl",
+    "SessionModule.bsl",
+    "ExternalConnectionModule.bsl",
+    "CommandInterface.xml",
+    "ManagedApplicationCommandInterface.xml",
+    "OrdinaryApplicationCommandInterface.xml",
+    "ClientApplicationInterface.xml",
+    "HomePageWorkArea.xml",
+    "Help.xml",
+];
+
+pub(crate) const EDT_DIAGNOSTIC_MARKERS_V1: &[&str] = &[
+    ".project",
+    "DT-INF/PROJECT.PMF",
+    "Configuration/Configuration.mdo",
+    "src/Configuration/Configuration.mdo",
+];
+
 pub(crate) fn metadata_kind(tag: &str) -> Option<&'static MetadataKind> {
     METADATA_KINDS.iter().find(|kind| kind.tag == tag)
 }
 
 pub(crate) fn metadata_kind_by_directory(directory: &str) -> Option<&'static MetadataKind> {
     METADATA_KINDS
         .iter()
         .find(|kind| kind.directory.eq_ignore_ascii_case(directory))
 }
 
diff --git a/crates/unica-coder/src/domain/project_sources.rs b/crates/unica-coder/src/domain/project_sources.rs
index 1d49ca7..0c1a003 100644
--- a/crates/unica-coder/src/domain/project_sources.rs
+++ b/crates/unica-coder/src/domain/project_sources.rs
@@ -1,588 +1,41 @@
 use serde::{Deserialize, Serialize};
-use serde_yaml::Value as YamlValue;
-use std::path::{Path, PathBuf};
 
+/// Transport-facing view of the project source topology. Filesystem and YAML
+/// discovery deliberately live in infrastructure.
 #[derive(Debug, Clone, Serialize)]
 #[serde(rename_all = "camelCase")]
 pub struct ProjectSourceMap {
     pub workspace_root: String,
     pub config_path: Option<String>,
     pub source_sets: Vec<ProjectSourceSet>,
     #[serde(skip_serializing)]
     pub(crate) configured_format_raw: Option<String>,
 }
 
 #[derive(Debug, Clone, Serialize)]
 #[serde(rename_all = "camelCase")]
 pub struct ProjectSourceSet {
     pub name: String,
     pub kind: SourceSetKind,
     pub path: String,
     pub source_format: SourceFormat,
     pub format_evidence: Vec<String>,
 }
 
-#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
+#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
 #[serde(rename_all = "snake_case")]
 pub enum SourceSetKind {
     Configuration,
     Extension,
     ExternalProcessor,
     ExternalReport,
 }
 
-#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
+#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
 #[serde(rename_all = "snake_case")]
 pub enum SourceFormat {
     PlatformXml,
     Edt,
     Unknown,
     Invalid,
 }
-
-#[derive(Debug, Clone)]
-struct ConfigSourceSet {
-    name: String,
-    kind: SourceSetKind,
-    path: String,
-    default_format: Option<SourceFormat>,
-}
-
-pub fn discover_project_source_map(workspace_root: &Path) -> Result<ProjectSourceMap, String> {
-    let config_path = find_project_config(workspace_root);
-    let (mut source_sets, configured_format_raw) = if let Some(path) = &config_path {
-        read_config_source_sets(workspace_root, path)?
-    } else {
-        (autodetect_source_sets(workspace_root), None)
-    };
-
-    if source_sets.is_empty() {
-        source_sets = autodetect_source_sets(workspace_root);
-    }
-
-    let project_source_sets = source_sets
-        .into_iter()
-        .map(|source_set| detect_source_set_format(workspace_root, source_set))
-        .collect::<Vec<_>>();
-
-    Ok(ProjectSourceMap {
-        workspace_root: workspace_root.display().to_string(),
-        config_path: config_path.map(|path| path.display().to_string()),
-        source_sets: project_source_sets,
-        configured_format_raw,
-    })
-}
-
-fn find_project_config(workspace_root: &Path) -> Option<PathBuf> {
-    let default = workspace_root.join("v8project.yaml");
-    default.is_file().then_some(default)
-}
-
-fn read_config_source_sets(
-    workspace_root: &Path,
-    config_path: &Path,
-) -> Result<(Vec<ConfigSourceSet>, Option<String>), String> {
-    let text = std::fs::read_to_string(config_path)
-        .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?;
-    let yaml = serde_yaml::from_str::<YamlValue>(&text)
-        .map_err(|err| format!("failed to parse {}: {err}", config_path.display()))?;
-    let configured_format_raw = match yaml_mapping_get(&yaml, "format") {
-        None => None,
-        Some(YamlValue::String(value)) => Some(value.clone()),
-        Some(_) => {
-            return Err(format!(
-                "{} field `format` must be a string",
-                config_path.display()
-            ));
-        }
-    };
-    let default_format = configured_format_raw
-        .clone()
-        .and_then(source_format_from_config);
-    let base_path = yaml_string(&yaml, "basePath").unwrap_or_else(|| ".".to_string());
-    let source_set_value = yaml_mapping_get(&yaml, "source-set");
-    let mut source_sets = Vec::new();
-
-    match source_set_value {
-        Some(YamlValue::Sequence(entries)) => {
-            for entry in entries {
-                source_sets.push(config_source_set_from_yaml(entry, default_format)?);
-            }
-        }
-        Some(YamlValue::Mapping(entries)) => {
-            for (key, entry) in entries {
-                let name = key.as_str().unwrap_or("main");
-                source_sets.push(config_source_set_from_named_yaml(
-                    name,
-                    entry,
-                    default_format,
-                )?);
-            }
-        }
-        Some(YamlValue::Null) | None => {}
-        Some(_) => {
-            return Err(format!(
-                "{} field `source-set` must be a list or mapping",
-                config_path.display()
-            ));
-        }
-    }
-
-    for source_set in &mut source_sets {
-        source_set.path = normalize_configured_path(workspace_root, &base_path, &source_set.path);
-    }
-
-    Ok((source_sets, configured_format_raw))
-}
-
-fn config_source_set_from_yaml(
-    entry: &YamlValue,
-    default_format: Option<SourceFormat>,
-) -> Result<ConfigSourceSet, String> {
-    let name = yaml_string(entry, "name").unwrap_or_else(|| "main".to_string());
-    config_source_set_from_named_yaml(&name, entry, default_format)
-}
-
-fn config_source_set_from_named_yaml(
-    name: &str,
-    entry: &YamlValue,
-    default_format: Option<SourceFormat>,
-) -> Result<ConfigSourceSet, String> {
-    let source_type = yaml_string(entry, "type")
-        .or_else(|| yaml_string(entry, "purpose"))
-        .unwrap_or_else(|| "CONFIGURATION".to_string());
-    let kind = source_set_kind_from_config(&source_type)?;
-    let path = yaml_string(entry, "path").unwrap_or_else(|| ".".to_string());
-    Ok(ConfigSourceSet {
-        name: name.to_string(),
-        kind,
-        path,
-        default_format,
-    })
-}
-
-fn normalize_configured_path(workspace_root: &Path, base_path: &str, raw_path: &str) -> String {
-    let base = PathBuf::from(base_path);
-    let path = PathBuf::from(raw_path);
-    let resolved = if path.is_absolute() {
-        path
-    } else if base.is_absolute() {
-        base.join(path)
-    } else {
-        workspace_root.join(base).join(path)
-    };
-    path_relative_to(workspace_root, &resolved)
-}
-
-fn autodetect_source_sets(workspace_root: &Path) -> Vec<ConfigSourceSet> {
-    for path in [".", "src", "src/cf"] {
-        let root = workspace_root.join(path);
-        if root.join("Configuration.xml").is_file()
-            || root.join("Configuration/Configuration.mdo").is_file()
-            || root.join("src/Configuration/Configuration.mdo").is_file()
-        {
-            return vec![ConfigSourceSet {
-                name: "main".to_string(),
-                kind: SourceSetKind::Configuration,
-                path: path.to_string(),
-                default_format: None,
-            }];
-        }
-    }
-    Vec::new()
-}
-
-fn detect_source_set_format(
-    workspace_root: &Path,
-    source_set: ConfigSourceSet,
-) -> ProjectSourceSet {
-    let source_root = workspace_root.join(&source_set.path);
-    let platform_evidence = platform_xml_evidence(workspace_root, &source_root, source_set.kind);
-    let edt_evidence = edt_evidence(workspace_root, &source_root);
-    let source_format = match (platform_evidence.is_empty(), edt_evidence.is_empty()) {
-        (false, false) => SourceFormat::Invalid,
-        (false, true) => SourceFormat::PlatformXml,
-        (true, false) => SourceFormat::Edt,
-        (true, true) => source_set.default_format.unwrap_or(SourceFormat::Unknown),
-    };
-    let mut format_evidence = Vec::new();
-    format_evidence.extend(platform_evidence);
-    format_evidence.extend(edt_evidence);
-    if format_evidence.is_empty() {
-        if let Some(default_format) = source_set.default_format {
-            format_evidence.push(match default_format {
-                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".to_string(),
-                SourceFormat::Edt => "v8project.yaml:format=EDT".to_string(),
-                SourceFormat::Unknown | SourceFormat::Invalid => {
-                    "v8project.yaml:format".to_string()
-                }
-            });
-        }
-    }
-
-    ProjectSourceSet {
-        name: source_set.name,
-        kind: source_set.kind,
-        path: source_set.path,
-        source_format,
-        format_evidence,
-    }
-}
-
-fn platform_xml_evidence(
-    workspace_root: &Path,
-    source_root: &Path,
-    kind: SourceSetKind,
-) -> Vec<String> {
-    let mut evidence = Vec::new();
-    for rel in ["Configuration.xml", "ConfigDumpInfo.xml"] {
-        push_existing(&mut evidence, workspace_root, &source_root.join(rel));
-    }
-
-    if matches!(
-        kind,
-        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
-    ) {
-        if let Ok(entries) = std::fs::read_dir(source_root) {
-            for entry in entries.flatten() {
-                let path = entry.path();
-                if path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
-                    push_existing(&mut evidence, workspace_root, &path);
-                }
-            }
-        }
-    }
-    evidence.sort();
-    evidence.dedup();
-    evidence
-}
-
-fn edt_evidence(workspace_root: &Path, source_root: &Path) -> Vec<String> {
-    let mut evidence = Vec::new();
-    for rel in [
-        ".project",
-        "DT-INF/PROJECT.PMF",
-        "Configuration/Configuration.mdo",
-        "src/Configuration/Configuration.mdo",
-    ] {
-        push_existing(&mut evidence, workspace_root, &source_root.join(rel));
-    }
-    evidence.sort();
-    evidence.dedup();
-    evidence
-}
-
-fn push_existing(evidence: &mut Vec<String>, workspace_root: &Path, path: &Path) {
-    if path.is_file() {
-        evidence.push(path_relative_to(workspace_root, path));
-    }
-}
-
-fn path_relative_to(root: &Path, path: &Path) -> String {
-    let path = path
-        .strip_prefix(root)
-        .unwrap_or(path)
-        .display()
-        .to_string();
-    #[cfg(windows)]
-    {
-        path.replace('\\', "/")
-    }
-    #[cfg(not(windows))]
-    {
-        path
-    }
-}
-
-fn source_set_kind_from_config(raw: &str) -> Result<SourceSetKind, String> {
-    match raw.to_ascii_uppercase().as_str() {
-        "CONFIGURATION" => Ok(SourceSetKind::Configuration),
-        "EXTENSION" => Ok(SourceSetKind::Extension),
-        "EXTERNAL_DATA_PROCESSORS" => Ok(SourceSetKind::ExternalProcessor),
-        "EXTERNAL_REPORTS" => Ok(SourceSetKind::ExternalReport),
-        other => Err(format!("unsupported source-set type `{other}`")),
-    }
-}
-
-fn source_format_from_config(raw: String) -> Option<SourceFormat> {
-    match raw.to_ascii_uppercase().as_str() {
-        "DESIGNER" | "PLATFORM_XML" | "XML" => Some(SourceFormat::PlatformXml),
-        "EDT" => Some(SourceFormat::Edt),
-        _ => None,
-    }
-}
-
-fn yaml_string(value: &YamlValue, key: &str) -> Option<String> {
-    yaml_mapping_get(value, key).and_then(|value| match value {
-        YamlValue::String(text) => Some(text.clone()),
-        YamlValue::Number(number) => Some(number.to_string()),
-        _ => None,
-    })
-}
-
-fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
-    let mapping = value.as_mapping()?;
-    mapping.get(YamlValue::String(key.to_string()))
-}
-
-#[cfg(test)]
-mod tests {
-    use super::*;
-    use std::ffi::{OsStr, OsString};
-    use std::fs;
-    use std::path::{Path, PathBuf};
-    use std::time::{SystemTime, UNIX_EPOCH};
-
-    #[test]
-    fn detects_edt_configuration_and_platform_external_processor_source_sets() {
-        let root = temp_workspace("unica-source-map-multi");
-        write(
-            &root.join("v8project.yaml"),
-            r#"
-format: EDT
-source-set:
-  - name: main
-    type: CONFIGURATION
-    path: src
-  - name: external-processors
-    type: EXTERNAL_DATA_PROCESSORS
-    path: epf
-"#,
-        );
-        write(&root.join("src/.project"), "<projectDescription/>");
-        write(
-            &root.join("src/Configuration/Configuration.mdo"),
-            "<mdclass:Configuration/>",
-        );
-        write(
-            &root.join("epf/PriceLoader.xml"),
-            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
-        );
-
-        let map = discover_project_source_map(&root).unwrap();
-
-        assert_eq!(map.source_sets.len(), 2);
-        assert_source_set(
-            &map,
-            "main",
-            SourceSetKind::Configuration,
-            SourceFormat::Edt,
-            &["src/.project", "src/Configuration/Configuration.mdo"],
-        );
-        assert_source_set(
-            &map,
-            "external-processors",
-            SourceSetKind::ExternalProcessor,
-            SourceFormat::PlatformXml,
-            &["epf/PriceLoader.xml"],
-        );
-
-        fs::remove_dir_all(root).unwrap();
-    }
-
-    #[test]
-    fn detects_single_platform_configuration_source_set() {
-        let root = temp_workspace("unica-source-map-platform");
-        write(
-            &root.join("v8project.yaml"),
-            r#"
-format: DESIGNER
-source-set:
-  - name: main
-    type: CONFIGURATION
-    path: src
-"#,
-        );
-        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
-        write(&root.join("src/ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");
-
-        let map = discover_project_source_map(&root).unwrap();
-
-        assert_source_set(
-            &map,
-            "main",
-            SourceSetKind::Configuration,
-            SourceFormat::PlatformXml,
-            &["src/Configuration.xml", "src/ConfigDumpInfo.xml"],
-        );
-
-        fs::remove_dir_all(root).unwrap();
-    }
-
-    #[test]
-    fn ignores_legacy_v8tr_config_environment_override() {
-        let root = temp_workspace("unica-source-map-ignore-v8tr-config");
-        write(
-            &root.join("v8project.yaml"),
-            r#"
-format: DESIGNER
-source-set:
-  - name: main
-    type: CONFIGURATION
-    path: src
-"#,
-        );
-        write(
-            &root.join("custom.yaml"),
-            r#"
-format: DESIGNER
-source-set:
-  - name: env
-    type: CONFIGURATION
-    path: env-src
-"#,
-        );
-        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
-        write(&root.join("env-src/Configuration.xml"), "<MetaDataObject/>");
-        let _guard = EnvVarGuard::set("V8TR_CONFIG", root.join("custom.yaml"));
-
-        let map = discover_project_source_map(&root).unwrap();
-
-        assert_source_set(
-            &map,
-            "main",
-            SourceSetKind::Configuration,
-            SourceFormat::PlatformXml,
-            &["src/Configuration.xml"],
-        );
-        assert!(
-            map.source_sets
-                .iter()
-                .all(|source_set| source_set.name != "env"),
-            "legacy V8TR_CONFIG source set must be ignored: {map:?}"
-        );
-
-        fs::remove_dir_all(root).unwrap();
-    }
-
-    #[test]
-    fn detects_single_edt_configuration_source_set() {
-        let root = temp_workspace("unica-source-map-edt");
-        write(
-            &root.join("v8project.yaml"),
-            r#"
-format: EDT
-source-set:
-  - name: main
-    type: CONFIGURATION
-    path: src
-"#,
-        );
-        write(&root.join("src/.project"), "<projectDescription/>");
-        write(
-            &root.join("src/Configuration/Configuration.mdo"),
-            "<mdclass:Configuration/>",
-        );
-
-        let map = discover_project_source_map(&root).unwrap();
-
-        assert_source_set(
-            &map,
-            "main",
-            SourceSetKind::Configuration,
-            SourceFormat::Edt,
-            &["src/.project", "src/Configuration/Configuration.mdo"],
-        );
-
-        fs::remove_dir_all(root).unwrap();
-    }
-
-    #[test]
-    fn conflicting_markers_inside_one_source_set_are_invalid_not_mixed() {
-        let root = temp_workspace("unica-source-map-invalid");
-        write(
-            &root.join("v8project.yaml"),
-            r#"
-source-set:
-  - name: main
-    type: CONFIGURATION
-    path: src
-"#,
-        );
-        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
-        write(
-            &root.join("src/Configuration/Configuration.mdo"),
-            "<mdclass:Configuration/>",
-        );
-
-        let map = discover_project_source_map(&root).unwrap();
-
-        assert_source_set(
-            &map,
-            "main",
-            SourceSetKind::Configuration,
-            SourceFormat::Invalid,
-            &[
-                "src/Configuration.xml",
-                "src/Configuration/Configuration.mdo",
-            ],
-        );
-
-        fs::remove_dir_all(root).unwrap();
-    }
-
-    fn assert_source_set(
-        map: &ProjectSourceMap,
-        name: &str,
-        kind: SourceSetKind,
-        source_format: SourceFormat,
-        expected_evidence: &[&str],
-    ) {
-        let source_set = map
-            .source_sets
-            .iter()
-            .find(|source_set| source_set.name == name)
-            .unwrap_or_else(|| panic!("source set {name} not found in {map:?}"));
-        assert_eq!(source_set.kind, kind);
-        assert_eq!(source_set.source_format, source_format);
-        for evidence in expected_evidence {
-            assert!(
-                source_set
-                    .format_evidence
-                    .iter()
-                    .any(|actual| actual == evidence),
-                "missing evidence {evidence} in {source_set:?}"
-            );
-        }
-    }
-
-    fn temp_workspace(prefix: &str) -> PathBuf {
-        let nanos = SystemTime::now()
-            .duration_since(UNIX_EPOCH)
-            .unwrap()
-            .as_nanos();
-        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
-        fs::create_dir_all(&root).unwrap();
-        root
-    }
-
-    fn write(path: &Path, text: &str) {
-        if let Some(parent) = path.parent() {
-            fs::create_dir_all(parent).unwrap();
-        }
-        fs::write(path, text).unwrap();
-    }
-
-    struct EnvVarGuard {
-        key: &'static str,
-        previous: Option<OsString>,
-    }
-
-    impl EnvVarGuard {
-        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
-            let previous = std::env::var_os(key);
-            std::env::set_var(key, value);
-            Self { key, previous }
-        }
-    }
-
-    impl Drop for EnvVarGuard {
-        fn drop(&mut self) {
-            if let Some(previous) = &self.previous {
-                std::env::set_var(self.key, previous);
-            } else {
-                std::env::remove_var(self.key);
-            }
-        }
-    }
-}
diff --git a/crates/unica-coder/src/domain/source_snapshot.rs b/crates/unica-coder/src/domain/source_snapshot.rs
index cecdcbe..107eeff 100644
--- a/crates/unica-coder/src/domain/source_snapshot.rs
+++ b/crates/unica-coder/src/domain/source_snapshot.rs
@@ -1,140 +1,428 @@
 use super::project_sources::{SourceFormat, SourceSetKind};
+use sha2::{Digest, Sha256};
 use std::collections::BTreeMap;
+use std::fmt;
+
+const SOURCE_FINGERPRINT_DOMAIN: &[u8] = b"unica.source-set-snapshot.v1";
+const COMPOSITE_FINGERPRINT_DOMAIN: &[u8] = b"unica.source-composite.v1";
 
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub(crate) struct ResolvedSourceSet {
     pub(crate) name: String,
     pub(crate) kind: SourceSetKind,
     pub(crate) relative_root: String,
     pub(crate) source_format: SourceFormat,
     pub(crate) mapping_digest: String,
 }
 
 impl ResolvedSourceSet {
+    pub(crate) fn new(
+        name: String,
+        kind: SourceSetKind,
+        relative_root: String,
+        source_format: SourceFormat,
+        mapping_digest: String,
+    ) -> Result<Self, String> {
+        let source = Self {
+            name,
+            kind,
+            relative_root,
+            source_format,
+            mapping_digest,
+        };
+        source.validate()?;
+        Ok(source)
+    }
+
     pub(crate) fn validate(&self) -> Result<(), String> {
         stable_component(&self.name, "source-set name", 1024)?;
-        contained_relative_path(&self.relative_root)?;
+        contained_relative_root(&self.relative_root)?;
         validate_fingerprint(&self.mapping_digest)?;
         Ok(())
     }
 }
 
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct ResolvedSourceSelection {
+    pub(crate) mapping_digest: String,
+    pub(crate) analysis: ResolvedSourceSet,
+    pub(crate) mutations: Vec<ResolvedSourceSet>,
+}
+
+impl ResolvedSourceSelection {
+    pub(crate) fn new(
+        analysis: ResolvedSourceSet,
+        mut mutations: Vec<ResolvedSourceSet>,
+    ) -> Result<Self, String> {
+        let mapping_digest = analysis.mapping_digest.clone();
+        mutations.sort_by(|left, right| resolved_source_key(left).cmp(&resolved_source_key(right)));
+        for pair in mutations.windows(2) {
+            if pair[0].name.to_lowercase() == pair[1].name.to_lowercase() && pair[0] != pair[1] {
+                return Err(
+                    "one mutation source-set name cannot resolve to conflicting identities".into(),
+                );
+            }
+        }
+        mutations.dedup();
+        let selection = Self {
+            mapping_digest,
+            analysis,
+            mutations,
+        };
+        selection.validate()?;
+        Ok(selection)
+    }
+
+    pub(crate) fn validate(&self) -> Result<(), String> {
+        self.analysis.validate()?;
+        for mutation in &self.mutations {
+            mutation.validate()?;
+        }
+        if self.mapping_digest != self.analysis.mapping_digest
+            || self
+                .mutations
+                .iter()
+                .any(|mutation| mutation.mapping_digest != self.mapping_digest)
+        {
+            return Err("resolved sources must come from one mapping digest".into());
+        }
+        let mut normalized = self.mutations.clone();
+        normalized
+            .sort_by(|left, right| resolved_source_key(left).cmp(&resolved_source_key(right)));
+        for pair in normalized.windows(2) {
+            if pair[0].name.to_lowercase() == pair[1].name.to_lowercase() && pair[0] != pair[1] {
+                return Err(
+                    "one mutation source-set name cannot resolve to conflicting identities".into(),
+                );
+            }
+        }
+        normalized.dedup();
+        if self.mutations != normalized {
+            return Err("mutation source sets must be canonically sorted and unique".into());
+        }
+        Ok(())
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct MaterialFile {
+    pub(crate) byte_length: u64,
+    pub(crate) content_digest: String,
+}
+
+impl MaterialFile {
+    pub(crate) fn new(byte_length: u64, content_digest: String) -> Result<Self, String> {
+        validate_fingerprint(&content_digest)?;
+        Ok(Self {
+            byte_length,
+            content_digest,
+        })
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) enum ManifestEntry {
+    Present(MaterialFile),
+    AbsentOptional(OptionalMaterialTag),
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub(crate) enum OptionalMaterialTag {
+    ParentConfigurations,
+    EdtProject,
+    EdtProjectPmf,
+    EdtConfigurationMdo,
+    EdtSourceConfigurationMdo,
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct SourceManifest {
+    entries: BTreeMap<String, ManifestEntry>,
+}
+
+impl SourceManifest {
+    pub(crate) fn new(entries: BTreeMap<String, ManifestEntry>) -> Result<Self, String> {
+        if entries.is_empty() {
+            return Err("source manifest must not be empty".into());
+        }
+        for (path, entry) in &entries {
+            contained_relative_file(path)?;
+            match entry {
+                ManifestEntry::Present(file) => validate_fingerprint(&file.content_digest)?,
+                ManifestEntry::AbsentOptional(tag) if !optional_tag_matches_path(*tag, path) => {
+                    return Err("optional-material tombstone must name its declared path".into());
+                }
+                ManifestEntry::AbsentOptional(_) => {}
+            }
+        }
+        Ok(Self { entries })
+    }
+
+    // Task 5 evidence providers consume the immutable manifest directly.
+    #[allow(dead_code)]
+    pub(crate) fn entries(&self) -> &BTreeMap<String, ManifestEntry> {
+        &self.entries
+    }
+
+    pub(crate) fn get(&self, path: &str) -> Option<&ManifestEntry> {
+        self.entries.get(path)
+    }
+}
+
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub(crate) struct SourceSetSnapshot {
     pub(crate) source_set: ResolvedSourceSet,
     pub(crate) source_fingerprint: String,
+    pub(crate) manifest: SourceManifest,
 }
 
 impl SourceSetSnapshot {
+    pub(crate) fn from_manifest(
+        source_set: ResolvedSourceSet,
+        manifest: SourceManifest,
+    ) -> Result<Self, String> {
+        source_set.validate()?;
+        let source_fingerprint = source_fingerprint(&source_set, &manifest)?;
+        Ok(Self {
+            source_set,
+            source_fingerprint,
+            manifest,
+        })
+    }
+
     pub(crate) fn validate(&self) -> Result<(), String> {
         self.source_set.validate()?;
-        validate_fingerprint(&self.source_fingerprint)
+        validate_fingerprint(&self.source_fingerprint)?;
+        if self.source_fingerprint != source_fingerprint(&self.source_set, &self.manifest)? {
+            return Err("source fingerprint does not match source identity and manifest".into());
+        }
+        Ok(())
     }
 }
 
 #[derive(Debug, Clone, PartialEq, Eq)]
 pub(crate) struct SourceSnapshot {
     pub(crate) analysis: SourceSetSnapshot,
     pub(crate) mutations: Vec<SourceSetSnapshot>,
     pub(crate) composite_fingerprint: String,
     pub(crate) workspace_epoch: u64,
 }
 
 impl SourceSnapshot {
-    // Concrete filesystem capture is delivered in Task 4; fakes and that
-    // adapter construct snapshots through this invariant-preserving boundary.
-    #[allow(dead_code)]
     pub(crate) fn new(
         analysis: SourceSetSnapshot,
         mut mutations: Vec<SourceSetSnapshot>,
-        composite_fingerprint: String,
         workspace_epoch: u64,
     ) -> Result<Self, String> {
         analysis.validate()?;
         for mutation in &mutations {
             mutation.validate()?;
         }
-        mutations.sort_by(|left, right| {
-            snapshot_key(SnapshotRoleKey::Mutation, left)
-                .cmp(&snapshot_key(SnapshotRoleKey::Mutation, right))
-        });
+        mutations.sort_by(|left, right| snapshot_key(left).cmp(&snapshot_key(right)));
+        for pair in mutations.windows(2) {
+            if pair[0].source_set.name.to_lowercase() == pair[1].source_set.name.to_lowercase()
+                && pair[0] != pair[1]
+            {
+                return Err(
+                    "one mutation source-set name cannot have conflicting snapshots".into(),
+                );
+            }
+        }
         mutations.dedup();
-        validate_fingerprint(&composite_fingerprint)?;
+        let composite_fingerprint = composite_fingerprint(&analysis, &mutations)?;
         let snapshot = Self {
             analysis,
             mutations,
             composite_fingerprint,
             workspace_epoch,
         };
         snapshot.validate()?;
         Ok(snapshot)
     }
 
     pub(crate) fn validate(&self) -> Result<(), String> {
         self.analysis.validate()?;
-        validate_fingerprint(&self.composite_fingerprint)?;
-        let mut role_identities = BTreeMap::new();
-        role_identities.insert(
-            (
-                SnapshotRoleKey::Analysis,
-                self.analysis.source_set.name.as_str(),
-            ),
-            &self.analysis,
-        );
-        let mut previous = None;
         for mutation in &self.mutations {
             mutation.validate()?;
-            if let Some(previous_snapshot) = previous {
-                if snapshot_key(SnapshotRoleKey::Mutation, previous_snapshot)
-                    > snapshot_key(SnapshotRoleKey::Mutation, mutation)
-                {
-                    return Err("mutation source snapshots must be canonically sorted".to_string());
-                }
+        }
+        if self
+            .mutations
+            .windows(2)
+            .any(|pair| snapshot_key(&pair[0]) >= snapshot_key(&pair[1]))
+        {
+            return Err("mutation source snapshots must be canonically sorted and unique".into());
+        }
+        let expected = composite_fingerprint(&self.analysis, &self.mutations)?;
+        if self.composite_fingerprint != expected {
+            return Err("composite fingerprint does not match linked source snapshots".into());
+        }
+        Ok(())
+    }
+
+    pub(crate) fn snapshot_named(&self, name: &str) -> Option<&SourceSetSnapshot> {
+        std::iter::once(&self.analysis)
+            .chain(self.mutations.iter())
+            .find(|snapshot| snapshot.source_set.name == name)
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) enum SourceReadError {
+    NotInManifest { path: String },
+    SourceFingerprintMismatch { path: String },
+    SnapshotUnavailable { path: String, detail: String },
+}
+
+impl SourceReadError {
+    pub(crate) fn reason_code(&self) -> &'static str {
+        match self {
+            Self::NotInManifest { .. } => "source_path_not_in_manifest",
+            Self::SourceFingerprintMismatch { .. } => "source_fingerprint_mismatch",
+            Self::SnapshotUnavailable { .. } => "source_snapshot_unavailable",
+        }
+    }
+
+    #[allow(dead_code)]
+    pub(crate) fn retryable(&self) -> bool {
+        !matches!(self, Self::NotInManifest { .. })
+    }
+}
+
+impl fmt::Display for SourceReadError {
+    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
+        match self {
+            Self::NotInManifest { path } => {
+                write!(formatter, "{}: {path}", self.reason_code())
             }
-            if let Some(existing) = role_identities.insert(
-                (SnapshotRoleKey::Mutation, mutation.source_set.name.as_str()),
-                mutation,
-            ) {
-                if existing != mutation {
-                    return Err(
-                        "one source-set role cannot have conflicting snapshot identities"
-                            .to_string(),
-                    );
-                }
+            Self::SourceFingerprintMismatch { path } => {
+                write!(formatter, "{}: {path}", self.reason_code())
+            }
+            Self::SnapshotUnavailable { path, detail } => {
+                write!(formatter, "{}: {path}: {detail}", self.reason_code())
             }
-            previous = Some(mutation);
         }
-        if self.mutations.windows(2).any(|pair| pair[0] == pair[1]) {
-            return Err("mutation source snapshots must be deduplicated".to_string());
+    }
+}
+
+impl std::error::Error for SourceReadError {}
+
+fn source_fingerprint(
+    source_set: &ResolvedSourceSet,
+    manifest: &SourceManifest,
+) -> Result<String, String> {
+    let mut encoder = FingerprintEncoder::new(SOURCE_FINGERPRINT_DOMAIN);
+    encode_source_identity(&mut encoder, source_set)?;
+    encoder.write_u64(manifest.entries.len() as u64);
+    for (path, entry) in &manifest.entries {
+        encoder.write_string(path)?;
+        match entry {
+            ManifestEntry::Present(file) => {
+                encoder.write_u8(1);
+                encoder.write_u64(file.byte_length);
+                encoder.write_string(&file.content_digest)?;
+            }
+            ManifestEntry::AbsentOptional(tag) => {
+                encoder.write_u8(2);
+                encoder.write_u8(match tag {
+                    OptionalMaterialTag::ParentConfigurations => 1,
+                    OptionalMaterialTag::EdtProject => 2,
+                    OptionalMaterialTag::EdtProjectPmf => 3,
+                    OptionalMaterialTag::EdtConfigurationMdo => 4,
+                    OptionalMaterialTag::EdtSourceConfigurationMdo => 5,
+                });
+            }
         }
+    }
+    Ok(encoder.finish())
+}
+
+fn composite_fingerprint(
+    analysis: &SourceSetSnapshot,
+    mutations: &[SourceSetSnapshot],
+) -> Result<String, String> {
+    let mut encoder = FingerprintEncoder::new(COMPOSITE_FINGERPRINT_DOMAIN);
+    encoder.write_u8(1);
+    encode_source_identity(&mut encoder, &analysis.source_set)?;
+    encoder.write_string(&analysis.source_fingerprint)?;
+    encoder.write_u64(mutations.len() as u64);
+    for mutation in mutations {
+        encoder.write_u8(2);
+        encode_source_identity(&mut encoder, &mutation.source_set)?;
+        encoder.write_string(&mutation.source_fingerprint)?;
+    }
+    Ok(encoder.finish())
+}
+
+fn encode_source_identity(
+    encoder: &mut FingerprintEncoder,
+    source_set: &ResolvedSourceSet,
+) -> Result<(), String> {
+    source_set.validate()?;
+    encoder.write_string(&source_set.name)?;
+    encoder.write_u8(source_set_kind_tag(source_set.kind));
+    encoder.write_u8(source_format_tag(source_set.source_format));
+    encoder.write_string(&source_set.relative_root)?;
+    encoder.write_string(&source_set.mapping_digest)?;
+    Ok(())
+}
+
+struct FingerprintEncoder {
+    hasher: Sha256,
+}
+
+impl FingerprintEncoder {
+    fn new(domain: &[u8]) -> Self {
+        let mut hasher = Sha256::new();
+        hasher.update((domain.len() as u64).to_be_bytes());
+        hasher.update(domain);
+        Self { hasher }
+    }
+
+    fn write_u8(&mut self, value: u8) {
+        self.hasher.update([value]);
+    }
+
+    fn write_u64(&mut self, value: u64) {
+        self.hasher.update(value.to_be_bytes());
+    }
+
+    fn write_string(&mut self, value: &str) -> Result<(), String> {
+        let length = u64::try_from(value.len()).map_err(|_| "value is too large to hash")?;
+        self.write_u64(length);
+        self.hasher.update(value.as_bytes());
         Ok(())
     }
+
+    fn finish(self) -> String {
+        format!("sha256:{:x}", self.hasher.finalize())
+    }
 }
 
-#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
-enum SnapshotRoleKey {
-    Analysis,
-    Mutation,
+fn resolved_source_key(source: &ResolvedSourceSet) -> (String, u8, u8, &str, &str) {
+    (
+        source.name.to_lowercase(),
+        source_set_kind_tag(source.kind),
+        source_format_tag(source.source_format),
+        source.relative_root.as_str(),
+        source.mapping_digest.as_str(),
+    )
 }
 
-fn snapshot_key(
-    role: SnapshotRoleKey,
-    snapshot: &SourceSetSnapshot,
-) -> (SnapshotRoleKey, &str, u8, u8, &str, &str, &str) {
+fn snapshot_key(snapshot: &SourceSetSnapshot) -> (String, u8, u8, &str, &str, &str) {
+    let source = &snapshot.source_set;
     (
-        role,
-        snapshot.source_set.name.as_str(),
-        source_set_kind_tag(snapshot.source_set.kind),
-        source_format_tag(snapshot.source_set.source_format),
-        snapshot.source_set.relative_root.as_str(),
-        snapshot.source_set.mapping_digest.as_str(),
+        source.name.to_lowercase(),
+        source_set_kind_tag(source.kind),
+        source_format_tag(source.source_format),
+        source.relative_root.as_str(),
+        source.mapping_digest.as_str(),
         snapshot.source_fingerprint.as_str(),
     )
 }
 
 fn source_set_kind_tag(kind: SourceSetKind) -> u8 {
     match kind {
         SourceSetKind::Configuration => 1,
         SourceSetKind::Extension => 2,
         SourceSetKind::ExternalProcessor => 3,
         SourceSetKind::ExternalReport => 4,
@@ -150,165 +438,221 @@ fn source_format_tag(format: SourceFormat) -> u8 {
     }
 }
 
 fn stable_component(value: &str, field: &str, maximum: usize) -> Result<(), String> {
     if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
         return Err(format!("{field} must contain 1..={maximum} stable bytes"));
     }
     Ok(())
 }
 
-fn contained_relative_path(path: &str) -> Result<(), String> {
+fn contained_relative_root(path: &str) -> Result<(), String> {
+    if path == "." {
+        return Ok(());
+    }
+    contained_relative_file(path)
+        .map_err(|_| "source root must be `.` or a contained workspace-relative slash path".into())
+}
+
+fn contained_relative_file(path: &str) -> Result<(), String> {
     if path.is_empty()
+        || path == "."
         || path.len() > 4096
         || path.starts_with('/')
         || path.starts_with('\\')
         || path.contains('\\')
         || path.contains(':')
         || path.chars().any(char::is_control)
         || path
             .split('/')
             .any(|component| component.is_empty() || matches!(component, "." | ".."))
     {
-        return Err("source root must be a contained workspace-relative slash path".to_string());
+        return Err("path must be a contained workspace-relative slash path".into());
     }
     Ok(())
 }
 
 fn validate_fingerprint(value: &str) -> Result<(), String> {
     let Some(digest) = value.strip_prefix("sha256:") else {
-        return Err("fingerprint must start with sha256:".to_string());
+        return Err("fingerprint must start with sha256:".into());
     };
     if digest.len() != 64
         || !digest
             .bytes()
             .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
     {
-        return Err("fingerprint must contain 64 lowercase hexadecimal characters".to_string());
+        return Err("fingerprint must contain 64 lowercase hexadecimal characters".into());
     }
     Ok(())
 }
 
+fn optional_tag_matches_path(tag: OptionalMaterialTag, path: &str) -> bool {
+    let suffix = match tag {
+        OptionalMaterialTag::ParentConfigurations => "Ext/ParentConfigurations.bin",
+        OptionalMaterialTag::EdtProject => ".project",
+        OptionalMaterialTag::EdtProjectPmf => "DT-INF/PROJECT.PMF",
+        OptionalMaterialTag::EdtConfigurationMdo => "Configuration/Configuration.mdo",
+        OptionalMaterialTag::EdtSourceConfigurationMdo => "src/Configuration/Configuration.mdo",
+    };
+    path == suffix
+        || path
+            .strip_suffix(suffix)
+            .is_some_and(|prefix| prefix.ends_with('/'))
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
 
-    fn resolved(name: &str, source_format: SourceFormat) -> ResolvedSourceSet {
-        ResolvedSourceSet {
-            name: name.to_string(),
-            kind: SourceSetKind::Extension,
-            relative_root: format!("src/{name}"),
-            source_format,
-            mapping_digest: format!("sha256:{}", "a".repeat(64)),
-        }
+    fn resolved(name: &str) -> ResolvedSourceSet {
+        ResolvedSourceSet::new(
+            name.into(),
+            SourceSetKind::Extension,
+            format!("src/{name}"),
+            SourceFormat::PlatformXml,
+            format!("sha256:{}", "a".repeat(64)),
+        )
+        .unwrap()
     }
 
-    fn snapshot(name: &str, digit: char) -> SourceSetSnapshot {
-        SourceSetSnapshot {
-            source_set: resolved(name, SourceFormat::PlatformXml),
-            source_fingerprint: format!("sha256:{}", digit.to_string().repeat(64)),
-        }
+    fn snapshot(name: &str, byte: u8) -> SourceSetSnapshot {
+        let path = format!("src/{name}/Configuration.xml");
+        let manifest = SourceManifest::new(BTreeMap::from([(
+            path,
+            ManifestEntry::Present(MaterialFile::new(1, format!("sha256:{:064x}", byte)).unwrap()),
+        )]))
+        .unwrap();
+        SourceSetSnapshot::from_manifest(resolved(name), manifest).unwrap()
     }
 
     #[test]
-    fn source_snapshot_keeps_one_analysis_and_sorts_deduplicates_mutations() {
-        let mutation_a = snapshot("a", '2');
-        let mutation_b = snapshot("b", '3');
-        let result = SourceSnapshot::new(
-            SourceSetSnapshot {
-                source_set: resolved("main", SourceFormat::Edt),
-                source_fingerprint: format!("sha256:{}", "1".repeat(64)),
-            },
-            vec![mutation_b.clone(), mutation_a.clone(), mutation_b],
-            format!("sha256:{}", "4".repeat(64)),
+    fn source_snapshot_computes_composite_and_sorts_deduplicates_mutations() {
+        let analysis = snapshot("main", 1);
+        let mutation_a = snapshot("a", 2);
+        let mutation_b = snapshot("b", 3);
+        let first = SourceSnapshot::new(
+            analysis.clone(),
+            vec![mutation_b.clone(), mutation_a.clone(), mutation_b.clone()],
             9,
         )
         .unwrap();
+        let second = SourceSnapshot::new(analysis, vec![mutation_a, mutation_b], 9).unwrap();
 
-        assert_eq!(result.analysis.source_set.source_format, SourceFormat::Edt);
-        assert_eq!(result.mutations, [mutation_a, snapshot("b", '3')]);
-        assert_eq!(result.workspace_epoch, 9);
+        assert_eq!(first, second);
+        assert_eq!(first.workspace_epoch, 9);
     }
 
     #[test]
-    fn source_snapshot_identity_is_permutation_invariant() {
-        let analysis = SourceSetSnapshot {
-            source_set: resolved("main", SourceFormat::Edt),
-            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
-        };
-        let mutation_a = snapshot("a", '2');
-        let mutation_b = snapshot("b", '3');
+    fn composite_snapshot_binds_analysis_and_destination() {
+        let a =
+            SourceSnapshot::new(snapshot("main", 1), vec![snapshot("ExtensionA", 2)], 9).unwrap();
+        let b =
+            SourceSnapshot::new(snapshot("main", 1), vec![snapshot("ExtensionB", 2)], 9).unwrap();
 
-        let first = SourceSnapshot::new(
-            analysis.clone(),
-            vec![mutation_b.clone(), mutation_a.clone()],
-            format!("sha256:{}", "4".repeat(64)),
-            9,
-        )
-        .unwrap();
-        let second = SourceSnapshot::new(
-            analysis,
-            vec![mutation_a, mutation_b],
-            format!("sha256:{}", "4".repeat(64)),
-            9,
-        )
-        .unwrap();
+        assert_ne!(a.composite_fingerprint, b.composite_fingerprint);
+    }
 
-        assert_eq!(first, second);
+    #[test]
+    fn source_fingerprint_binds_mapping_name_kind_format_and_root() {
+        let baseline = snapshot("base", 1);
+        let variants = [
+            ResolvedSourceSet::new(
+                "renamed".into(),
+                baseline.source_set.kind,
+                baseline.source_set.relative_root.clone(),
+                baseline.source_set.source_format,
+                baseline.source_set.mapping_digest.clone(),
+            )
+            .unwrap(),
+            ResolvedSourceSet::new(
+                baseline.source_set.name.clone(),
+                SourceSetKind::Configuration,
+                baseline.source_set.relative_root.clone(),
+                baseline.source_set.source_format,
+                baseline.source_set.mapping_digest.clone(),
+            )
+            .unwrap(),
+            ResolvedSourceSet::new(
+                baseline.source_set.name.clone(),
+                baseline.source_set.kind,
+                "different/root".into(),
+                baseline.source_set.source_format,
+                baseline.source_set.mapping_digest.clone(),
+            )
+            .unwrap(),
+            ResolvedSourceSet::new(
+                baseline.source_set.name.clone(),
+                baseline.source_set.kind,
+                baseline.source_set.relative_root.clone(),
+                SourceFormat::Edt,
+                baseline.source_set.mapping_digest.clone(),
+            )
+            .unwrap(),
+            ResolvedSourceSet::new(
+                baseline.source_set.name.clone(),
+                baseline.source_set.kind,
+                baseline.source_set.relative_root.clone(),
+                baseline.source_set.source_format,
+                format!("sha256:{}", "b".repeat(64)),
+            )
+            .unwrap(),
+        ];
+        for variant in variants {
+            let changed =
+                SourceSetSnapshot::from_manifest(variant, baseline.manifest.clone()).unwrap();
+            assert_ne!(baseline.source_fingerprint, changed.source_fingerprint);
+        }
     }
 
     #[test]
-    fn same_name_and_role_with_conflicting_mapping_identity_is_rejected() {
-        let analysis = SourceSetSnapshot {
-            source_set: resolved("main", SourceFormat::Edt),
-            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
-        };
-        let first = snapshot("extension", '2');
-        let mut variants = Vec::new();
-        let mut kind = first.clone();
-        kind.source_set.kind = SourceSetKind::Configuration;
-        variants.push(kind);
-        let mut format = first.clone();
-        format.source_set.source_format = SourceFormat::Edt;
-        variants.push(format);
-        let mut root = first.clone();
-        root.source_set.relative_root = "different/extension".to_string();
-        variants.push(root);
-        let mut mapping = first.clone();
-        mapping.source_set.mapping_digest = format!("sha256:{}", "b".repeat(64));
-        variants.push(mapping);
-        let mut content = first.clone();
-        content.source_fingerprint = format!("sha256:{}", "3".repeat(64));
-        variants.push(content);
-
-        for conflicting in variants {
-            let result = SourceSnapshot::new(
-                analysis.clone(),
-                vec![first.clone(), conflicting],
-                format!("sha256:{}", "4".repeat(64)),
-                9,
+    fn workspace_root_dot_is_valid_but_embedded_dot_and_traversal_are_not() {
+        assert!(ResolvedSourceSet::new(
+            "main".into(),
+            SourceSetKind::Configuration,
+            ".".into(),
+            SourceFormat::PlatformXml,
+            format!("sha256:{}", "a".repeat(64)),
+        )
+        .is_ok());
+        for invalid in ["", "./src", "src/./cf", "src/../cf", "/src", "C:/src"] {
+            assert!(
+                ResolvedSourceSet::new(
+                    "main".into(),
+                    SourceSetKind::Configuration,
+                    invalid.into(),
+                    SourceFormat::PlatformXml,
+                    format!("sha256:{}", "a".repeat(64)),
+                )
+                .is_err(),
+                "accepted {invalid}"
             );
-
-            assert!(result.is_err());
         }
     }
 
     #[test]
-    fn same_name_with_different_roles_keeps_distinct_identities() {
-        let analysis = SourceSetSnapshot {
-            source_set: resolved("shared", SourceFormat::Edt),
-            source_fingerprint: format!("sha256:{}", "1".repeat(64)),
-        };
-        let mutation = snapshot("shared", '2');
+    fn selection_rejects_mixed_mapping_versions() {
+        let analysis = resolved("main");
+        let mut mutation = resolved("extension");
+        mutation.mapping_digest = format!("sha256:{}", "b".repeat(64));
 
-        let result = SourceSnapshot::new(
-            analysis.clone(),
-            vec![mutation.clone()],
-            format!("sha256:{}", "4".repeat(64)),
-            9,
-        )
-        .unwrap();
+        assert!(ResolvedSourceSelection::new(analysis, vec![mutation]).is_err());
+    }
+
+    #[test]
+    fn selection_validation_rejects_forged_digest_order_and_duplicates() {
+        let valid =
+            ResolvedSourceSelection::new(resolved("main"), vec![resolved("a"), resolved("b")])
+                .unwrap();
+
+        let mut mixed_digest = valid.clone();
+        mixed_digest.mapping_digest = format!("sha256:{}", "b".repeat(64));
+        assert!(mixed_digest.validate().is_err());
+
+        let mut reordered = valid.clone();
+        reordered.mutations.reverse();
+        assert!(reordered.validate().is_err());
 
-        assert_eq!(result.analysis, analysis);
-        assert_eq!(result.mutations, [mutation]);
+        let mut duplicate = valid;
+        duplicate.mutations.push(duplicate.mutations[0].clone());
+        assert!(duplicate.validate().is_err());
     }
 }
diff --git a/crates/unica-coder/src/infrastructure/contained_fs.rs b/crates/unica-coder/src/infrastructure/contained_fs.rs
new file mode 100644
index 0000000..d2bc721
--- /dev/null
+++ b/crates/unica-coder/src/infrastructure/contained_fs.rs
@@ -0,0 +1,650 @@
+use std::fs::{File, Metadata, OpenOptions};
+use std::path::{Component, Path, PathBuf};
+use std::time::SystemTime;
+
+pub(crate) fn canonical_workspace(root: &Path) -> Result<PathBuf, String> {
+    let metadata = std::fs::metadata(root)
+        .map_err(|error| format!("workspace_root_unavailable: {}: {error}", root.display()))?;
+    if !metadata.is_dir() {
+        return Err(format!("workspace_root_not_directory: {}", root.display()));
+    }
+    root.canonicalize()
+        .map_err(|error| format!("workspace_root_unavailable: {}: {error}", root.display()))
+}
+
+pub(crate) fn validate_configured_relative_path(raw: &str, field: &str) -> Result<(), String> {
+    if raw.is_empty() {
+        return Err(format!(
+            "empty_configured_path: `{field}` must not be empty"
+        ));
+    }
+    let path = Path::new(raw);
+    if path.is_absolute() || looks_like_windows_absolute(raw) {
+        return Err(format!("absolute_source_root: `{field}` must be relative"));
+    }
+    if raw.contains('\\') || raw.chars().any(char::is_control) {
+        return Err(format!(
+            "invalid_configured_path: `{field}` contains unsafe bytes"
+        ));
+    }
+    let components = raw.split('/').collect::<Vec<_>>();
+    if components.iter().any(|component| component.is_empty()) {
+        return Err(format!(
+            "empty_path_component: `{field}` contains an empty component"
+        ));
+    }
+    if components
+        .iter()
+        .any(|component| matches!(*component, ".."))
+    {
+        return Err(format!("path_traversal: `{field}` contains `..`"));
+    }
+    if raw != "." && components.contains(&".") {
+        return Err(format!("embedded_current_dir: `{field}` contains `.`"));
+    }
+    Ok(())
+}
+
+pub(crate) fn normalize_relative(base: &str, path: &str) -> Result<String, String> {
+    validate_configured_relative_path(base, "basePath")?;
+    validate_configured_relative_path(path, "path")?;
+    let mut parts = Vec::new();
+    if base != "." {
+        parts.extend(base.split('/'));
+    }
+    if path != "." {
+        parts.extend(path.split('/'));
+    }
+    if parts.is_empty() {
+        Ok(".".into())
+    } else {
+        Ok(parts.join("/"))
+    }
+}
+
+pub(crate) fn resolve_contained_directory(
+    canonical_workspace: &Path,
+    relative: &str,
+) -> Result<PathBuf, String> {
+    validate_configured_relative_path(relative, "source root")?;
+    let root = if relative == "." {
+        canonical_workspace.to_path_buf()
+    } else {
+        canonical_workspace.join(relative)
+    };
+    reject_link_components(canonical_workspace, &root)?;
+    let metadata = std::fs::symlink_metadata(&root)
+        .map_err(|error| format!("source_root_unavailable: {}: {error}", root.display()))?;
+    if metadata_is_link_or_reparse_point(&metadata) {
+        return Err(format!("source_root_symlink: {}", root.display()));
+    }
+    if !metadata.is_dir() {
+        return Err(format!("source_root_not_directory: {}", root.display()));
+    }
+    let canonical = root
+        .canonicalize()
+        .map_err(|error| format!("source_root_unavailable: {}: {error}", root.display()))?;
+    if !canonical.starts_with(canonical_workspace) {
+        return Err(format!("source_root_escape: {}", root.display()));
+    }
+    Ok(canonical)
+}
+
+pub(crate) fn reject_link_components(workspace: &Path, target: &Path) -> Result<(), String> {
+    let relative = target
+        .strip_prefix(workspace)
+        .map_err(|_| format!("path_escape: {}", target.display()))?;
+    let mut current = workspace.to_path_buf();
+    for component in relative.components() {
+        if !matches!(component, Component::Normal(_)) {
+            return Err(format!("invalid_path_component: {}", target.display()));
+        }
+        current.push(component.as_os_str());
+        let metadata = std::fs::symlink_metadata(&current)
+            .map_err(|error| format!("path_unavailable: {}: {error}", current.display()))?;
+        if metadata_is_link_or_reparse_point(&metadata) {
+            return Err(format!("symlink_or_reparse_escape: {}", current.display()));
+        }
+    }
+    Ok(())
+}
+
+pub(crate) struct ContainedOpen {
+    file: File,
+    #[cfg(windows)]
+    parents: Vec<WindowsHandleGuard>,
+    #[cfg(windows)]
+    leaf_snapshot: WindowsHandleSnapshot,
+}
+
+impl ContainedOpen {
+    pub(crate) fn file(&self) -> &File {
+        &self.file
+    }
+
+    pub(crate) fn file_mut(&mut self) -> &mut File {
+        &mut self.file
+    }
+
+    pub(crate) fn validate_after_read(&self) -> Result<(), String> {
+        #[cfg(windows)]
+        {
+            for parent in &self.parents {
+                validate_windows_handle(&parent.file, &parent.snapshot)?;
+            }
+            validate_windows_handle(&self.file, &self.leaf_snapshot)?;
+        }
+        Ok(())
+    }
+}
+
+#[cfg(windows)]
+struct WindowsHandleGuard {
+    file: File,
+    snapshot: WindowsHandleSnapshot,
+}
+
+#[cfg(windows)]
+struct WindowsHandleSnapshot {
+    expected_final: String,
+    identity: FileIdentity,
+    directory: bool,
+}
+
+pub(crate) fn open_no_follow(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
+    #[cfg(unix)]
+    {
+        open_no_follow_unix(workspace, path)
+    }
+    #[cfg(windows)]
+    {
+        open_no_follow_windows(workspace, path)
+    }
+    #[cfg(not(any(unix, windows)))]
+    {
+        let _ = workspace;
+        let _ = path;
+        Err("file_identity_unavailable: contained open is unsupported".into())
+    }
+}
+
+#[cfg(unix)]
+fn open_no_follow_unix(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
+    use std::ffi::CString;
+    use std::os::fd::{AsRawFd, FromRawFd};
+    use std::os::unix::ffi::OsStrExt;
+    use std::os::unix::fs::OpenOptionsExt;
+
+    let relative = path
+        .strip_prefix(workspace)
+        .map_err(|_| format!("path_escape: {}", path.display()))?;
+    let components = relative
+        .components()
+        .map(|component| match component {
+            Component::Normal(value) => CString::new(value.as_bytes())
+                .map_err(|_| format!("invalid_material_path: {}", path.display())),
+            _ => Err(format!("invalid_material_path: {}", path.display())),
+        })
+        .collect::<Result<Vec<_>, _>>()?;
+    let (file_name, parents) = components
+        .split_last()
+        .ok_or_else(|| "material path must name a file".to_string())?;
+    let mut root_options = OpenOptions::new();
+    root_options
+        .read(true)
+        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
+    let mut directory = root_options.open(workspace).map_err(|error| {
+        format!(
+            "workspace_root_unavailable: {}: {error}",
+            workspace.display()
+        )
+    })?;
+    for component in parents {
+        let descriptor = unsafe {
+            libc::openat(
+                directory.as_raw_fd(),
+                component.as_ptr(),
+                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
+            )
+        };
+        if descriptor < 0 {
+            return Err(format!(
+                "material_subtree_unreadable: {}: {}",
+                path.display(),
+                std::io::Error::last_os_error()
+            ));
+        }
+        directory = unsafe { File::from_raw_fd(descriptor) };
+    }
+    let descriptor = unsafe {
+        libc::openat(
+            directory.as_raw_fd(),
+            file_name.as_ptr(),
+            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
+        )
+    };
+    if descriptor < 0 {
+        return Err(format!(
+            "material_file_unreadable: {}: {}",
+            path.display(),
+            std::io::Error::last_os_error()
+        ));
+    }
+    Ok(ContainedOpen {
+        file: unsafe { File::from_raw_fd(descriptor) },
+    })
+}
+
+#[cfg(windows)]
+fn open_no_follow_windows(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
+    use std::os::windows::fs::OpenOptionsExt;
+
+    let relative = path
+        .strip_prefix(workspace)
+        .map_err(|_| format!("path_escape: {}", path.display()))?;
+    let components = relative
+        .components()
+        .map(|component| match component {
+            Component::Normal(value) => Ok(value.to_os_string()),
+            _ => Err(format!("invalid_material_path: {}", path.display())),
+        })
+        .collect::<Result<Vec<_>, _>>()?;
+    if components.is_empty() {
+        return Err("material path must name a file".into());
+    }
+
+    use windows_sys::Win32::Storage::FileSystem::{
+        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
+        FILE_SHARE_READ, FILE_SHARE_WRITE,
+    };
+
+    let mut root_options = OpenOptions::new();
+    root_options
+        .read(true)
+        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
+        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
+    let root = root_options.open(workspace).map_err(|error| {
+        format!(
+            "workspace_root_unavailable: {}: {error}",
+            workspace.display()
+        )
+    })?;
+    let root_snapshot = snapshot_windows_handle(&root, true)?;
+    let root_volume = windows_identity_volume(&root_snapshot.identity);
+    let mut parents = vec![WindowsHandleGuard {
+        file: root,
+        snapshot: root_snapshot,
+    }];
+    let mut expected_final = parents[0].snapshot.expected_final.clone();
+    let mut leaf = None;
+    for (index, component) in components.iter().enumerate() {
+        let is_leaf = index + 1 == components.len();
+        let parent = &parents
+            .last()
+            .ok_or_else(|| "file_identity_unavailable: missing parent handle".to_string())?
+            .file;
+        let handle = open_windows_component(parent, component, is_leaf)?;
+        let component = windows_component_string(component)?;
+        expected_final = format!("{}\\{component}", expected_final.trim_end_matches('\\'));
+        let mut snapshot = snapshot_windows_handle(&handle, !is_leaf)?;
+        if windows_identity_volume(&snapshot.identity) != root_volume
+            || !windows_paths_equal(&snapshot.expected_final, &expected_final)?
+        {
+            return Err(format!("symlink_or_reparse_escape: {expected_final}"));
+        }
+        snapshot.expected_final = expected_final.clone();
+        if is_leaf {
+            leaf = Some((handle, snapshot));
+        } else {
+            parents.push(WindowsHandleGuard {
+                file: handle,
+                snapshot,
+            });
+        }
+    }
+    let (file, leaf_snapshot) = leaf.ok_or_else(|| "material path must name a file".to_string())?;
+    Ok(ContainedOpen {
+        file,
+        parents,
+        leaf_snapshot,
+    })
+}
+
+#[cfg(windows)]
+fn open_windows_component(
+    parent: &File,
+    component: &std::ffi::OsStr,
+    leaf: bool,
+) -> Result<File, String> {
+    use std::os::windows::ffi::OsStrExt;
+    use std::os::windows::io::{AsRawHandle, FromRawHandle};
+    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
+    use windows_sys::Wdk::Storage::FileSystem::{
+        NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
+        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
+    };
+    use windows_sys::Win32::Foundation::{HANDLE, UNICODE_STRING};
+    use windows_sys::Win32::Storage::FileSystem::{
+        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
+        FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
+    };
+    use windows_sys::Win32::System::Kernel::OBJ_CASE_INSENSITIVE;
+    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
+
+    let mut name = component.encode_wide().collect::<Vec<_>>();
+    let byte_length = name
+        .len()
+        .checked_mul(std::mem::size_of::<u16>())
+        .and_then(|length| u16::try_from(length).ok())
+        .ok_or_else(|| "invalid_material_path: Windows component is too long".to_string())?;
+    if name.is_empty() || name.contains(&0) {
+        return Err("invalid_material_path: invalid Windows component".into());
+    }
+    let unicode = UNICODE_STRING {
+        Length: byte_length,
+        MaximumLength: byte_length,
+        Buffer: name.as_mut_ptr(),
+    };
+    let attributes = OBJECT_ATTRIBUTES {
+        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
+        RootDirectory: parent.as_raw_handle() as HANDLE,
+        ObjectName: &unicode,
+        Attributes: OBJ_CASE_INSENSITIVE as u32,
+        SecurityDescriptor: std::ptr::null(),
+        SecurityQualityOfService: std::ptr::null(),
+    };
+    let mut raw: HANDLE = 0;
+    let mut io_status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
+    let desired_access = FILE_READ_ATTRIBUTES
+        | SYNCHRONIZE
+        | if leaf {
+            FILE_READ_DATA
+        } else {
+            FILE_LIST_DIRECTORY
+        };
+    let create_options = FILE_OPEN_REPARSE_POINT
+        | FILE_SYNCHRONOUS_IO_NONALERT
+        | if leaf {
+            FILE_NON_DIRECTORY_FILE
+        } else {
+            FILE_DIRECTORY_FILE
+        };
+    let status = unsafe {
+        NtCreateFile(
+            &mut raw,
+            desired_access,
+            &attributes,
+            &mut io_status,
+            std::ptr::null(),
+            0,
+            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
+            FILE_OPEN,
+            create_options,
+            std::ptr::null(),
+            0,
+        )
+    };
+    if status < 0 || raw == 0 {
+        return Err(format!(
+            "material_file_unreadable: NtCreateFile failed with NTSTATUS 0x{:08x}",
+            status as u32
+        ));
+    }
+    Ok(unsafe { File::from_raw_handle(raw as _) })
+}
+
+#[cfg(windows)]
+fn windows_component_string(component: &std::ffi::OsStr) -> Result<String, String> {
+    use std::os::windows::ffi::OsStrExt;
+    String::from_utf16(&component.encode_wide().collect::<Vec<_>>())
+        .map_err(|_| "invalid_material_path: Windows component is not valid UTF-16".into())
+}
+
+#[cfg(windows)]
+fn windows_final_path(file: &File) -> Result<String, String> {
+    use std::os::windows::io::AsRawHandle;
+    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
+
+    let required =
+        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle() as _, std::ptr::null_mut(), 0, 0) };
+    if required == 0 {
+        return Err("file_identity_unavailable: final handle path length".into());
+    }
+    let mut buffer = vec![0u16; required as usize + 1];
+    let length = unsafe {
+        GetFinalPathNameByHandleW(
+            file.as_raw_handle() as _,
+            buffer.as_mut_ptr(),
+            u32::try_from(buffer.len())
+                .map_err(|_| "file_identity_unavailable: final handle path too long")?,
+            0,
+        )
+    };
+    if length == 0 || length as usize >= buffer.len() {
+        return Err("file_identity_unavailable: final handle path".into());
+    }
+    String::from_utf16(&buffer[..length as usize])
+        .map_err(|_| "file_identity_unavailable: final handle path is not UTF-16".into())
+}
+
+#[cfg(windows)]
+fn windows_paths_equal(left: &str, right: &str) -> Result<bool, String> {
+    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
+
+    let left = left.encode_utf16().collect::<Vec<_>>();
+    let right = right.encode_utf16().collect::<Vec<_>>();
+    let result = unsafe {
+        CompareStringOrdinal(
+            left.as_ptr(),
+            i32::try_from(left.len())
+                .map_err(|_| "file_identity_unavailable: final path too long")?,
+            right.as_ptr(),
+            i32::try_from(right.len())
+                .map_err(|_| "file_identity_unavailable: final path too long")?,
+            1,
+        )
+    };
+    if result == 0 {
+        return Err("file_identity_unavailable: final path comparison failed".into());
+    }
+    Ok(result == CSTR_EQUAL)
+}
+
+#[cfg(windows)]
+fn snapshot_windows_handle(file: &File, directory: bool) -> Result<WindowsHandleSnapshot, String> {
+    let metadata = file
+        .metadata()
+        .map_err(|error| format!("file_identity_unavailable: handle metadata: {error}"))?;
+    if metadata_is_link_or_reparse_point(&metadata)
+        || (directory && !metadata.is_dir())
+        || (!directory && !metadata.is_file())
+    {
+        return Err("symlink_or_reparse_escape: opened handle type".into());
+    }
+    Ok(WindowsHandleSnapshot {
+        expected_final: windows_final_path(file)?,
+        identity: windows_file_identity(file)?,
+        directory,
+    })
+}
+
+#[cfg(windows)]
+fn validate_windows_handle(file: &File, expected: &WindowsHandleSnapshot) -> Result<(), String> {
+    let actual = snapshot_windows_handle(file, expected.directory)?;
+    if actual.identity != expected.identity
+        || !windows_paths_equal(&actual.expected_final, &expected.expected_final)?
+    {
+        return Err("source_snapshot_unavailable: contained handle changed after open".into());
+    }
+    Ok(())
+}
+
+#[cfg(windows)]
+fn windows_file_identity(file: &File) -> Result<FileIdentity, String> {
+    use std::os::windows::io::AsRawHandle;
+    use windows_sys::Win32::Storage::FileSystem::{
+        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
+    };
+
+    let mut information: FILE_ID_INFO = unsafe { std::mem::zeroed() };
+    let success = unsafe {
+        GetFileInformationByHandleEx(
+            file.as_raw_handle() as _,
+            FileIdInfo,
+            (&mut information as *mut FILE_ID_INFO).cast(),
+            std::mem::size_of::<FILE_ID_INFO>() as u32,
+        )
+    };
+    if success == 0 {
+        return Err("file_identity_unavailable: GetFileInformationByHandleEx failed".into());
+    }
+    Ok(FileIdentity::Windows {
+        volume: information.VolumeSerialNumber,
+        id: information.FileId.Identifier,
+    })
+}
+
+#[cfg(windows)]
+fn windows_identity_volume(identity: &FileIdentity) -> u64 {
+    let FileIdentity::Windows { volume, .. } = identity;
+    *volume
+}
+
+pub(crate) fn metadata_is_link_or_reparse_point(metadata: &Metadata) -> bool {
+    if metadata.file_type().is_symlink() {
+        return true;
+    }
+    #[cfg(windows)]
+    {
+        use std::os::windows::fs::MetadataExt;
+        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
+        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
+    }
+    #[cfg(not(windows))]
+    {
+        false
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) struct FileObservation {
+    pub(crate) identity: FileIdentity,
+    pub(crate) length: u64,
+    pub(crate) modified: Option<SystemTime>,
+    pub(crate) platform_metadata: u128,
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub(crate) enum FileIdentity {
+    #[cfg(unix)]
+    Unix { device: u64, inode: u64 },
+    #[cfg(windows)]
+    Windows { volume: u64, id: [u8; 16] },
+    #[cfg(not(any(unix, windows)))]
+    Unsupported,
+}
+
+pub(crate) fn observe_regular_file(
+    metadata: &Metadata,
+    path: &Path,
+) -> Result<FileObservation, String> {
+    if metadata_is_link_or_reparse_point(metadata) || !metadata.is_file() {
+        return Err(format!("material_file_not_regular: {}", path.display()));
+    }
+    #[cfg(unix)]
+    let (identity, platform_metadata) = {
+        use std::os::unix::fs::MetadataExt;
+        (
+            FileIdentity::Unix {
+                device: metadata.dev(),
+                inode: metadata.ino(),
+            },
+            ((metadata.mode() as u128) << 96)
+                | ((metadata.ctime() as u64 as u128) << 32)
+                | metadata.ctime_nsec() as u64 as u128,
+        )
+    };
+    #[cfg(windows)]
+    let (identity, platform_metadata) = {
+        return Err(format!(
+            "file_identity_unavailable: {}: path metadata has no stable file identity",
+            path.display()
+        ));
+        #[allow(unreachable_code)]
+        (
+            FileIdentity::Windows {
+                volume: 0,
+                id: [0; 16],
+            },
+            0,
+        )
+    };
+    #[cfg(not(any(unix, windows)))]
+    let (identity, platform_metadata) = {
+        return Err(format!("file_identity_unavailable: {}", path.display()));
+        #[allow(unreachable_code)]
+        (FileIdentity::Unsupported, 0)
+    };
+    Ok(FileObservation {
+        identity,
+        length: metadata.len(),
+        modified: metadata.modified().ok(),
+        platform_metadata,
+    })
+}
+
+pub(crate) fn observe_open_file(file: &File, path: &Path) -> Result<FileObservation, String> {
+    let metadata = file
+        .metadata()
+        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
+    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+        return Err(format!("material_file_not_regular: {}", path.display()));
+    }
+    #[cfg(unix)]
+    {
+        observe_regular_file(&metadata, path)
+    }
+    #[cfg(windows)]
+    {
+        Ok(FileObservation {
+            identity: windows_file_identity(file)?,
+            length: metadata.len(),
+            modified: metadata.modified().ok(),
+            platform_metadata: 0,
+        })
+    }
+    #[cfg(not(any(unix, windows)))]
+    {
+        Err(format!("file_identity_unavailable: {}", path.display()))
+    }
+}
+
+pub(crate) fn slash_relative(workspace: &Path, path: &Path) -> Result<String, String> {
+    let relative = path
+        .strip_prefix(workspace)
+        .map_err(|_| format!("path_escape: {}", path.display()))?;
+    let mut parts = Vec::new();
+    for component in relative.components() {
+        let Component::Normal(value) = component else {
+            return Err(format!("invalid_path_component: {}", path.display()));
+        };
+        let value = value
+            .to_str()
+            .ok_or_else(|| format!("non_utf8_material_path: {}", path.display()))?;
+        if value.is_empty() || matches!(value, "." | "..") || value.chars().any(char::is_control) {
+            return Err(format!("invalid_material_path: {}", path.display()));
+        }
+        parts.push(value);
+    }
+    if parts.is_empty() {
+        return Err("material path must name a file".into());
+    }
+    Ok(parts.join("/"))
+}
+
+fn looks_like_windows_absolute(raw: &str) -> bool {
+    let bytes = raw.as_bytes();
+    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
+        || raw.starts_with("//")
+        || raw.starts_with("\\\\")
+}
diff --git a/crates/unica-coder/src/infrastructure/mod.rs b/crates/unica-coder/src/infrastructure/mod.rs
index e68324f..3345193 100644
--- a/crates/unica-coder/src/infrastructure/mod.rs
+++ b/crates/unica-coder/src/infrastructure/mod.rs
@@ -1,18 +1,22 @@
 pub(crate) mod bundled_tools;
+pub(crate) mod contained_fs;
 pub mod internal_adapters;
 pub(crate) mod metadata_kinds;
 pub mod native_operations;
 pub mod path_policy;
+pub(crate) mod platform_xml;
 pub mod plugin_runtime;
+pub(crate) mod project_sources;
 pub(crate) mod redaction;
 pub(crate) mod runtime_jobs;
+pub(crate) mod source_snapshot;
 pub mod workspace_index;
 pub mod workspace_services;
 pub mod workspace_state;
 
 use serde::Serialize;
 
 #[derive(Debug, Clone, Serialize)]
 pub struct AdapterOutcome {
     pub ok: bool,
     pub summary: String,
diff --git a/crates/unica-coder/src/infrastructure/platform_xml.rs b/crates/unica-coder/src/infrastructure/platform_xml.rs
new file mode 100644
index 0000000..efbb0d8
--- /dev/null
+++ b/crates/unica-coder/src/infrastructure/platform_xml.rs
@@ -0,0 +1,267 @@
+use crate::domain::discovery_registry::metadata_kind;
+use roxmltree::{Document, Node};
+use std::collections::BTreeSet;
+
+#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
+pub(crate) struct RootRegistration {
+    pub(crate) kind: String,
+    pub(crate) directory: String,
+    pub(crate) name: String,
+}
+
+#[derive(Debug, Clone, PartialEq, Eq, Default)]
+pub(crate) struct NestedRegistrations {
+    pub(crate) forms: Vec<String>,
+    pub(crate) templates: Vec<String>,
+    pub(crate) commands: Vec<String>,
+}
+
+pub(crate) fn parse_configuration_registrations(
+    bytes: &[u8],
+) -> Result<Vec<RootRegistration>, String> {
+    let text = std::str::from_utf8(bytes)
+        .map_err(|_| "malformed_registration: Configuration.xml is not UTF-8")?;
+    let document = Document::parse(text)
+        .map_err(|error| format!("malformed_registration: Configuration.xml: {error}"))?;
+    let metadata_root = document.root_element();
+    require_local_name(metadata_root, "MetaDataObject", "Configuration.xml root")?;
+    let configuration = exactly_one_direct_element(metadata_root, "Configuration")?;
+    let child_objects = exactly_one_direct_element(configuration, "ChildObjects")?;
+    let mut registrations = Vec::new();
+    let mut paths = BTreeSet::new();
+    for node in child_objects.children().filter(Node::is_element) {
+        let tag = node.tag_name().name();
+        let kind = metadata_kind(tag).ok_or_else(|| {
+            format!("unknown_registration_kind: Configuration.xml ChildObjects/{tag}")
+        })?;
+        let name = registration_value(node, "Configuration.xml")?;
+        let folded_path = format!("{}/{}.xml", kind.directory, name).to_lowercase();
+        if !paths.insert(folded_path) {
+            return Err(format!(
+                "duplicate_registration: Configuration.xml contains duplicate {tag}/{name}"
+            ));
+        }
+        registrations.push(RootRegistration {
+            kind: tag.to_string(),
+            directory: kind.directory.to_string(),
+            name,
+        });
+    }
+    registrations.sort();
+    Ok(registrations)
+}
+
+pub(crate) fn parse_registered_descriptor(
+    bytes: &[u8],
+    registration: &RootRegistration,
+) -> Result<NestedRegistrations, String> {
+    let text = std::str::from_utf8(bytes).map_err(|_| {
+        format!(
+            "malformed_registered_object: {}/{}.xml is not UTF-8",
+            registration.directory, registration.name
+        )
+    })?;
+    let document = Document::parse(text).map_err(|error| {
+        format!(
+            "malformed_registered_object: {}/{}.xml: {error}",
+            registration.directory, registration.name
+        )
+    })?;
+    let metadata_root = document.root_element();
+    require_local_name(
+        metadata_root,
+        "MetaDataObject",
+        "registered descriptor root",
+    )?;
+    let object = exactly_one_direct_element(metadata_root, &registration.kind)?;
+    let properties = exactly_one_direct_element(object, "Properties")?;
+    let name_node = exactly_one_direct_element(properties, "Name")?;
+    let actual_name = registration_value(name_node, "registered descriptor Name")?;
+    if actual_name != registration.name {
+        return Err(format!(
+            "registered_object_identity_mismatch: expected {} {}, descriptor names {}",
+            registration.kind, registration.name, actual_name
+        ));
+    }
+
+    let child_objects = optional_one_direct_element(object, "ChildObjects")?;
+    let mut nested = NestedRegistrations::default();
+    let mut forms = BTreeSet::new();
+    let mut templates = BTreeSet::new();
+    let mut commands = BTreeSet::new();
+    if let Some(child_objects) = child_objects {
+        for child in child_objects.children().filter(Node::is_element) {
+            let value = registration_value(child, "registered descriptor ChildObjects")?;
+            match child.tag_name().name() {
+                "Form" => {
+                    if !forms.insert(value.to_lowercase()) {
+                        return Err(format!("duplicate_nested_registration: Form/{value}"));
+                    }
+                    nested.forms.push(value);
+                }
+                "Template" => {
+                    if !templates.insert(value.to_lowercase()) {
+                        return Err(format!("duplicate_nested_registration: Template/{value}"));
+                    }
+                    nested.templates.push(value);
+                }
+                "Command" => {
+                    if !commands.insert(value.to_lowercase()) {
+                        return Err(format!("duplicate_nested_registration: Command/{value}"));
+                    }
+                    nested.commands.push(value);
+                }
+                _ => {}
+            }
+        }
+    }
+    nested.forms.sort_by_key(|name| name.to_lowercase());
+    nested.templates.sort_by_key(|name| name.to_lowercase());
+    nested.commands.sort_by_key(|name| name.to_lowercase());
+    Ok(nested)
+}
+
+fn registration_value(node: Node<'_, '_>, context: &str) -> Result<String, String> {
+    if node.children().any(|child| child.is_element()) {
+        return Err(format!(
+            "invalid_registration_value: {context}: element content is forbidden"
+        ));
+    }
+    let mut semantic_text = node
+        .children()
+        .filter_map(|child| child.text())
+        .filter(|text| !text.is_empty());
+    let text = semantic_text.next().unwrap_or_default();
+    if semantic_text.next().is_some() {
+        return Err(format!(
+            "invalid_registration_value: {context}: ambiguous text content"
+        ));
+    }
+    if text.is_empty()
+        || text.trim() != text
+        || matches!(text, "." | "..")
+        || text.contains('/')
+        || text.contains('\\')
+        || text.contains(':')
+        || text.chars().any(char::is_control)
+        || !text
+            .chars()
+            .all(|character| character.is_alphanumeric() || character == '_')
+    {
+        return Err(format!("invalid_registration_value: {context}: {text:?}"));
+    }
+    Ok(text.to_string())
+}
+
+fn require_local_name(node: Node<'_, '_>, expected: &str, context: &str) -> Result<(), String> {
+    if node.tag_name().name() != expected {
+        return Err(format!(
+            "malformed_registration: {context} must be {expected}, got {}",
+            node.tag_name().name()
+        ));
+    }
+    Ok(())
+}
+
+fn exactly_one_direct_element<'a, 'input>(
+    parent: Node<'a, 'input>,
+    name: &str,
+) -> Result<Node<'a, 'input>, String> {
+    optional_one_direct_element(parent, name)?.ok_or_else(|| {
+        format!(
+            "malformed_registration: {} must contain direct {name}",
+            parent.tag_name().name()
+        )
+    })
+}
+
+fn optional_one_direct_element<'a, 'input>(
+    parent: Node<'a, 'input>,
+    name: &str,
+) -> Result<Option<Node<'a, 'input>>, String> {
+    let mut matches = parent
+        .children()
+        .filter(Node::is_element)
+        .filter(|node| node.tag_name().name() == name);
+    let first = matches.next();
+    if matches.next().is_some() {
+        return Err(format!(
+            "malformed_registration: {} contains duplicate direct {name}",
+            parent.tag_name().name()
+        ));
+    }
+    Ok(first)
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    #[test]
+    fn configuration_parser_requires_direct_known_safe_unique_registrations() {
+        let valid = br#"<MetaDataObject><Configuration><ChildObjects><CommonModule>Safe_Name1</CommonModule></ChildObjects></Configuration></MetaDataObject>"#;
+        assert_eq!(
+            parse_configuration_registrations(valid).unwrap(),
+            [RootRegistration {
+                kind: "CommonModule".into(),
+                directory: "CommonModules".into(),
+                name: "Safe_Name1".into(),
+            }]
+        );
+
+        for invalid in [
+            "<MetaDataObject><Configuration><ChildObjects><Unknown>X</Unknown></ChildObjects></Configuration></MetaDataObject>",
+            "<MetaDataObject><Configuration><ChildObjects><CommonModule>../X</CommonModule></ChildObjects></Configuration></MetaDataObject>",
+            "<MetaDataObject><Configuration><Wrapper><ChildObjects><CommonModule>X</CommonModule></ChildObjects></Wrapper></Configuration></MetaDataObject>",
+            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule><CommonModule>x</CommonModule></ChildObjects></Configuration></MetaDataObject>",
+        ] {
+            assert!(parse_configuration_registrations(invalid.as_bytes()).is_err(), "accepted {invalid}");
+        }
+    }
+
+    #[test]
+    fn configuration_parser_accepts_utf8_bom_and_namespace_prefixes() {
+        let prefixed = b"\xef\xbb\xbf<md:MetaDataObject xmlns:md=\"urn:1c\"><md:Configuration><md:ChildObjects><md:CommonModule>Safe</md:CommonModule></md:ChildObjects></md:Configuration></md:MetaDataObject>";
+        assert_eq!(
+            parse_configuration_registrations(prefixed).unwrap(),
+            [RootRegistration {
+                kind: "CommonModule".into(),
+                directory: "CommonModules".into(),
+                name: "Safe".into(),
+            }]
+        );
+    }
+
+    #[test]
+    fn parsers_reject_mixed_registration_content() {
+        let root_mixed = br#"<MetaDataObject><Configuration><ChildObjects><CommonModule>Safe<Trap/>Name</CommonModule></ChildObjects></Configuration></MetaDataObject>"#;
+        assert!(parse_configuration_registrations(root_mixed).is_err());
+
+        let registration = RootRegistration {
+            kind: "Document".into(),
+            directory: "Documents".into(),
+            name: "Sale".into(),
+        };
+        let nested_mixed = br#"<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Form>Main<Trap/>Form</Form></ChildObjects></Document></MetaDataObject>"#;
+        assert!(parse_registered_descriptor(nested_mixed, &registration).is_err());
+    }
+
+    #[test]
+    fn descriptor_parser_binds_kind_name_and_nested_form_template_registration() {
+        let registration = RootRegistration {
+            kind: "Document".into(),
+            directory: "Documents".into(),
+            name: "Sale".into(),
+        };
+        let valid = br#"<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Form>Main</Form><Template>Print</Template><Command>Post</Command><Attribute>Number</Attribute></ChildObjects></Document></MetaDataObject>"#;
+        let nested = parse_registered_descriptor(valid, &registration).unwrap();
+        assert_eq!(nested.forms, ["Main"]);
+        assert_eq!(nested.templates, ["Print"]);
+        assert_eq!(nested.commands, ["Post"]);
+
+        let wrong_kind = br#"<MetaDataObject><Catalog><Properties><Name>Sale</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#;
+        let wrong_name = br#"<MetaDataObject><Document><Properties><Name>Other</Name></Properties><ChildObjects/></Document></MetaDataObject>"#;
+        assert!(parse_registered_descriptor(wrong_kind, &registration).is_err());
+        assert!(parse_registered_descriptor(wrong_name, &registration).is_err());
+    }
+}
diff --git a/crates/unica-coder/src/infrastructure/project_sources.rs b/crates/unica-coder/src/infrastructure/project_sources.rs
new file mode 100644
index 0000000..d65041b
--- /dev/null
+++ b/crates/unica-coder/src/infrastructure/project_sources.rs
@@ -0,0 +1,945 @@
+use super::contained_fs::{
+    canonical_workspace, metadata_is_link_or_reparse_point, normalize_relative, observe_open_file,
+    observe_regular_file, open_no_follow, reject_link_components, resolve_contained_directory,
+    slash_relative, validate_configured_relative_path,
+};
+use crate::application::discovery::ports::{
+    DiscoveryError, DiscoveryExecutionContext, ProjectSourceResolverPort, SourceReadinessError,
+    SourceReadinessReason, SourceRole,
+};
+use crate::domain::project_sources::{
+    ProjectSourceMap, ProjectSourceSet, SourceFormat, SourceSetKind,
+};
+use crate::domain::source_snapshot::{ResolvedSourceSelection, ResolvedSourceSet};
+use serde_yaml::Value as YamlValue;
+use sha2::{Digest, Sha256};
+use std::collections::{BTreeMap, BTreeSet};
+use std::io::Read;
+use std::path::{Path, PathBuf};
+
+const MAPPING_DOMAIN: &[u8] = b"unica.project-source-topology.v1";
+
+#[derive(Debug, Clone)]
+struct ConfiguredSourceSet {
+    name: String,
+    kind: SourceSetKind,
+    relative_root: String,
+    default_format: Option<SourceFormat>,
+}
+
+#[derive(Debug, Clone)]
+struct LoadedSourceMap {
+    canonical_workspace: PathBuf,
+    config_path: Option<PathBuf>,
+    configured_format_raw: Option<String>,
+    source_sets: Vec<ProjectSourceSet>,
+    mapping_digest: String,
+}
+
+pub(crate) struct FilesystemProjectSourceResolver;
+
+impl ProjectSourceResolverPort for FilesystemProjectSourceResolver {
+    fn resolve_all(
+        &self,
+        context: &DiscoveryExecutionContext,
+        requested_analysis: Option<&str>,
+        requested_mutations: &[String],
+    ) -> Result<ResolvedSourceSelection, DiscoveryError> {
+        resolve_source_selection_typed(
+            Path::new(&context.workspace_root),
+            requested_analysis,
+            requested_mutations,
+        )
+    }
+}
+
+pub fn discover_project_source_map(workspace_root: &Path) -> Result<ProjectSourceMap, String> {
+    let loaded = load_source_map(workspace_root)?;
+    Ok(ProjectSourceMap {
+        workspace_root: loaded.canonical_workspace.display().to_string(),
+        config_path: loaded.config_path.map(|path| path.display().to_string()),
+        source_sets: loaded.source_sets,
+        configured_format_raw: loaded.configured_format_raw,
+    })
+}
+
+pub(crate) fn resolve_source_selection(
+    workspace_root: &Path,
+    requested_analysis: Option<&str>,
+    requested_mutations: &[String],
+) -> Result<ResolvedSourceSelection, String> {
+    resolve_source_selection_typed(workspace_root, requested_analysis, requested_mutations)
+        .map_err(|error| error.to_string())
+}
+
+pub(crate) fn resolve_source_selection_typed(
+    workspace_root: &Path,
+    requested_analysis: Option<&str>,
+    requested_mutations: &[String],
+) -> Result<ResolvedSourceSelection, DiscoveryError> {
+    let loaded = load_source_map(workspace_root).map_err(DiscoveryError::Operation)?;
+    let eligible = loaded
+        .source_sets
+        .iter()
+        .filter(|source| analysis_readiness(source).is_ok())
+        .collect::<Vec<_>>();
+    let analysis = match requested_analysis {
+        Some(name) => resolve_analysis_named(&loaded, name)?,
+        None if eligible.len() == 1 => {
+            resolved(&loaded, eligible[0]).map_err(DiscoveryError::Operation)?
+        }
+        None if eligible.is_empty() => {
+            if loaded.source_sets.len() == 1 {
+                return Err(analysis_readiness(&loaded.source_sets[0]).unwrap_err());
+            }
+            return Err(DiscoveryError::Operation(
+                "no_eligible_source_set: discovery v1 requires an authoritative source layout"
+                    .into(),
+            ));
+        }
+        None => {
+            return Err(DiscoveryError::Operation("ambiguous_source_set: sourceSet is required when multiple eligible source sets exist".into()));
+        }
+    };
+    let mut mutation_names = requested_mutations.to_vec();
+    mutation_names.sort_by_key(|name| name.to_lowercase());
+    mutation_names.dedup_by(|left, right| left.to_lowercase() == right.to_lowercase());
+    let mutations = mutation_names
+        .iter()
+        .map(|name| resolve_mutation_named(&loaded, name))
+        .collect::<Result<Vec<_>, _>>()?;
+    ResolvedSourceSelection::new(analysis, mutations).map_err(DiscoveryError::Operation)
+}
+
+fn find_named<'a>(
+    loaded: &'a LoadedSourceMap,
+    name: &str,
+) -> Result<&'a ProjectSourceSet, DiscoveryError> {
+    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
+        return Err(DiscoveryError::Operation(
+            "invalid_source_set_name: sourceSet must contain stable non-blank bytes".into(),
+        ));
+    }
+    loaded
+        .source_sets
+        .iter()
+        .find(|source| source.name == name)
+        .ok_or_else(|| DiscoveryError::Operation(format!("source_set_not_found: {name}")))
+}
+
+fn resolve_analysis_named(
+    loaded: &LoadedSourceMap,
+    name: &str,
+) -> Result<ResolvedSourceSet, DiscoveryError> {
+    let source = find_named(loaded, name)?;
+    analysis_readiness(source)?;
+    resolved(loaded, source).map_err(DiscoveryError::Operation)
+}
+
+fn resolve_mutation_named(
+    loaded: &LoadedSourceMap,
+    name: &str,
+) -> Result<ResolvedSourceSet, DiscoveryError> {
+    let source = find_named(loaded, name)?;
+    if source.kind != SourceSetKind::Extension {
+        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnsupportedDestinationKind,
+            SourceRole::Destination,
+            &source.name,
+        )));
+    }
+    if source.source_format != SourceFormat::PlatformXml {
+        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnsupportedDestinationFormat,
+            SourceRole::Destination,
+            &source.name,
+        )));
+    }
+    resolved(loaded, source).map_err(DiscoveryError::Operation)
+}
+
+fn analysis_readiness(source: &ProjectSourceSet) -> Result<(), DiscoveryError> {
+    if !matches!(
+        source.kind,
+        SourceSetKind::Configuration | SourceSetKind::Extension
+    ) {
+        return Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnsupportedSourceKind,
+            SourceRole::Analysis,
+            &source.name,
+        )));
+    }
+    match source.source_format {
+        SourceFormat::PlatformXml => Ok(()),
+        SourceFormat::Edt
+            if source.kind == SourceSetKind::Configuration
+                && source
+                    .format_evidence
+                    .iter()
+                    .any(|evidence| !evidence.starts_with("v8project.yaml:")) =>
+        {
+            Ok(())
+        }
+        SourceFormat::Edt => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnsupportedSourceFormat,
+            SourceRole::Analysis,
+            &source.name,
+        ))),
+        SourceFormat::Unknown => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::UnknownSourceFormat,
+            SourceRole::Analysis,
+            &source.name,
+        ))),
+        SourceFormat::Invalid => Err(DiscoveryError::SourceReadiness(SourceReadinessError::new(
+            SourceReadinessReason::InvalidSourceFormat,
+            SourceRole::Analysis,
+            &source.name,
+        ))),
+    }
+}
+
+fn resolved(
+    loaded: &LoadedSourceMap,
+    source: &ProjectSourceSet,
+) -> Result<ResolvedSourceSet, String> {
+    ResolvedSourceSet::new(
+        source.name.clone(),
+        source.kind,
+        source.path.clone(),
+        source.source_format,
+        loaded.mapping_digest.clone(),
+    )
+}
+
+fn load_source_map(workspace_root: &Path) -> Result<LoadedSourceMap, String> {
+    let canonical_workspace = canonical_workspace(workspace_root)?;
+    let config_path = canonical_workspace.join("v8project.yaml");
+    let (configured, configured_format_raw, actual_config_path) =
+        match std::fs::symlink_metadata(&config_path) {
+            Ok(metadata) => {
+                if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+                    return Err(format!(
+                        "source_map_config_not_regular: {}",
+                        config_path.display()
+                    ));
+                }
+                let bytes = read_stable_file(&canonical_workspace, &config_path)?;
+                let (configured, format) = parse_configured_source_sets(&bytes)?;
+                (configured, format, Some(config_path))
+            }
+            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None, None),
+            Err(error) => {
+                return Err(format!(
+                    "source_map_config_unavailable: {}: {error}",
+                    config_path.display()
+                ));
+            }
+        };
+
+    let configured = if configured.is_empty() {
+        autodetect_source_sets(&canonical_workspace)?
+    } else {
+        configured
+    };
+    validate_source_set_identities(&canonical_workspace, &configured)?;
+    let mut source_sets = configured
+        .iter()
+        .map(|source| detect_source_set_format(&canonical_workspace, source))
+        .collect::<Result<Vec<_>, _>>()?;
+    let mapping_digest = mapping_digest(&source_sets)?;
+    // Public map preserves configured order. Identity hashing canonicalizes it.
+    if actual_config_path.is_none() {
+        source_sets.sort_by_key(|source| source.name.to_lowercase());
+    }
+    Ok(LoadedSourceMap {
+        canonical_workspace,
+        config_path: actual_config_path,
+        configured_format_raw,
+        source_sets,
+        mapping_digest,
+    })
+}
+
+fn parse_configured_source_sets(
+    bytes: &[u8],
+) -> Result<(Vec<ConfiguredSourceSet>, Option<String>), String> {
+    let text = std::str::from_utf8(bytes)
+        .map_err(|_| "source_map_config_invalid: v8project.yaml is not UTF-8")?;
+    let yaml = serde_yaml::from_str::<YamlValue>(text)
+        .map_err(|error| format!("source_map_config_invalid: {error}"))?;
+    if !yaml.is_mapping() {
+        return Err("source_map_config_invalid: root must be a mapping".into());
+    }
+    let configured_format_raw = optional_strict_string(&yaml, "format")?;
+    let default_format = configured_format_raw
+        .as_deref()
+        .and_then(source_format_from_config);
+    let base_path = optional_strict_string(&yaml, "basePath")?.unwrap_or_else(|| ".".into());
+    validate_configured_relative_path(&base_path, "basePath")?;
+    let mut source_sets = Vec::new();
+    match yaml_mapping_get(&yaml, "source-set") {
+        None | Some(YamlValue::Null) => {}
+        Some(YamlValue::Sequence(entries)) => {
+            for entry in entries {
+                source_sets.push(config_source_set_from_yaml(
+                    None,
+                    entry,
+                    &base_path,
+                    default_format,
+                )?);
+            }
+        }
+        Some(YamlValue::Mapping(entries)) => {
+            for (key, entry) in entries {
+                let name = key.as_str().ok_or_else(|| {
+                    "source_map_config_invalid: source-set mapping keys must be strings".to_string()
+                })?;
+                source_sets.push(config_source_set_from_yaml(
+                    Some(name),
+                    entry,
+                    &base_path,
+                    default_format,
+                )?);
+            }
+        }
+        Some(_) => {
+            return Err("source_map_config_invalid: source-set must be a list or mapping".into())
+        }
+    }
+    Ok((source_sets, configured_format_raw))
+}
+
+fn config_source_set_from_yaml(
+    mapped_name: Option<&str>,
+    entry: &YamlValue,
+    base_path: &str,
+    default_format: Option<SourceFormat>,
+) -> Result<ConfiguredSourceSet, String> {
+    if !entry.is_mapping() {
+        return Err("source_map_config_invalid: source-set entries must be mappings".into());
+    }
+    let entry_name = optional_strict_string(entry, "name")?;
+    if mapped_name.is_some() && entry_name.is_some() {
+        return Err("source_map_config_invalid: mapped source-set must not repeat name".into());
+    }
+    let name = mapped_name
+        .map(str::to_string)
+        .or(entry_name)
+        .unwrap_or_else(|| "main".into());
+    validate_source_name(&name)?;
+    let source_type = optional_strict_string(entry, "type")?;
+    let purpose = optional_strict_string(entry, "purpose")?;
+    if source_type.is_some() && purpose.is_some() && source_type != purpose {
+        return Err("source_map_config_invalid: source-set type and purpose conflict".into());
+    }
+    let kind = source_set_kind_from_config(
+        source_type
+            .or(purpose)
+            .as_deref()
+            .unwrap_or("CONFIGURATION"),
+    )?;
+    let path = optional_strict_string(entry, "path")?.unwrap_or_else(|| ".".into());
+    let relative_root = normalize_relative(base_path, &path)?;
+    Ok(ConfiguredSourceSet {
+        name,
+        kind,
+        relative_root,
+        default_format,
+    })
+}
+
+fn validate_source_name(name: &str) -> Result<(), String> {
+    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
+        return Err("invalid_source_set_name: name must contain stable non-blank bytes".into());
+    }
+    Ok(())
+}
+
+fn optional_strict_string(value: &YamlValue, key: &str) -> Result<Option<String>, String> {
+    match yaml_mapping_get(value, key) {
+        None => Ok(None),
+        Some(YamlValue::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
+        Some(YamlValue::String(_)) => Err(format!(
+            "source_map_config_invalid: `{key}` must not be empty"
+        )),
+        Some(_) => Err(format!(
+            "source_map_config_invalid: field `{key}` must be a string"
+        )),
+    }
+}
+
+fn yaml_mapping_get<'a>(value: &'a YamlValue, key: &str) -> Option<&'a YamlValue> {
+    value.as_mapping()?.get(YamlValue::String(key.to_string()))
+}
+
+fn validate_source_set_identities(
+    workspace: &Path,
+    source_sets: &[ConfiguredSourceSet],
+) -> Result<(), String> {
+    let mut names = BTreeSet::new();
+    let mut roots = BTreeMap::new();
+    for source in source_sets {
+        if !names.insert(source.name.to_lowercase()) {
+            return Err(format!("duplicate_source_set_name: {}", source.name));
+        }
+        let canonical_root = resolve_contained_directory(workspace, &source.relative_root)?;
+        if let Some(previous) = roots.insert(canonical_root.clone(), source.name.clone()) {
+            return Err(format!(
+                "duplicate_source_root: {} and {} resolve to {}",
+                previous,
+                source.name,
+                canonical_root.display()
+            ));
+        }
+    }
+    Ok(())
+}
+
+fn autodetect_source_sets(workspace: &Path) -> Result<Vec<ConfiguredSourceSet>, String> {
+    for relative_root in [".", "src", "src/cf"] {
+        let root = if relative_root == "." {
+            workspace.to_path_buf()
+        } else {
+            workspace.join(relative_root)
+        };
+        if !root.is_dir() {
+            continue;
+        }
+        if regular_marker(&root.join("Configuration.xml"))?
+            || regular_marker(&root.join("Configuration/Configuration.mdo"))?
+            || regular_marker(&root.join("src/Configuration/Configuration.mdo"))?
+        {
+            return Ok(vec![ConfiguredSourceSet {
+                name: "main".into(),
+                kind: SourceSetKind::Configuration,
+                relative_root: relative_root.into(),
+                default_format: None,
+            }]);
+        }
+    }
+    Ok(Vec::new())
+}
+
+fn detect_source_set_format(
+    workspace: &Path,
+    configured: &ConfiguredSourceSet,
+) -> Result<ProjectSourceSet, String> {
+    let root = resolve_contained_directory(workspace, &configured.relative_root)?;
+    let mut platform_evidence = Vec::new();
+    let configuration = root.join("Configuration.xml");
+    if regular_marker(&configuration)? {
+        platform_evidence.push(slash_relative(workspace, &configuration)?);
+    }
+    if matches!(
+        configured.kind,
+        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
+    ) && root.is_dir()
+    {
+        for entry in std::fs::read_dir(&root)
+            .map_err(|error| format!("source_root_unreadable: {}: {error}", root.display()))?
+        {
+            let entry = entry
+                .map_err(|error| format!("source_root_unreadable: {}: {error}", root.display()))?;
+            let path = entry.path();
+            if path.extension().and_then(|extension| extension.to_str()) == Some("xml")
+                && entry.file_name() != "ConfigDumpInfo.xml"
+                && regular_marker(&path)?
+            {
+                platform_evidence.push(slash_relative(workspace, &path)?);
+            }
+        }
+    }
+    let mut edt_evidence = Vec::new();
+    for relative in [
+        ".project",
+        "DT-INF/PROJECT.PMF",
+        "Configuration/Configuration.mdo",
+        "src/Configuration/Configuration.mdo",
+    ] {
+        let path = root.join(relative);
+        if regular_marker(&path)? {
+            edt_evidence.push(slash_relative(workspace, &path)?);
+        }
+    }
+    platform_evidence.sort();
+    platform_evidence.dedup();
+    edt_evidence.sort();
+    edt_evidence.dedup();
+    let source_format = match (platform_evidence.is_empty(), edt_evidence.is_empty()) {
+        (false, false) => SourceFormat::Invalid,
+        (false, true) => SourceFormat::PlatformXml,
+        (true, false) => SourceFormat::Edt,
+        (true, true) => configured.default_format.unwrap_or(SourceFormat::Unknown),
+    };
+    let mut format_evidence = platform_evidence;
+    format_evidence.extend(edt_evidence);
+    if format_evidence.is_empty() {
+        if let Some(default) = configured.default_format {
+            format_evidence.push(match default {
+                SourceFormat::PlatformXml => "v8project.yaml:format=DESIGNER".into(),
+                SourceFormat::Edt => "v8project.yaml:format=EDT".into(),
+                SourceFormat::Unknown | SourceFormat::Invalid => "v8project.yaml:format".into(),
+            });
+        }
+    }
+    Ok(ProjectSourceSet {
+        name: configured.name.clone(),
+        kind: configured.kind,
+        path: configured.relative_root.clone(),
+        source_format,
+        format_evidence,
+    })
+}
+
+fn mapping_digest(source_sets: &[ProjectSourceSet]) -> Result<String, String> {
+    let mut topology = source_sets.iter().collect::<Vec<_>>();
+    topology.sort_by(|left, right| {
+        left.name
+            .to_lowercase()
+            .cmp(&right.name.to_lowercase())
+            .then_with(|| left.name.cmp(&right.name))
+    });
+    let mut hasher = Sha256::new();
+    write_hash_bytes(&mut hasher, MAPPING_DOMAIN)?;
+    write_hash_u64(&mut hasher, topology.len() as u64);
+    for source in topology {
+        write_hash_bytes(&mut hasher, source.name.as_bytes())?;
+        hasher.update([
+            source_kind_tag(source.kind),
+            source_format_tag(source.source_format),
+        ]);
+        write_hash_bytes(&mut hasher, source.path.as_bytes())?;
+    }
+    Ok(format!("sha256:{:x}", hasher.finalize()))
+}
+
+fn write_hash_bytes(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
+    let length = u64::try_from(bytes.len()).map_err(|_| "mapping value too large")?;
+    write_hash_u64(hasher, length);
+    hasher.update(bytes);
+    Ok(())
+}
+
+fn write_hash_u64(hasher: &mut Sha256, value: u64) {
+    hasher.update(value.to_be_bytes());
+}
+
+fn source_kind_tag(kind: SourceSetKind) -> u8 {
+    match kind {
+        SourceSetKind::Configuration => 1,
+        SourceSetKind::Extension => 2,
+        SourceSetKind::ExternalProcessor => 3,
+        SourceSetKind::ExternalReport => 4,
+    }
+}
+
+fn source_format_tag(format: SourceFormat) -> u8 {
+    match format {
+        SourceFormat::PlatformXml => 1,
+        SourceFormat::Edt => 2,
+        SourceFormat::Unknown => 3,
+        SourceFormat::Invalid => 4,
+    }
+}
+
+fn source_set_kind_from_config(raw: &str) -> Result<SourceSetKind, String> {
+    match raw.to_ascii_uppercase().as_str() {
+        "CONFIGURATION" => Ok(SourceSetKind::Configuration),
+        "EXTENSION" => Ok(SourceSetKind::Extension),
+        "EXTERNAL_DATA_PROCESSORS" => Ok(SourceSetKind::ExternalProcessor),
+        "EXTERNAL_REPORTS" => Ok(SourceSetKind::ExternalReport),
+        other => Err(format!("unsupported_source_set_type: {other}")),
+    }
+}
+
+fn source_format_from_config(raw: &str) -> Option<SourceFormat> {
+    match raw.to_ascii_uppercase().as_str() {
+        "DESIGNER" | "PLATFORM_XML" | "XML" => Some(SourceFormat::PlatformXml),
+        "EDT" => Some(SourceFormat::Edt),
+        _ => None,
+    }
+}
+
+fn regular_marker(path: &Path) -> Result<bool, String> {
+    match std::fs::symlink_metadata(path) {
+        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
+            Err(format!("symlink_or_reparse_marker: {}", path.display()))
+        }
+        Ok(metadata) => Ok(metadata.is_file()),
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
+        Err(error) => Err(format!("marker_unavailable: {}: {error}", path.display())),
+    }
+}
+
+fn read_stable_file(workspace: &Path, path: &Path) -> Result<Vec<u8>, String> {
+    reject_link_components(workspace, path)?;
+    let path_before = std::fs::symlink_metadata(path)
+        .map_err(|error| format!("source_map_config_unavailable: {}: {error}", path.display()))?;
+    if metadata_is_link_or_reparse_point(&path_before) || !path_before.is_file() {
+        return Err(format!("source_map_config_not_regular: {}", path.display()));
+    }
+    #[cfg(unix)]
+    let before = observe_regular_file(&path_before, path)?;
+    let before_length = path_before.len();
+    let mut contained = open_no_follow(workspace, path)?;
+    let opened = observe_open_file(contained.file(), path)?;
+    #[cfg(unix)]
+    if before != opened {
+        return Err("source_mapping_changed: source map changed during resolution".into());
+    }
+    #[cfg(windows)]
+    if before_length != opened.length {
+        return Err("source_mapping_changed: source map changed during resolution".into());
+    }
+    let capacity = usize::try_from(before_length.min(64 * 1024))
+        .map_err(|_| "source_map_config_too_large: cannot address file")?;
+    let mut bytes = Vec::with_capacity(capacity);
+    let read_limit = before_length
+        .checked_add(1)
+        .ok_or("source_map_config_too_large: length overflow")?;
+    contained
+        .file_mut()
+        .take(read_limit)
+        .read_to_end(&mut bytes)
+        .map_err(|error| format!("source_map_config_unavailable: {}: {error}", path.display()))?;
+    if bytes.len() as u64 > before_length {
+        return Err("source_mapping_changed: source map changed during resolution".into());
+    }
+    let after_handle = observe_open_file(contained.file(), path)?;
+    contained.validate_after_read()?;
+    #[cfg(unix)]
+    let after_path = observe_regular_file(
+        &std::fs::symlink_metadata(path).map_err(|error| {
+            format!("source_map_config_unavailable: {}: {error}", path.display())
+        })?,
+        path,
+    )?;
+    #[cfg(windows)]
+    let after_path = {
+        let reopened = open_no_follow(workspace, path)?;
+        let observation = observe_open_file(reopened.file(), path)?;
+        reopened.validate_after_read()?;
+        observation
+    };
+    #[cfg(unix)]
+    let baseline = before;
+    #[cfg(windows)]
+    let baseline = opened;
+    if baseline != after_handle || baseline != after_path || bytes.len() as u64 != before_length {
+        return Err("source_mapping_changed: source map changed during resolution".into());
+    }
+    Ok(bytes)
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use std::fs;
+    use std::time::{SystemTime, UNIX_EPOCH};
+
+    #[test]
+    fn legacy_map_preserves_external_detection_and_edt_analysis_readiness() {
+        let root = fixture("source-map-legacy");
+        write(
+            &root.join("v8project.yaml"),
+            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: edt\n  - name: epf\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
+        );
+        write(&root.join("edt/.project"), "x");
+        write(&root.join("epf/Tool.xml"), "x");
+        let map = discover_project_source_map(&root).unwrap();
+        assert_eq!(map.source_sets[0].source_format, SourceFormat::Edt);
+        assert_eq!(map.source_sets[1].kind, SourceSetKind::ExternalProcessor);
+        let edt = resolve_source_selection_typed(&root, Some("main"), &[]).unwrap();
+        assert_eq!(edt.analysis.source_format, SourceFormat::Edt);
+        let external = resolve_source_selection_typed(&root, Some("epf"), &[]).unwrap_err();
+        let DiscoveryError::SourceReadiness(external) = external else {
+            panic!("expected typed readiness error");
+        };
+        assert_eq!(external.reason_code(), "unsupported_source_kind");
+        assert_eq!(external.role, SourceRole::Analysis);
+        assert!(!external.retryable());
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn config_dump_info_alone_does_not_prove_platform_xml_format() {
+        let root = fixture("source-map-config-dump-info-only");
+        write(
+            &root.join("v8project.yaml"),
+            "source-set:\n - { name: main, type: CONFIGURATION, path: main }\n",
+        );
+        write(&root.join("main/ConfigDumpInfo.xml"), "x");
+
+        let map = discover_project_source_map(&root).unwrap();
+        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
+        assert!(map.source_sets[0].format_evidence.is_empty());
+        let error = resolve_source_selection_typed(&root, Some("main"), &[]).unwrap_err();
+        let DiscoveryError::SourceReadiness(error) = error else {
+            panic!("expected typed source readiness error")
+        };
+        assert_eq!(error.reason_code(), "unknown_source_format");
+        assert_eq!(error.role, SourceRole::Analysis);
+        assert!(!error.retryable());
+
+        write(&root.join("main/Configuration.xml"), "x");
+        let map = discover_project_source_map(&root).unwrap();
+        assert_eq!(map.source_sets[0].source_format, SourceFormat::PlatformXml);
+        assert_eq!(
+            map.source_sets[0].format_evidence,
+            vec!["main/Configuration.xml"]
+        );
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn public_map_rejects_a_truly_missing_configured_root() {
+        let root = fixture("source-map-missing-root");
+        write(
+            &root.join("v8project.yaml"),
+            "format: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: missing }\n",
+        );
+
+        let error = discover_project_source_map(&root).unwrap_err();
+        assert!(error.contains("unavailable"), "{error}");
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn public_map_rejects_dangling_leaf_and_ancestor_symlinks() {
+        use std::os::unix::fs::symlink;
+
+        for (name, configured_path, link_path) in [
+            ("leaf", "linked", "linked"),
+            ("ancestor", "linked/main", "linked"),
+        ] {
+            let root = fixture(&format!("source-map-dangling-{name}"));
+            write(
+                &root.join("v8project.yaml"),
+                &format!(
+                    "format: DESIGNER\nsource-set:\n - {{ name: main, type: CONFIGURATION, path: {configured_path} }}\n"
+                ),
+            );
+            symlink(root.join("does-not-exist"), root.join(link_path)).unwrap();
+
+            let error = discover_project_source_map(&root).unwrap_err();
+            assert!(
+                error.contains("symlink") || error.contains("reparse"),
+                "{name}: {error}"
+            );
+            fs::remove_dir_all(root).unwrap();
+        }
+    }
+
+    #[test]
+    fn external_config_dump_info_alone_does_not_prove_platform_xml_format() {
+        let root = fixture("source-map-external-config-dump-info-only");
+        write(
+            &root.join("v8project.yaml"),
+            "source-set:\n - { name: tool, type: EXTERNAL_DATA_PROCESSORS, path: tool }\n",
+        );
+        write(&root.join("tool/ConfigDumpInfo.xml"), "x");
+
+        let map = discover_project_source_map(&root).unwrap();
+        assert_eq!(map.source_sets[0].source_format, SourceFormat::Unknown);
+        assert!(map.source_sets[0].format_evidence.is_empty());
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn rejects_duplicate_names_absolute_traversal_empty_and_duplicate_roots() {
+        let cases = [
+            (
+                "source-set:\n - { name: Main, path: a }\n - { name: main, path: b }\n",
+                "duplicate_source_set_name",
+            ),
+            (
+                "source-set:\n - { name: main, path: /tmp }\n",
+                "absolute_source_root",
+            ),
+            (
+                "source-set:\n - { name: main, path: ../a }\n",
+                "path_traversal",
+            ),
+            (
+                "source-set:\n - { name: main, path: '' }\n",
+                "must not be empty",
+            ),
+            (
+                "source-set:\n - { name: one }\n - { name: two, path: . }\n",
+                "duplicate_source_root",
+            ),
+        ];
+        for (index, (yaml, reason)) in cases.iter().enumerate() {
+            let root = fixture(&format!("source-map-invalid-{index}"));
+            fs::create_dir_all(root.join("a")).unwrap();
+            fs::create_dir_all(root.join("b")).unwrap();
+            write(&root.join("v8project.yaml"), yaml);
+            let error = discover_project_source_map(&root).unwrap_err();
+            assert!(error.contains(reason), "expected {reason}, got {error}");
+            fs::remove_dir_all(root).unwrap();
+        }
+    }
+
+    #[test]
+    fn mapping_digest_is_semantic_and_batch_resolution_is_canonical() {
+        let root = fixture("source-map-semantic-digest");
+        fs::create_dir_all(root.join("main")).unwrap();
+        fs::create_dir_all(root.join("ext")).unwrap();
+        write(&root.join("main/Configuration.xml"), "x");
+        write(&root.join("ext/Configuration.xml"), "x");
+        write(&root.join("v8project.yaml"), "# comment\ninfobase: ignored\nformat: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: Extension, type: EXTENSION, path: ext }\n");
+        let before = resolve_source_selection(&root, Some("main"), &["Extension".into()]).unwrap();
+        write(&root.join("v8project.yaml"), "source-set:\n - { path: ext, type: EXTENSION, name: Extension }\n - { path: main, name: main, type: CONFIGURATION }\nformat: DESIGNER\nother: value\n");
+        let reordered = resolve_source_selection(
+            &root,
+            Some("main"),
+            &["Extension".into(), "Extension".into()],
+        )
+        .unwrap();
+        assert_eq!(
+            before.analysis.mapping_digest,
+            reordered.analysis.mapping_digest
+        );
+        assert_eq!(reordered.mutations.len(), 1);
+        write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: renamed, type: CONFIGURATION, path: main }\n - { name: Extension, type: EXTENSION, path: ext }\n");
+        let changed =
+            resolve_source_selection(&root, Some("renamed"), &["Extension".into()]).unwrap();
+        assert_ne!(
+            before.analysis.mapping_digest,
+            changed.analysis.mapping_digest
+        );
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn auto_selection_requires_exactly_one_eligible_platform_xml_set() {
+        let root = fixture("source-map-ambiguous");
+        for dir in ["main", "ext"] {
+            fs::create_dir_all(root.join(dir)).unwrap();
+            write(&root.join(dir).join("Configuration.xml"), "x");
+        }
+        write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: ext, type: EXTENSION, path: ext }\n");
+        assert!(resolve_source_selection(&root, None, &[])
+            .unwrap_err()
+            .contains("ambiguous_source_set"));
+        assert_eq!(
+            resolve_source_selection(&root, Some("main"), &[])
+                .unwrap()
+                .analysis
+                .relative_root,
+            "main"
+        );
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn destination_role_requires_platform_xml_extension() {
+        let root = fixture("source-map-destination-role");
+        for dir in ["main", "edt-ext"] {
+            fs::create_dir_all(root.join(dir)).unwrap();
+        }
+        write(&root.join("main/Configuration.xml"), "x");
+        write(&root.join("edt-ext/.project"), "x");
+        write(&root.join("v8project.yaml"), "source-set:\n - { name: main, type: CONFIGURATION, path: main }\n - { name: edt, type: EXTENSION, path: edt-ext }\n");
+        for (name, code) in [
+            ("main", "unsupported_destination_kind"),
+            ("edt", "unsupported_destination_format"),
+        ] {
+            let error =
+                resolve_source_selection_typed(&root, Some("main"), &[name.into()]).unwrap_err();
+            let DiscoveryError::SourceReadiness(error) = error else {
+                panic!("expected typed readiness error")
+            };
+            assert_eq!(error.reason_code(), code);
+            assert_eq!(error.role, SourceRole::Destination);
+            assert!(!error.retryable());
+        }
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn edt_analysis_still_resolves_and_validates_requested_destinations() {
+        let root = fixture("source-map-edt-with-destinations");
+        for directory in ["edt", "valid-ext", "invalid-ext"] {
+            fs::create_dir_all(root.join(directory)).unwrap();
+        }
+        write(&root.join("edt/.project"), "x");
+        write(&root.join("valid-ext/Configuration.xml"), "x");
+        write(&root.join("invalid-ext/.project"), "x");
+        write(
+            &root.join("v8project.yaml"),
+            "source-set:\n - { name: edt, type: CONFIGURATION, path: edt }\n - { name: valid, type: EXTENSION, path: valid-ext }\n - { name: invalid, type: EXTENSION, path: invalid-ext }\n",
+        );
+
+        let selection =
+            resolve_source_selection_typed(&root, Some("edt"), &["valid".into()]).unwrap();
+        assert_eq!(selection.analysis.source_format, SourceFormat::Edt);
+        assert_eq!(selection.mutations.len(), 1);
+        assert_eq!(selection.mutations[0].name, "valid");
+
+        let error =
+            resolve_source_selection_typed(&root, Some("edt"), &["invalid".into()]).unwrap_err();
+        let DiscoveryError::SourceReadiness(error) = error else {
+            panic!("expected typed destination readiness error")
+        };
+        assert_eq!(error.reason_code(), "unsupported_destination_format");
+        assert_eq!(error.role, SourceRole::Destination);
+        assert!(!error.retryable());
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[test]
+    fn markerless_declared_edt_is_typed_unsupported_analysis_format() {
+        let root = fixture("source-map-markerless-edt");
+        fs::create_dir_all(root.join("edt")).unwrap();
+        write(
+            &root.join("v8project.yaml"),
+            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
+        );
+
+        let error = resolve_source_selection_typed(&root, Some("main"), &[]).unwrap_err();
+        let DiscoveryError::SourceReadiness(error) = error else {
+            panic!("expected typed readiness error");
+        };
+        assert_eq!(error.reason_code(), "unsupported_source_format");
+        assert_eq!(error.role, SourceRole::Analysis);
+        assert!(!error.retryable());
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn rejects_symlink_source_root() {
+        use std::os::unix::fs::symlink;
+        let root = fixture("source-map-symlink");
+        fs::create_dir_all(root.join("real")).unwrap();
+        symlink(root.join("real"), root.join("linked")).unwrap();
+        write(
+            &root.join("v8project.yaml"),
+            "format: DESIGNER\nsource-set:\n - { name: main, path: linked }\n",
+        );
+        assert!(discover_project_source_map(&root)
+            .unwrap_err()
+            .contains("symlink"));
+        fs::remove_dir_all(root).unwrap();
+    }
+
+    fn fixture(name: &str) -> PathBuf {
+        let nonce = SystemTime::now()
+            .duration_since(UNIX_EPOCH)
+            .unwrap()
+            .as_nanos();
+        let root =
+            std::env::temp_dir().join(format!("unica-{name}-{}-{nonce}", std::process::id()));
+        fs::create_dir_all(&root).unwrap();
+        root
+    }
+
+    fn write(path: &Path, text: &str) {
+        if let Some(parent) = path.parent() {
+            fs::create_dir_all(parent).unwrap();
+        }
+        fs::write(path, text).unwrap();
+    }
+}
diff --git a/crates/unica-coder/src/infrastructure/source_snapshot.rs b/crates/unica-coder/src/infrastructure/source_snapshot.rs
new file mode 100644
index 0000000..6051254
--- /dev/null
+++ b/crates/unica-coder/src/infrastructure/source_snapshot.rs
@@ -0,0 +1,2389 @@
+#[cfg(unix)]
+use super::contained_fs::observe_regular_file;
+use super::contained_fs::{
+    canonical_workspace, metadata_is_link_or_reparse_point, observe_open_file, open_no_follow,
+    reject_link_components, resolve_contained_directory, slash_relative, FileObservation,
+};
+use super::platform_xml::{parse_configuration_registrations, parse_registered_descriptor};
+use super::project_sources::resolve_source_selection;
+use crate::application::discovery::ports::{
+    SnapshotCaptureError, SnapshotCaptureReason, SourceSnapshotPort,
+};
+use crate::domain::discovery_registry::{EDT_DIAGNOSTIC_MARKERS_V1, SOURCE_ROOT_EXT_ARTIFACTS_V1};
+use crate::domain::project_sources::SourceFormat;
+use crate::domain::source_snapshot::{
+    ManifestEntry, MaterialFile, OptionalMaterialTag, ResolvedSourceSet, SourceManifest,
+    SourceReadError, SourceSetSnapshot, SourceSnapshot,
+};
+use sha2::{Digest, Sha256};
+use std::collections::{BTreeMap, BTreeSet};
+use std::io::Read;
+use std::path::{Path, PathBuf};
+use std::sync::Arc;
+use std::time::{Duration, Instant};
+
+pub(crate) const MAX_SNAPSHOT_FILES: usize = 200_000;
+pub(crate) const MAX_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
+pub(crate) const MAX_SNAPSHOT_ELAPSED: Duration = Duration::from_secs(120);
+pub(crate) const MAX_SNAPSHOT_TRAVERSAL_ENTRIES: usize = 1_600_000;
+pub(crate) const MAX_SNAPSHOT_TRAVERSAL_DEPTH: usize = 64;
+pub(crate) const MAX_SNAPSHOT_XML_BYTES: u64 = 64 * 1024 * 1024;
+const OPTIONAL_PARENT_CONFIGURATIONS: &str = "Ext/ParentConfigurations.bin";
+const IGNORED_REGISTERED_SUBTREE_DIRECTORIES: &[&str] = &[".git", ".build", "target", "dist"];
+
+#[derive(Debug, Clone, Copy)]
+struct SnapshotLimits {
+    max_files: usize,
+    max_bytes: u64,
+    max_elapsed: Duration,
+    max_traversal_entries: usize,
+    max_traversal_depth: usize,
+    max_xml_bytes: u64,
+}
+
+impl Default for SnapshotLimits {
+    fn default() -> Self {
+        Self {
+            max_files: MAX_SNAPSHOT_FILES,
+            max_bytes: MAX_SNAPSHOT_BYTES,
+            max_elapsed: MAX_SNAPSHOT_ELAPSED,
+            max_traversal_entries: MAX_SNAPSHOT_TRAVERSAL_ENTRIES,
+            max_traversal_depth: MAX_SNAPSHOT_TRAVERSAL_DEPTH,
+            max_xml_bytes: MAX_SNAPSHOT_XML_BYTES,
+        }
+    }
+}
+
+trait SnapshotClock: Send + Sync {
+    fn now(&self) -> Duration;
+}
+
+#[allow(dead_code)]
+struct SystemSnapshotClock {
+    origin: Instant,
+}
+
+#[allow(dead_code)]
+impl SystemSnapshotClock {
+    fn new() -> Self {
+        Self {
+            origin: Instant::now(),
+        }
+    }
+}
+
+impl SnapshotClock for SystemSnapshotClock {
+    fn now(&self) -> Duration {
+        self.origin.elapsed()
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+enum CaptureEvent {
+    InitialPathScansComplete,
+    #[cfg(unix)]
+    BeforeContainedOpen(String),
+    ContainedOpenEstablished(String),
+    FileHashed(String),
+    BeforeFinalIdentityValidation,
+}
+
+trait CaptureHook: Send + Sync {
+    fn on_event(&self, event: &CaptureEvent);
+}
+
+#[allow(dead_code)]
+struct NoopCaptureHook;
+
+impl CaptureHook for NoopCaptureHook {
+    fn on_event(&self, _event: &CaptureEvent) {}
+}
+
+pub(crate) struct FilesystemSourceSnapshots {
+    workspace: PathBuf,
+    limits: SnapshotLimits,
+    clock: Arc<dyn SnapshotClock>,
+    hook: Arc<dyn CaptureHook>,
+}
+
+impl FilesystemSourceSnapshots {
+    // Constructed by Task 5 when concrete providers are wired publicly.
+    #[allow(dead_code)]
+    pub(crate) fn new(workspace: &Path) -> Result<Self, String> {
+        Ok(Self {
+            workspace: canonical_workspace(workspace)?,
+            limits: SnapshotLimits::default(),
+            clock: Arc::new(SystemSnapshotClock::new()),
+            hook: Arc::new(NoopCaptureHook),
+        })
+    }
+
+    fn capture_authoritative(
+        &self,
+        analysis: &ResolvedSourceSet,
+        mutation_sources: &[ResolvedSourceSet],
+        workspace_epoch: u64,
+    ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+        let mutation_names = mutation_sources
+            .iter()
+            .map(|source| source.name.clone())
+            .collect::<Vec<_>>();
+        let before =
+            resolve_source_selection(&self.workspace, Some(&analysis.name), &mutation_names)
+                .map_err(classify_mapping_revalidation_error)?;
+        if before.analysis != *analysis || before.mutations != mutation_sources {
+            return Err(SnapshotCaptureError::source_changed(
+                "source mapping no longer matches the resolved selection",
+            ));
+        }
+
+        let started = self.clock.now();
+        let mut budget = CaptureBudget::new(self.limits, started);
+        let mut sources = Vec::with_capacity(1 + mutation_sources.len());
+        sources.push(analysis.clone());
+        sources.extend_from_slice(mutation_sources);
+        let mut initial_plans = Vec::with_capacity(sources.len());
+        for source in &sources {
+            budget.check_deadline(self.clock.as_ref())?;
+            let plan = scan_source_plan(&self.workspace, source, &mut budget, self.clock.as_ref())?;
+            initial_plans.push(plan);
+        }
+        let unique_present = initial_plans
+            .iter()
+            .flat_map(|plan| plan.present.iter().cloned())
+            .collect::<BTreeSet<_>>();
+        budget.register_files(unique_present.len())?;
+        self.hook.on_event(&CaptureEvent::InitialPathScansComplete);
+
+        let mut captured = Vec::with_capacity(initial_plans.len());
+        let mut material_cache = BTreeMap::<String, (MaterialFile, FileObservation)>::new();
+        for plan in &initial_plans {
+            let mut entries = BTreeMap::new();
+            let mut observations = BTreeMap::new();
+            for relative in &plan.present {
+                budget.check_deadline(self.clock.as_ref())?;
+                let (material, observation) = if let Some(cached) = material_cache.get(relative) {
+                    cached.clone()
+                } else {
+                    let path = self.workspace.join(relative);
+                    let read = read_stable_bytes(
+                        &self.workspace,
+                        &path,
+                        budget.remaining_bytes()?,
+                        Some(self.hook.as_ref()),
+                    )?;
+                    budget.register_bytes(read.bytes.len() as u64)?;
+                    let digest = digest_bytes(&read.bytes);
+                    let material = MaterialFile::new(read.bytes.len() as u64, digest)?;
+                    let cached = (material, read.observation);
+                    material_cache.insert(relative.clone(), cached.clone());
+                    self.hook
+                        .on_event(&CaptureEvent::FileHashed(relative.clone()));
+                    cached
+                };
+                observations.insert(
+                    relative.clone(),
+                    (observation, material.content_digest.clone()),
+                );
+                entries.insert(relative.clone(), ManifestEntry::Present(material));
+            }
+            for (relative, tag) in &plan.absent_optional {
+                entries.insert(relative.clone(), ManifestEntry::AbsentOptional(*tag));
+            }
+            captured.push((
+                plan.source_set.clone(),
+                SourceManifest::new(entries)?,
+                observations,
+            ));
+        }
+
+        let mut final_plans = Vec::with_capacity(sources.len());
+        for source in &sources {
+            budget.check_deadline(self.clock.as_ref())?;
+            final_plans.push(
+                scan_source_plan(&self.workspace, source, &mut budget, self.clock.as_ref())
+                    .map_err(classify_final_scan_error)?,
+            );
+        }
+        if initial_plans != final_plans {
+            return Err(SnapshotCaptureError::source_changed(
+                "authoritative path set changed during capture",
+            ));
+        }
+
+        self.hook
+            .on_event(&CaptureEvent::BeforeFinalIdentityValidation);
+        let final_observations = captured
+            .iter()
+            .flat_map(|(_, _, observations)| observations.iter())
+            .map(|(path, observation)| (path.clone(), observation.clone()))
+            .collect::<BTreeMap<_, _>>();
+        for (relative, (expected_observation, expected_digest)) in &final_observations {
+            budget.check_deadline(self.clock.as_ref())?;
+            let read = read_stable_bytes(
+                &self.workspace,
+                &self.workspace.join(relative),
+                expected_observation.length,
+                Some(self.hook.as_ref()),
+            )
+            .map_err(classify_final_present_revalidation_error)?;
+            if &read.observation != expected_observation
+                || &digest_bytes(&read.bytes) != expected_digest
+            {
+                return Err(SnapshotCaptureError::source_changed(format!(
+                    "material file changed during capture: {relative}"
+                )));
+            }
+        }
+        let final_absent = captured
+            .iter()
+            .flat_map(|(_, manifest, _)| manifest.entries().iter())
+            .filter_map(|(path, entry)| {
+                matches!(entry, ManifestEntry::AbsentOptional(_)).then_some(path.clone())
+            })
+            .collect::<BTreeSet<_>>();
+        for relative in final_absent {
+            budget.check_deadline(self.clock.as_ref())?;
+            let path = self.workspace.join(&relative);
+            match std::fs::symlink_metadata(&path) {
+                Ok(_) => {
+                    return Err(SnapshotCaptureError::source_changed(format!(
+                        "optional material appeared during capture: {relative}"
+                    )));
+                }
+                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+                    reject_existing_ancestor_links(&self.workspace, &path).map_err(|detail| {
+                        if detail.starts_with("symlink_or_reparse_escape:") {
+                            SnapshotCaptureError::source_changed(format!(
+                                "optional material topology changed during capture: {relative}"
+                            ))
+                        } else {
+                            SnapshotCaptureError::classify(detail)
+                        }
+                    })?;
+                }
+                Err(error) => {
+                    return Err(SnapshotCaptureError::classify(format!(
+                        "material_file_unavailable: {}: {error}",
+                        path.display()
+                    )));
+                }
+            }
+        }
+
+        let after =
+            resolve_source_selection(&self.workspace, Some(&analysis.name), &mutation_names)
+                .map_err(classify_mapping_revalidation_error)?;
+        if before != after {
+            return Err(SnapshotCaptureError::source_changed(
+                "source mapping changed during snapshot capture",
+            ));
+        }
+        budget.check_deadline(self.clock.as_ref())?;
+
+        let mut snapshots = captured
+            .into_iter()
+            .map(|(source, manifest, _)| SourceSetSnapshot::from_manifest(source, manifest))
+            .collect::<Result<Vec<_>, _>>()?;
+        let analysis_snapshot = snapshots.remove(0);
+        Ok(SourceSnapshot::new(
+            analysis_snapshot,
+            snapshots,
+            workspace_epoch,
+        )?)
+    }
+
+    fn verified_read(
+        &self,
+        snapshot: &SourceSetSnapshot,
+        workspace_relative_path: &str,
+        optional: bool,
+    ) -> Result<Option<Vec<u8>>, SourceReadError> {
+        // Manifest membership is intentionally checked before touching the filesystem.
+        let Some(entry) = snapshot.manifest.get(workspace_relative_path) else {
+            return Err(SourceReadError::NotInManifest {
+                path: workspace_relative_path.into(),
+            });
+        };
+        if !path_belongs_to_source(&snapshot.source_set.relative_root, workspace_relative_path) {
+            return Err(SourceReadError::NotInManifest {
+                path: workspace_relative_path.into(),
+            });
+        }
+        if let Err(detail) = snapshot.validate() {
+            return Err(SourceReadError::SnapshotUnavailable {
+                path: workspace_relative_path.into(),
+                detail,
+            });
+        }
+        match entry {
+            ManifestEntry::AbsentOptional(_) => {
+                if !optional {
+                    return Err(SourceReadError::NotInManifest {
+                        path: workspace_relative_path.into(),
+                    });
+                }
+                match std::fs::symlink_metadata(self.workspace.join(workspace_relative_path)) {
+                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+                        reject_existing_ancestor_links(
+                            &self.workspace,
+                            &self.workspace.join(workspace_relative_path),
+                        )
+                        .map_err(|detail| {
+                            if detail.starts_with("symlink_or_reparse_escape:") {
+                                SourceReadError::SourceFingerprintMismatch {
+                                    path: workspace_relative_path.into(),
+                                }
+                            } else {
+                                SourceReadError::SnapshotUnavailable {
+                                    path: workspace_relative_path.into(),
+                                    detail,
+                                }
+                            }
+                        })?;
+                        Ok(None)
+                    }
+                    Ok(_) => Err(SourceReadError::SourceFingerprintMismatch {
+                        path: workspace_relative_path.into(),
+                    }),
+                    Err(error) => Err(SourceReadError::SnapshotUnavailable {
+                        path: workspace_relative_path.into(),
+                        detail: format!(
+                            "material_file_unavailable: {workspace_relative_path}: {error}"
+                        ),
+                    }),
+                }
+            }
+            ManifestEntry::Present(expected) => {
+                let path = self.workspace.join(workspace_relative_path);
+                match std::fs::symlink_metadata(&path) {
+                    Ok(metadata)
+                        if !metadata_is_link_or_reparse_point(&metadata)
+                            && metadata.is_file()
+                            && metadata.len() == expected.byte_length => {}
+                    Ok(_) => {
+                        return Err(SourceReadError::SourceFingerprintMismatch {
+                            path: workspace_relative_path.into(),
+                        });
+                    }
+                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+                        return Err(SourceReadError::SourceFingerprintMismatch {
+                            path: workspace_relative_path.into(),
+                        });
+                    }
+                    Err(error) => {
+                        return Err(SourceReadError::SnapshotUnavailable {
+                            path: workspace_relative_path.into(),
+                            detail: format!(
+                                "material_file_unavailable: {workspace_relative_path}: {error}"
+                            ),
+                        });
+                    }
+                }
+                let read = read_stable_bytes(&self.workspace, &path, expected.byte_length, None)
+                    .map_err(|detail| {
+                        if detail.starts_with("source_snapshot_byte_limit:")
+                            || detail.starts_with("source_snapshot_unavailable:")
+                        {
+                            SourceReadError::SourceFingerprintMismatch {
+                                path: workspace_relative_path.into(),
+                            }
+                        } else {
+                            SourceReadError::SnapshotUnavailable {
+                                path: workspace_relative_path.into(),
+                                detail,
+                            }
+                        }
+                    })?;
+                if read.bytes.len() as u64 != expected.byte_length
+                    || digest_bytes(&read.bytes) != expected.content_digest
+                {
+                    return Err(SourceReadError::SourceFingerprintMismatch {
+                        path: workspace_relative_path.into(),
+                    });
+                }
+                Ok(Some(read.bytes))
+            }
+        }
+    }
+}
+
+impl SourceSnapshotPort for FilesystemSourceSnapshots {
+    fn capture(
+        &self,
+        analysis: &ResolvedSourceSet,
+        mutation_sources: &[ResolvedSourceSet],
+        workspace_epoch: u64,
+    ) -> Result<SourceSnapshot, SnapshotCaptureError> {
+        self.capture_authoritative(analysis, mutation_sources, workspace_epoch)
+    }
+
+    fn read_verified(
+        &self,
+        snapshot: &SourceSetSnapshot,
+        workspace_relative_path: &str,
+    ) -> Result<Vec<u8>, SourceReadError> {
+        self.verified_read(snapshot, workspace_relative_path, false)?
+            .ok_or_else(|| SourceReadError::NotInManifest {
+                path: workspace_relative_path.into(),
+            })
+    }
+
+    fn read_optional_verified(
+        &self,
+        snapshot: &SourceSetSnapshot,
+        workspace_relative_path: &str,
+    ) -> Result<Option<Vec<u8>>, SourceReadError> {
+        self.verified_read(snapshot, workspace_relative_path, true)
+    }
+}
+
+#[derive(Debug, Clone, PartialEq, Eq)]
+struct SourcePlan {
+    source_set: ResolvedSourceSet,
+    present: BTreeSet<String>,
+    absent_optional: BTreeMap<String, OptionalMaterialTag>,
+}
+
+fn scan_source_plan(
+    workspace: &Path,
+    source: &ResolvedSourceSet,
+    budget: &mut CaptureBudget,
+    clock: &dyn SnapshotClock,
+) -> Result<SourcePlan, String> {
+    source.validate()?;
+    let root = resolve_contained_directory(workspace, &source.relative_root)?;
+    match source.source_format {
+        SourceFormat::PlatformXml => {
+            scan_platform_xml_plan(workspace, &root, source, budget, clock)
+        }
+        SourceFormat::Edt => scan_edt_diagnostic_plan(workspace, &root, source, budget, clock),
+        SourceFormat::Unknown | SourceFormat::Invalid => Err(format!(
+            "unsupported_source_format: unsupported source format {:?}",
+            source.source_format
+        )),
+    }
+}
+
+fn classify_final_scan_error(detail: String) -> SnapshotCaptureError {
+    let classified = SnapshotCaptureError::classify(detail);
+    match classified.reason {
+        SnapshotCaptureReason::MalformedSourceMaterial
+        | SnapshotCaptureReason::UnsupportedSourceLayout
+        | SnapshotCaptureReason::InvalidSourcePath => {
+            SnapshotCaptureError::source_changed(classified.detail)
+        }
+        SnapshotCaptureReason::UnsafeSourceTopology
+            if !classified.detail.starts_with("file_identity_unavailable:") =>
+        {
+            SnapshotCaptureError::source_changed(classified.detail)
+        }
+        _ => classified,
+    }
+}
+
+fn classify_mapping_revalidation_error(detail: String) -> SnapshotCaptureError {
+    let classified = SnapshotCaptureError::classify(detail);
+    match classified.reason {
+        SnapshotCaptureReason::TransientSourceIo
+        | SnapshotCaptureReason::SnapshotDeadlineExceeded
+        | SnapshotCaptureReason::SnapshotResourceLimit => classified,
+        SnapshotCaptureReason::UnsafeSourceTopology
+            if classified.detail.starts_with("file_identity_unavailable:") =>
+        {
+            classified
+        }
+        _ => SnapshotCaptureError::source_changed(classified.detail),
+    }
+}
+
+fn classify_final_present_revalidation_error(detail: String) -> SnapshotCaptureError {
+    if detail.starts_with("source_snapshot_byte_limit:") {
+        SnapshotCaptureError::source_changed(detail)
+    } else {
+        SnapshotCaptureError::classify(detail)
+    }
+}
+
+fn scan_platform_xml_plan(
+    workspace: &Path,
+    root: &Path,
+    source: &ResolvedSourceSet,
+    budget: &mut CaptureBudget,
+    clock: &dyn SnapshotClock,
+) -> Result<SourcePlan, String> {
+    let mut present = BTreeSet::new();
+    let configuration_path = root.join("Configuration.xml");
+    let configuration = read_stable_bytes(
+        workspace,
+        &configuration_path,
+        budget.limits.max_xml_bytes,
+        None,
+    )?;
+    let configuration_relative = slash_relative(workspace, &configuration_path)?;
+    present.insert(configuration_relative);
+    let registrations = parse_configuration_registrations(&configuration.bytes)?;
+    collect_exact_ext_files(
+        workspace,
+        &root.join("Ext"),
+        SOURCE_ROOT_EXT_ARTIFACTS_V1,
+        &mut present,
+        budget,
+        clock,
+    )?;
+    for registration in registrations {
+        budget.check_deadline(clock)?;
+        let descriptor = root
+            .join(&registration.directory)
+            .join(format!("{}.xml", registration.name));
+        require_regular_material(workspace, &descriptor)?;
+        let descriptor_bytes =
+            read_stable_bytes(workspace, &descriptor, budget.limits.max_xml_bytes, None)?;
+        present.insert(slash_relative(workspace, &descriptor)?);
+        let nested = parse_registered_descriptor(&descriptor_bytes.bytes, &registration)?;
+        collect_registered_subtree(
+            workspace,
+            &root
+                .join(&registration.directory)
+                .join(&registration.name)
+                .join("Ext"),
+            &mut present,
+            budget,
+            clock,
+        )?;
+        for (collection, names) in [
+            ("Forms", nested.forms),
+            ("Templates", nested.templates),
+            ("Commands", nested.commands),
+        ] {
+            for name in names {
+                let nested_descriptor = root
+                    .join(&registration.directory)
+                    .join(&registration.name)
+                    .join(collection)
+                    .join(format!("{name}.xml"));
+                require_regular_material(workspace, &nested_descriptor)?;
+                present.insert(slash_relative(workspace, &nested_descriptor)?);
+                collect_registered_subtree(
+                    workspace,
+                    &root
+                        .join(&registration.directory)
+                        .join(&registration.name)
+                        .join(collection)
+                        .join(&name)
+                        .join("Ext"),
+                    &mut present,
+                    budget,
+                    clock,
+                )?;
+            }
+        }
+    }
+
+    let parent_configurations = root.join(OPTIONAL_PARENT_CONFIGURATIONS);
+    let parent_relative = slash_relative(workspace, &parent_configurations)?;
+    let mut absent_optional = BTreeMap::new();
+    match std::fs::symlink_metadata(&parent_configurations) {
+        Ok(metadata) => {
+            if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+                return Err(format!(
+                    "material_file_not_regular: {}",
+                    parent_configurations.display()
+                ));
+            }
+            present.insert(parent_relative);
+        }
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+            reject_existing_ancestor_links(workspace, &parent_configurations)?;
+            absent_optional.insert(parent_relative, OptionalMaterialTag::ParentConfigurations);
+        }
+        Err(error) => {
+            return Err(format!(
+                "material_file_unavailable: {}: {error}",
+                parent_configurations.display()
+            ));
+        }
+    }
+    Ok(SourcePlan {
+        source_set: source.clone(),
+        present,
+        absent_optional,
+    })
+}
+
+fn scan_edt_diagnostic_plan(
+    workspace: &Path,
+    root: &Path,
+    source: &ResolvedSourceSet,
+    budget: &mut CaptureBudget,
+    clock: &dyn SnapshotClock,
+) -> Result<SourcePlan, String> {
+    let mut present = BTreeSet::new();
+    let mut absent_optional = BTreeMap::new();
+    for (relative, tag) in [
+        (
+            EDT_DIAGNOSTIC_MARKERS_V1[0],
+            OptionalMaterialTag::EdtProject,
+        ),
+        (
+            EDT_DIAGNOSTIC_MARKERS_V1[1],
+            OptionalMaterialTag::EdtProjectPmf,
+        ),
+        (
+            EDT_DIAGNOSTIC_MARKERS_V1[2],
+            OptionalMaterialTag::EdtConfigurationMdo,
+        ),
+        (
+            EDT_DIAGNOSTIC_MARKERS_V1[3],
+            OptionalMaterialTag::EdtSourceConfigurationMdo,
+        ),
+    ] {
+        budget.check_deadline(clock)?;
+        let path = root.join(relative);
+        match std::fs::symlink_metadata(&path) {
+            Ok(metadata) => {
+                if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+                    return Err(format!("material_file_not_regular: {}", path.display()));
+                }
+                present.insert(slash_relative(workspace, &path)?);
+            }
+            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+                reject_existing_ancestor_links(workspace, &path)?;
+                absent_optional.insert(slash_relative(workspace, &path)?, tag);
+            }
+            Err(error) => {
+                return Err(format!(
+                    "material_file_unavailable: {}: {error}",
+                    path.display()
+                ))
+            }
+        }
+    }
+    if present.is_empty() {
+        return Err("source_snapshot_unavailable: EDT diagnostic markers disappeared".into());
+    }
+    Ok(SourcePlan {
+        source_set: source.clone(),
+        present,
+        absent_optional,
+    })
+}
+
+fn collect_exact_ext_files(
+    workspace: &Path,
+    base: &Path,
+    allowed: &[&str],
+    present: &mut BTreeSet<String>,
+    budget: &mut CaptureBudget,
+    clock: &dyn SnapshotClock,
+) -> Result<(), String> {
+    let metadata = match std::fs::symlink_metadata(base) {
+        Ok(metadata) => metadata,
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
+        Err(error) => {
+            return Err(format!(
+                "material_subtree_unavailable: {}: {error}",
+                base.display()
+            ))
+        }
+    };
+    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
+        return Err(format!(
+            "material_subtree_not_directory: {}",
+            base.display()
+        ));
+    }
+    reject_link_components(workspace, base)?;
+    for relative in allowed {
+        budget.check_deadline(clock)?;
+        budget.register_traversal_entry()?;
+        let path = base.join(relative);
+        match std::fs::symlink_metadata(&path) {
+            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
+                return Err(format!("material_file_not_regular: {}", path.display()));
+            }
+            Ok(_) => {
+                present.insert(slash_relative(workspace, &path)?);
+            }
+            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
+            Err(error) => {
+                return Err(format!(
+                    "material_file_unavailable: {}: {error}",
+                    path.display()
+                ))
+            }
+        }
+    }
+    Ok(())
+}
+
+fn collect_registered_subtree(
+    workspace: &Path,
+    base: &Path,
+    present: &mut BTreeSet<String>,
+    budget: &mut CaptureBudget,
+    clock: &dyn SnapshotClock,
+) -> Result<(), String> {
+    let metadata = match std::fs::symlink_metadata(base) {
+        Ok(metadata) => metadata,
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
+        Err(error) => {
+            return Err(format!(
+                "material_subtree_unavailable: {}: {error}",
+                base.display()
+            ))
+        }
+    };
+    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
+        return Err(format!(
+            "material_subtree_not_directory: {}",
+            base.display()
+        ));
+    }
+    reject_link_components(workspace, base)?;
+    let mut pending = vec![(base.to_path_buf(), 0usize)];
+    while let Some((directory, depth)) = pending.pop() {
+        if depth > budget.limits.max_traversal_depth {
+            return Err("source_snapshot_traversal_depth: authoritative snapshot discarded".into());
+        }
+        budget.check_deadline(clock)?;
+        let entries = std::fs::read_dir(&directory).map_err(|error| {
+            format!(
+                "material_subtree_unreadable: {}: {error}",
+                directory.display()
+            )
+        })?;
+        let mut entries = entries.collect::<Result<Vec<_>, _>>().map_err(|error| {
+            format!(
+                "material_subtree_unreadable: {}: {error}",
+                directory.display()
+            )
+        })?;
+        entries.sort_by_key(|entry| entry.file_name());
+        let mut child_directories = Vec::new();
+        for entry in entries {
+            budget.register_traversal_entry()?;
+            let path = entry.path();
+            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
+                format!("material_file_unavailable: {}: {error}", path.display())
+            })?;
+            if metadata_is_link_or_reparse_point(&metadata) {
+                return Err(format!("symlink_or_reparse_escape: {}", path.display()));
+            }
+            if metadata.is_dir() {
+                let ignored = entry
+                    .file_name()
+                    .to_str()
+                    .is_some_and(|name| IGNORED_REGISTERED_SUBTREE_DIRECTORIES.contains(&name));
+                if !ignored {
+                    let next_depth = depth
+                        .checked_add(1)
+                        .ok_or_else(|| "source_snapshot_traversal_depth: overflow".to_string())?;
+                    child_directories.push((path, next_depth));
+                }
+            } else if metadata.is_file() {
+                present.insert(slash_relative(workspace, &path)?);
+            } else {
+                return Err(format!("material_file_not_regular: {}", path.display()));
+            }
+        }
+        pending.extend(child_directories.into_iter().rev());
+    }
+    Ok(())
+}
+
+fn require_regular_material(workspace: &Path, path: &Path) -> Result<(), String> {
+    let parent = path
+        .parent()
+        .ok_or_else(|| format!("invalid_material_path: {}", path.display()))?;
+    reject_link_components(workspace, parent)?;
+    let metadata = std::fs::symlink_metadata(path)
+        .map_err(|error| format!("registered_material_missing: {}: {error}", path.display()))?;
+    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+        return Err(format!("material_file_not_regular: {}", path.display()));
+    }
+    Ok(())
+}
+
+fn reject_existing_ancestor_links(workspace: &Path, path: &Path) -> Result<(), String> {
+    let relative = path
+        .strip_prefix(workspace)
+        .map_err(|_| format!("path_escape: {}", path.display()))?;
+    let mut current = workspace.to_path_buf();
+    for component in relative.components() {
+        current.push(component.as_os_str());
+        match std::fs::symlink_metadata(&current) {
+            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
+                return Err(format!("symlink_or_reparse_escape: {}", current.display()));
+            }
+            Ok(_) => {}
+            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
+            Err(error) => return Err(format!("path_unavailable: {}: {error}", current.display())),
+        }
+    }
+    Ok(())
+}
+
+struct StableRead {
+    bytes: Vec<u8>,
+    observation: FileObservation,
+}
+
+fn read_stable_bytes(
+    workspace: &Path,
+    path: &Path,
+    max_bytes: u64,
+    hook: Option<&dyn CaptureHook>,
+) -> Result<StableRead, String> {
+    reject_link_components(workspace, path)?;
+    let before_path_metadata = std::fs::symlink_metadata(path)
+        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
+    if metadata_is_link_or_reparse_point(&before_path_metadata) || !before_path_metadata.is_file() {
+        return Err(format!("material_file_not_regular: {}", path.display()));
+    }
+    #[cfg(unix)]
+    let before = observe_regular_file(&before_path_metadata, path)?;
+    let before_length = before_path_metadata.len();
+    if before_length > max_bytes {
+        return Err(format!("source_snapshot_byte_limit: {}", path.display()));
+    }
+    #[cfg(unix)]
+    if let Some(hook) = hook {
+        hook.on_event(&CaptureEvent::BeforeContainedOpen(slash_relative(
+            workspace, path,
+        )?));
+    }
+    let mut contained = open_no_follow(workspace, path).map_err(|detail| {
+        classify_open_failure_after_observation(
+            workspace,
+            path,
+            &before_path_metadata,
+            #[cfg(unix)]
+            &before,
+            detail,
+        )
+    })?;
+    let opened = observe_open_file(contained.file(), path).map_err(|detail| {
+        if detail.starts_with("material_file_not_regular:") {
+            format!(
+                "source_snapshot_unavailable: opened material type changed after observation: {}",
+                path.display()
+            )
+        } else {
+            detail
+        }
+    })?;
+    if let Some(hook) = hook {
+        hook.on_event(&CaptureEvent::ContainedOpenEstablished(slash_relative(
+            workspace, path,
+        )?));
+    }
+    #[cfg(unix)]
+    if before != opened {
+        return Err(format!(
+            "source_snapshot_unavailable: concurrent replacement: {}",
+            path.display()
+        ));
+    }
+    #[cfg(windows)]
+    if before_length != opened.length {
+        return Err(format!(
+            "source_snapshot_unavailable: concurrent replacement: {}",
+            path.display()
+        ));
+    }
+    let capacity = usize::try_from(before_length.min(64 * 1024))
+        .map_err(|_| format!("source_snapshot_byte_limit: {}", path.display()))?;
+    let mut bytes = Vec::with_capacity(capacity);
+    let read_limit = before_length
+        .checked_add(1)
+        .ok_or_else(|| format!("source_snapshot_byte_limit: {}", path.display()))?;
+    contained
+        .file_mut()
+        .take(read_limit)
+        .read_to_end(&mut bytes)
+        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
+    if bytes.len() as u64 > before_length {
+        return Err(format!(
+            "source_snapshot_unavailable: material grew during bounded read: {}",
+            path.display()
+        ));
+    }
+    let after_handle = observe_open_file(contained.file(), path)?;
+    contained.validate_after_read()?;
+    #[cfg(unix)]
+    let baseline = before;
+    #[cfg(windows)]
+    let baseline = opened;
+    #[cfg(unix)]
+    let after_path = observe_path_after_read(path)?;
+    #[cfg(windows)]
+    let after_path = {
+        let reopened = open_no_follow(workspace, path).map_err(|detail| {
+            classify_reopen_failure_after_observation(workspace, path, baseline.length, detail)
+        })?;
+        let observation = observe_open_file(reopened.file(), path)?;
+        reopened.validate_after_read()?;
+        observation
+    };
+    if baseline != after_handle || baseline != after_path || bytes.len() as u64 != before_length {
+        return Err(format!(
+            "source_snapshot_unavailable: concurrent mutation: {}",
+            path.display()
+        ));
+    }
+    Ok(StableRead {
+        bytes,
+        observation: baseline,
+    })
+}
+
+fn classify_open_failure_after_observation(
+    workspace: &Path,
+    path: &Path,
+    before_metadata: &std::fs::Metadata,
+    #[cfg(unix)] before: &FileObservation,
+    detail: String,
+) -> String {
+    if let Err(ancestor_detail) = reject_existing_ancestor_links(workspace, path) {
+        if ancestor_detail.starts_with("symlink_or_reparse_escape:") {
+            return format!(
+                "source_snapshot_unavailable: contained source topology changed before open: {}",
+                path.display()
+            );
+        }
+        return detail;
+    }
+    match std::fs::symlink_metadata(path) {
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => format!(
+            "source_snapshot_unavailable: material disappeared before open: {}",
+            path.display()
+        ),
+        Ok(metadata)
+            if metadata_is_link_or_reparse_point(&metadata)
+                || !metadata.is_file()
+                || metadata.len() != before_metadata.len() =>
+        {
+            format!(
+                "source_snapshot_unavailable: material type or length changed before open: {}",
+                path.display()
+            )
+        }
+        #[cfg(unix)]
+        Ok(metadata) => match observe_regular_file(&metadata, path) {
+            Ok(after) if &after != before => format!(
+                "source_snapshot_unavailable: material identity changed before open: {}",
+                path.display()
+            ),
+            Ok(_) | Err(_) => detail,
+        },
+        #[cfg(not(unix))]
+        Ok(_) => detail,
+        Err(_) => detail,
+    }
+}
+
+#[cfg(any(windows, test))]
+fn classify_reopen_failure_after_observation(
+    workspace: &Path,
+    path: &Path,
+    expected_length: u64,
+    detail: String,
+) -> String {
+    if let Err(ancestor_detail) = reject_existing_ancestor_links(workspace, path) {
+        if ancestor_detail.starts_with("symlink_or_reparse_escape:") {
+            return format!(
+                "source_snapshot_unavailable: contained source topology changed after open: {}",
+                path.display()
+            );
+        }
+        return detail;
+    }
+    match std::fs::symlink_metadata(path) {
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => format!(
+            "source_snapshot_unavailable: material disappeared after open: {}",
+            path.display()
+        ),
+        Ok(metadata)
+            if metadata_is_link_or_reparse_point(&metadata)
+                || !metadata.is_file()
+                || metadata.len() != expected_length =>
+        {
+            format!(
+                "source_snapshot_unavailable: material type or length changed after open: {}",
+                path.display()
+            )
+        }
+        Ok(_) | Err(_) => detail,
+    }
+}
+
+#[cfg(unix)]
+fn observe_path_after_read(path: &Path) -> Result<FileObservation, String> {
+    let metadata = match std::fs::symlink_metadata(path) {
+        Ok(metadata) => metadata,
+        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
+            return Err(format!(
+                "source_snapshot_unavailable: material disappeared after open: {}",
+                path.display()
+            ));
+        }
+        Err(error) => {
+            return Err(format!(
+                "material_file_unavailable: {}: {error}",
+                path.display()
+            ));
+        }
+    };
+    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
+        return Err(format!(
+            "source_snapshot_unavailable: material type changed after open: {}",
+            path.display()
+        ));
+    }
+    observe_regular_file(&metadata, path)
+}
+
+fn digest_bytes(bytes: &[u8]) -> String {
+    format!("sha256:{:x}", Sha256::digest(bytes))
+}
+
+fn path_belongs_to_source(root: &str, path: &str) -> bool {
+    root == "."
+        || path
+            .strip_prefix(root)
+            .is_some_and(|rest| rest.starts_with('/'))
+}
+
+#[derive(Debug, Clone)]
+struct CaptureBudget {
+    limits: SnapshotLimits,
+    started: Duration,
+    files: usize,
+    bytes: u64,
+    traversal_entries: usize,
+}
+
+impl CaptureBudget {
+    fn new(limits: SnapshotLimits, started: Duration) -> Self {
+        Self {
+            limits,
+            started,
+            files: 0,
+            bytes: 0,
+            traversal_entries: 0,
+        }
+    }
+
+    fn check_deadline(&self, clock: &dyn SnapshotClock) -> Result<(), String> {
+        let elapsed = clock
+            .now()
+            .checked_sub(self.started)
+            .ok_or_else(|| "source_snapshot_clock_invalid: clock moved backwards".to_string())?;
+        if elapsed > self.limits.max_elapsed {
+            return Err("source_snapshot_deadline: authoritative snapshot discarded".into());
+        }
+        Ok(())
+    }
+
+    fn register_files(&mut self, count: usize) -> Result<(), String> {
+        self.files = self
+            .files
+            .checked_add(count)
+            .ok_or_else(|| "source_snapshot_file_limit: overflow".to_string())?;
+        if self.files > self.limits.max_files {
+            return Err("source_snapshot_file_limit: authoritative snapshot discarded".into());
+        }
+        Ok(())
+    }
+
+    fn register_bytes(&mut self, count: u64) -> Result<(), String> {
+        self.bytes = self
+            .bytes
+            .checked_add(count)
+            .ok_or_else(|| "source_snapshot_byte_limit: overflow".to_string())?;
+        if self.bytes > self.limits.max_bytes {
+            return Err("source_snapshot_byte_limit: authoritative snapshot discarded".into());
+        }
+        Ok(())
+    }
+
+    fn remaining_bytes(&self) -> Result<u64, String> {
+        self.limits
+            .max_bytes
+            .checked_sub(self.bytes)
+            .ok_or_else(|| "source_snapshot_byte_limit: aggregate overflow".to_string())
+    }
+
+    fn register_traversal_entry(&mut self) -> Result<(), String> {
+        self.traversal_entries = self
+            .traversal_entries
+            .checked_add(1)
+            .ok_or_else(|| "source_snapshot_traversal_limit: overflow".to_string())?;
+        if self.traversal_entries > self.limits.max_traversal_entries {
+            return Err("source_snapshot_traversal_limit: authoritative snapshot discarded".into());
+        }
+        Ok(())
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use std::fs;
+    use std::sync::atomic::{AtomicU64, Ordering};
+    use std::time::{SystemTime, UNIX_EPOCH};
+
+    #[test]
+    fn content_change_with_unchanged_len_and_mtime_changes_fingerprint() {
+        let fixture = Fixture::new("snapshot-content");
+        let (resolver, service) = fixture.services();
+        let source = resolver.analysis;
+        let before = service.capture_authoritative(&source, &[], 1).unwrap();
+        let module = fixture.root.join("main/CommonModules/X/Ext/Module.bsl");
+        #[cfg(unix)]
+        let timestamps = unix_timestamps(&module);
+        fs::write(&module, "BBBB").unwrap();
+        #[cfg(unix)]
+        restore_unix_timestamps(&module, timestamps);
+        let after = service.capture_authoritative(&source, &[], 1).unwrap();
+        assert_ne!(
+            before.analysis.source_fingerprint,
+            after.analysis.source_fingerprint
+        );
+    }
+
+    #[test]
+    fn platform_manifest_is_registration_aware_deterministic_and_excludes_generated_corpora() {
+        let fixture = Fixture::new("snapshot-selection");
+        write(
+            &fixture.root.join("main/Configuration.xml"),
+            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule><Role>Admin</Role><Document>Sale</Document></ChildObjects></Configuration></MetaDataObject>",
+        );
+        write(
+            &fixture.root.join("main/Roles/Admin.xml"),
+            "<MetaDataObject><Role><Properties><Name>Admin</Name></Properties><ChildObjects/></Role></MetaDataObject>",
+        );
+        write(
+            &fixture.root.join("main/Roles/Admin/Ext/Rights.xml"),
+            "rights",
+        );
+        write(
+            &fixture.root.join("main/Documents/Sale.xml"),
+            "<MetaDataObject><Document><Properties><Name>Sale</Name></Properties><ChildObjects><Command>Post</Command></ChildObjects></Document></MetaDataObject>",
+        );
+        write(
+            &fixture.root.join("main/Documents/Sale/Commands/Post.xml"),
+            "registered command",
+        );
+        write(
+            &fixture
+                .root
+                .join("main/Documents/Sale/Commands/Post/Ext/Module.bsl"),
+            "command",
+        );
+        write(
+            &fixture.root.join("main/Documents/Sale/Commands/Decoy.xml"),
+            "decoy command",
+        );
+        write(
+            &fixture
+                .root
+                .join("main/Documents/Sale/Commands/Decoy/Ext/Module.bsl"),
+            "decoy",
+        );
+        write(&fixture.root.join("main/CommonModules/Decoy.xml"), "decoy");
+        write(
+            &fixture.root.join("main/docs/research/secret.bsl"),
+            "secret",
+        );
+        write(
+            &fixture
+                .root
+                .join("main/CommonModules/X/Ext/target/generated.bin"),
+            "generated",
+        );
+        write(&fixture.root.join("main/Ext/Decoy.bsl"), "decoy");
+        write(&fixture.root.join("main/Ext/SessionModule.bsl"), "session");
+        let (selection, service) = fixture.services();
+        let first = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let paths = first
+            .analysis
+            .manifest
+            .entries()
+            .keys()
+            .cloned()
+            .collect::<Vec<_>>();
+        assert_eq!(paths, {
+            let mut sorted = paths.clone();
+            sorted.sort();
+            sorted
+        });
+        assert!(paths.iter().any(|path| path.ends_with("Configuration.xml")));
+        assert!(paths
+            .iter()
+            .any(|path| path.ends_with("CommonModules/X.xml")));
+        assert!(paths
+            .iter()
+            .any(|path| path.ends_with("CommonModules/X/Ext/Module.bsl")));
+        assert!(paths
+            .iter()
+            .any(|path| path.ends_with("Ext/SessionModule.bsl")));
+        assert!(paths
+            .iter()
+            .any(|path| path.ends_with("Roles/Admin/Ext/Rights.xml")));
+        assert!(paths
+            .iter()
+            .any(|path| path.ends_with("Commands/Post/Ext/Module.bsl")));
+        assert!(!paths.iter().any(|path| path.contains("Decoy")
+            || path.contains("docs/research")
+            || path.contains("target")));
+        write(
+            &fixture.root.join("main/CommonModules/Decoy.xml"),
+            "changed",
+        );
+        let second = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        assert_eq!(
+            first.analysis.source_fingerprint,
+            second.analysis.source_fingerprint
+        );
+    }
+
+    #[test]
+    fn composite_capture_sorts_and_deduplicates_destinations_and_binds_each_role() {
+        let fixture = Fixture::new("snapshot-composite");
+        fixture.add_extension("ExtensionA", "ext-a", "A");
+        fixture.add_extension("ExtensionB", "ext-b", "B");
+        let selection = resolve_source_selection(
+            &fixture.root,
+            Some("main"),
+            &[
+                "ExtensionB".into(),
+                "ExtensionA".into(),
+                "ExtensionB".into(),
+            ],
+        )
+        .unwrap();
+        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
+        let snapshot = service
+            .capture_authoritative(&selection.analysis, &selection.mutations, 7)
+            .unwrap();
+        assert_eq!(snapshot.mutations.len(), 2);
+        assert_eq!(snapshot.mutations[0].source_set.name, "ExtensionA");
+        let only_a =
+            resolve_source_selection(&fixture.root, Some("main"), &["ExtensionA".into()]).unwrap();
+        let a = service
+            .capture_authoritative(&only_a.analysis, &only_a.mutations, 7)
+            .unwrap();
+        assert_ne!(snapshot.composite_fingerprint, a.composite_fingerprint);
+    }
+
+    #[test]
+    fn optional_parent_configurations_absence_and_presence_are_snapshot_bound() {
+        let fixture = Fixture::new("snapshot-parent-config");
+        let (selection, service) = fixture.services();
+        let absent = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let path = "main/Ext/ParentConfigurations.bin";
+        assert_eq!(
+            service
+                .read_optional_verified(&absent.analysis, path)
+                .unwrap(),
+            None
+        );
+        write(&fixture.root.join(path), "parent");
+        let mismatch = service
+            .read_optional_verified(&absent.analysis, path)
+            .unwrap_err();
+        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
+        assert!(mismatch.retryable());
+        let present = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        assert_eq!(
+            service
+                .read_optional_verified(&present.analysis, path)
+                .unwrap(),
+            Some(b"parent".to_vec())
+        );
+        assert_ne!(
+            absent.analysis.source_fingerprint,
+            present.analysis.source_fingerprint
+        );
+    }
+
+    #[test]
+    fn authoritative_root_interfaces_change_source_fingerprint() {
+        for artifact in ["ClientApplicationInterface.xml", "HomePageWorkArea.xml"] {
+            let fixture = Fixture::new(&format!("snapshot-root-{}", artifact.replace('.', "-")));
+            let path = fixture.root.join("main/Ext").join(artifact);
+            write(&path, "AAAA");
+            let (selection, service) = fixture.services();
+            let before = service
+                .capture_authoritative(&selection.analysis, &[], 1)
+                .unwrap();
+            write(&path, "BBBB");
+            let after = service
+                .capture_authoritative(&selection.analysis, &[], 1)
+                .unwrap();
+            assert_ne!(
+                before.analysis.source_fingerprint, after.analysis.source_fingerprint,
+                "{artifact}"
+            );
+        }
+    }
+
+    #[test]
+    fn verified_read_checks_manifest_membership_then_detects_byte_mismatch() {
+        let fixture = Fixture::new("snapshot-verified-read");
+        let (selection, service) = fixture.services();
+        let snapshot = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let outside = service
+            .read_verified(&snapshot.analysis, "../outside")
+            .unwrap_err();
+        assert_eq!(outside.reason_code(), "source_path_not_in_manifest");
+        assert!(!outside.retryable());
+        let path = "main/CommonModules/X/Ext/Module.bsl";
+        fs::write(fixture.root.join(path), "BBBB").unwrap();
+        let mismatch = service.read_verified(&snapshot.analysis, path).unwrap_err();
+        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
+        assert!(mismatch.retryable());
+    }
+
+    #[test]
+    fn verified_read_rejects_oversized_replacement_as_fingerprint_mismatch() {
+        let fixture = Fixture::new("snapshot-verified-read-bounded");
+        let (selection, service) = fixture.services();
+        let snapshot = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let path = "main/CommonModules/X/Ext/Module.bsl";
+        let expected_length = match snapshot.analysis.manifest.get(path).unwrap() {
+            ManifestEntry::Present(file) => file.byte_length,
+            ManifestEntry::AbsentOptional(_) => unreachable!(),
+        };
+        fs::write(
+            fixture.root.join(path),
+            vec![b'X'; usize::try_from(expected_length + 1).unwrap()],
+        )
+        .unwrap();
+        let bounded = fixture.controlled_service(
+            SnapshotLimits {
+                max_bytes: expected_length,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+
+        let mismatch = bounded.read_verified(&snapshot.analysis, path).unwrap_err();
+        assert_eq!(mismatch.reason_code(), "source_fingerprint_mismatch");
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn absent_optional_read_rejects_symlinked_ancestor() {
+        use std::os::unix::fs::symlink;
+
+        let fixture = Fixture::new("snapshot-absent-symlink-ancestor");
+        let (selection, service) = fixture.services();
+        let snapshot = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let optional = "main/Ext/ParentConfigurations.bin";
+        assert!(matches!(
+            snapshot.analysis.manifest.get(optional),
+            Some(ManifestEntry::AbsentOptional(_))
+        ));
+        let ext = fixture.root.join("main/Ext");
+        let external = fixture.root.join("external-ext");
+        fs::create_dir_all(&external).unwrap();
+        symlink(&external, &ext).unwrap();
+
+        let error = service
+            .read_optional_verified(&snapshot.analysis, optional)
+            .unwrap_err();
+        assert_eq!(error.reason_code(), "source_fingerprint_mismatch");
+    }
+
+    #[test]
+    fn file_and_byte_limits_accept_boundary_and_reject_boundary_plus_one_globally() {
+        let fixture = Fixture::new("snapshot-bounds");
+        fixture.add_extension("ExtensionA", "ext-a", "A");
+        let selection =
+            resolve_source_selection(&fixture.root, Some("main"), &["ExtensionA".into()]).unwrap();
+        let baseline = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
+        let captured = baseline
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .unwrap();
+        let files = captured
+            .analysis
+            .manifest
+            .entries()
+            .values()
+            .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
+            .count()
+            + captured
+                .mutations
+                .iter()
+                .flat_map(|snapshot| snapshot.manifest.entries().values())
+                .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
+                .count();
+        let bytes = captured
+            .analysis
+            .manifest
+            .entries()
+            .values()
+            .chain(
+                captured
+                    .mutations
+                    .iter()
+                    .flat_map(|snapshot| snapshot.manifest.entries().values()),
+            )
+            .filter_map(|entry| match entry {
+                ManifestEntry::Present(file) => Some(file.byte_length),
+                ManifestEntry::AbsentOptional(_) => None,
+            })
+            .sum();
+        let exact = fixture.controlled_service(
+            SnapshotLimits {
+                max_files: files,
+                max_bytes: bytes,
+                max_elapsed: Duration::from_secs(60),
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        assert!(exact
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .is_ok());
+        let file_short = fixture.controlled_service(
+            SnapshotLimits {
+                max_files: files - 1,
+                max_bytes: bytes,
+                max_elapsed: Duration::from_secs(60),
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        let error = file_short
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
+        assert!(!error.retryable());
+        let byte_short = fixture.controlled_service(
+            SnapshotLimits {
+                max_files: files,
+                max_bytes: bytes - 1,
+                max_elapsed: Duration::from_secs(60),
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        let error = byte_short
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
+        assert!(!error.retryable());
+    }
+
+    #[test]
+    fn composite_budget_counts_overlapping_present_paths_once() {
+        let root = temp_root("snapshot-overlapping-composite-budget");
+        write(
+            &root.join("v8project.yaml"),
+            "source-set:\n - { name: main, type: CONFIGURATION, path: base }\n - { name: extension, type: EXTENSION, path: base/CommonModules/X }\n",
+        );
+        write(
+            &root.join("base/Configuration.xml"),
+            "<MetaDataObject><Configuration><ChildObjects><CommonModule>X</CommonModule></ChildObjects></Configuration></MetaDataObject>",
+        );
+        write(
+            &root.join("base/CommonModules/X.xml"),
+            "<MetaDataObject><CommonModule><Properties><Name>X</Name></Properties><ChildObjects/></CommonModule></MetaDataObject>",
+        );
+        write(
+            &root.join("base/CommonModules/X/Configuration.xml"),
+            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>",
+        );
+        write(&root.join("base/CommonModules/X/Ext/Module.bsl"), "shared");
+        let selection =
+            resolve_source_selection(&root, Some("main"), &["extension".into()]).unwrap();
+        let baseline = FilesystemSourceSnapshots::new(&root)
+            .unwrap()
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .unwrap();
+        let mut unique = BTreeMap::new();
+        for snapshot in std::iter::once(&baseline.analysis).chain(baseline.mutations.iter()) {
+            for (path, entry) in snapshot.manifest.entries() {
+                if let ManifestEntry::Present(material) = entry {
+                    unique.insert(path.clone(), material.byte_length);
+                }
+            }
+        }
+        let unique_bytes = unique.values().sum();
+        let exact = FilesystemSourceSnapshots {
+            workspace: canonical_workspace(&root).unwrap(),
+            limits: SnapshotLimits {
+                max_files: unique.len(),
+                max_bytes: unique_bytes,
+                ..SnapshotLimits::default()
+            },
+            clock: Arc::new(FixedClock),
+            hook: Arc::new(NoopCaptureHook),
+        };
+        assert!(exact
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .is_ok());
+
+        let short = FilesystemSourceSnapshots {
+            limits: SnapshotLimits {
+                max_files: unique.len() - 1,
+                max_bytes: unique_bytes,
+                ..SnapshotLimits::default()
+            },
+            ..exact
+        };
+        let error = short
+            .capture_authoritative(&selection.analysis, &selection.mutations, 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
+        assert!(!error.retryable());
+    }
+
+    #[test]
+    fn deadline_uses_injected_clock_and_discards_whole_snapshot() {
+        let fixture = Fixture::new("snapshot-deadline");
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let clock = Arc::new(AdvancingClock::default());
+        let service = fixture.controlled_service(
+            SnapshotLimits {
+                max_files: 100,
+                max_bytes: 1024 * 1024,
+                max_elapsed: Duration::from_millis(2),
+                ..SnapshotLimits::default()
+            },
+            clock,
+            Arc::new(NoopCaptureHook),
+        );
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SnapshotDeadlineExceeded
+        );
+        assert!(error.retryable());
+    }
+
+    #[test]
+    fn xml_and_traversal_bounds_accept_exact_boundary_and_reject_boundary_plus_one() {
+        let fixture = Fixture::new("snapshot-structural-bounds");
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let xml_max = [
+            fixture.root.join("main/Configuration.xml"),
+            fixture.root.join("main/CommonModules/X.xml"),
+        ]
+        .iter()
+        .map(|path| fs::metadata(path).unwrap().len())
+        .max()
+        .unwrap();
+        let exact_xml = fixture.controlled_service(
+            SnapshotLimits {
+                max_xml_bytes: xml_max,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        assert!(exact_xml
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .is_ok());
+        let short_xml = fixture.controlled_service(
+            SnapshotLimits {
+                max_xml_bytes: xml_max - 1,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        let error = short_xml
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
+        assert!(!error.retryable());
+
+        let clock = FixedClock;
+        let canonical_root = canonical_workspace(&fixture.root).unwrap();
+        let mut measured = CaptureBudget::new(SnapshotLimits::default(), Duration::ZERO);
+        scan_source_plan(&canonical_root, &selection.analysis, &mut measured, &clock).unwrap();
+        scan_source_plan(&canonical_root, &selection.analysis, &mut measured, &clock).unwrap();
+        let exact_count = measured.traversal_entries;
+        let exact_traversal = fixture.controlled_service(
+            SnapshotLimits {
+                max_traversal_entries: exact_count,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        assert!(exact_traversal
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .is_ok());
+        let short_traversal = fixture.controlled_service(
+            SnapshotLimits {
+                max_traversal_entries: exact_count - 1,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(NoopCaptureHook),
+        );
+        let error = short_traversal
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::SnapshotResourceLimit);
+        assert!(!error.retryable());
+    }
+
+    #[test]
+    fn concurrent_add_remove_write_and_replace_fail_the_whole_snapshot() {
+        let mut actions = vec![
+            RaceAction::Add,
+            RaceAction::Remove,
+            RaceAction::Write,
+            RaceAction::Replace,
+        ];
+        #[cfg(unix)]
+        actions.push(RaceAction::ParentSymlinkSwap);
+        for action in actions {
+            assert_retryable_race(action);
+        }
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn fifo_replacement_between_observation_and_open_is_nonblocking_and_retryable() {
+        assert_retryable_race(RaceAction::FifoSwapBeforeOpen);
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn parent_symlink_swap_between_observation_and_open_is_retryable() {
+        assert_retryable_race(RaceAction::ParentSymlinkSwapBeforeOpen);
+    }
+
+    #[test]
+    fn same_length_replacement_after_contained_open_is_retryable() {
+        assert_retryable_race(RaceAction::ReplaceAfterOpen);
+    }
+
+    #[test]
+    fn growth_after_contained_open_is_bounded_and_retryable() {
+        assert_retryable_race(RaceAction::GrowAfterOpen);
+    }
+
+    #[test]
+    fn absent_optional_appearance_after_final_scan_discards_snapshot() {
+        let fixture = Fixture::new("snapshot-optional-appears-before-final-validation");
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let service = fixture.controlled_service(
+            SnapshotLimits::default(),
+            Arc::new(FixedClock),
+            Arc::new(FinalValidationMutationHook::new(
+                fixture.root.clone(),
+                FinalValidationMutation::AddParentConfigurations,
+            )),
+        );
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SourceChangedDuringCapture
+        );
+        assert!(error.retryable());
+    }
+
+    #[test]
+    fn edt_absent_marker_appearance_after_final_scan_discards_snapshot() {
+        let root = temp_root("snapshot-edt-marker-appears-before-final-validation");
+        write(
+            &root.join("v8project.yaml"),
+            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
+        );
+        write(&root.join("edt/.project"), "project");
+        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
+        let service = FilesystemSourceSnapshots {
+            workspace: canonical_workspace(&root).unwrap(),
+            limits: SnapshotLimits::default(),
+            clock: Arc::new(FixedClock),
+            hook: Arc::new(FinalValidationMutationHook::new(
+                root,
+                FinalValidationMutation::AddEdtProjectPmf,
+            )),
+        };
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SourceChangedDuringCapture
+        );
+        assert!(error.retryable());
+    }
+
+    #[test]
+    fn final_present_reread_is_bounded_by_captured_length() {
+        let fixture = Fixture::new("snapshot-final-reread-bounded");
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let baseline = FilesystemSourceSnapshots::new(&fixture.root)
+            .unwrap()
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        let captured_bytes = baseline
+            .analysis
+            .manifest
+            .entries()
+            .values()
+            .filter_map(|entry| match entry {
+                ManifestEntry::Present(material) => Some(material.byte_length),
+                ManifestEntry::AbsentOptional(_) => None,
+            })
+            .sum();
+        let service = fixture.controlled_service(
+            SnapshotLimits {
+                max_bytes: captured_bytes,
+                ..SnapshotLimits::default()
+            },
+            Arc::new(FixedClock),
+            Arc::new(FinalValidationMutationHook::new(
+                fixture.root.clone(),
+                FinalValidationMutation::GrowPresent(captured_bytes + 1),
+            )),
+        );
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SourceChangedDuringCapture
+        );
+        assert!(error.retryable());
+    }
+
+    #[test]
+    fn malformed_registered_descriptor_is_stable_and_non_retryable() {
+        let fixture = Fixture::new("snapshot-malformed-descriptor");
+        write(
+            &fixture.root.join("main/CommonModules/X.xml"),
+            "<MetaDataObject><CommonModule>",
+        );
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::MalformedSourceMaterial);
+        assert!(!error.retryable());
+    }
+
+    #[test]
+    fn final_scan_and_identity_failures_preserve_their_classification() {
+        let io = classify_final_scan_error(
+            "material_subtree_unreadable: stable permission failure".into(),
+        );
+        assert_eq!(io.reason, SnapshotCaptureReason::TransientSourceIo);
+        assert!(io.retryable());
+
+        let identity = classify_final_scan_error(
+            "file_identity_unavailable: stable platform identity failure".into(),
+        );
+        assert_eq!(identity.reason, SnapshotCaptureReason::UnsafeSourceTopology);
+        assert!(!identity.retryable());
+    }
+
+    #[test]
+    fn post_open_reopen_failure_promotes_only_observed_change() {
+        let root = temp_root("snapshot-reopen-classifier");
+        let missing = root.join("missing.bsl");
+        let changed = classify_reopen_failure_after_observation(
+            &root,
+            &missing,
+            4,
+            "material_file_unreadable: reopen failed".into(),
+        );
+        assert!(changed.starts_with("source_snapshot_unavailable:"));
+
+        let stable = root.join("stable.bsl");
+        write(&stable, "same");
+        let io = classify_reopen_failure_after_observation(
+            &root,
+            &stable,
+            4,
+            "material_file_unreadable: stable share failure".into(),
+        );
+        assert_eq!(io, "material_file_unreadable: stable share failure");
+    }
+
+    #[test]
+    fn mapping_revalidation_promotes_structural_changes_but_preserves_io() {
+        let io = classify_mapping_revalidation_error(
+            "source_map_config_unavailable: stable share failure".into(),
+        );
+        assert_eq!(io.reason, SnapshotCaptureReason::TransientSourceIo);
+        assert!(io.retryable());
+
+        let mut actions = vec![
+            MappingMutation::RenameSource,
+            MappingMutation::DeleteMap,
+            MappingMutation::MalformedMap,
+        ];
+        #[cfg(unix)]
+        actions.push(MappingMutation::SymlinkMap);
+        for action in actions {
+            let fixture = Fixture::new(&format!("snapshot-mapping-race-{action:?}"));
+            let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+            let service = fixture.controlled_service(
+                SnapshotLimits::default(),
+                Arc::new(FixedClock),
+                Arc::new(MappingMutationHook::new(fixture.root.clone(), action)),
+            );
+
+            let error = service
+                .capture_authoritative(&selection.analysis, &[], 1)
+                .unwrap_err();
+            assert_eq!(
+                error.reason,
+                SnapshotCaptureReason::SourceChangedDuringCapture,
+                "{action:?}: {error:?}"
+            );
+            assert!(error.retryable());
+        }
+    }
+
+    #[test]
+    fn missing_registered_descriptor_is_stable_and_non_retryable() {
+        let fixture = Fixture::new("snapshot-missing-descriptor");
+        fs::remove_file(fixture.root.join("main/CommonModules/X.xml")).unwrap();
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::MalformedSourceMaterial);
+        assert!(!error.retryable());
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn non_utf8_material_path_is_rejected_non_retryably() {
+        use std::os::unix::ffi::OsStringExt;
+
+        let fixture = Fixture::new("snapshot-non-utf8-material");
+        let path = fixture
+            .root
+            .join("main/CommonModules/X/Ext")
+            .join(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
+        let detail = slash_relative(&fixture.root, &path).unwrap_err();
+        let error = SnapshotCaptureError::classify(detail);
+        assert_eq!(error.reason, SnapshotCaptureReason::InvalidSourcePath);
+        assert!(!error.retryable());
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn symlink_special_and_unreadable_material_fail_closed() {
+        use std::os::unix::fs::{symlink, PermissionsExt};
+        let cases = ["symlink", "special", "unreadable"];
+        for case in cases {
+            let fixture = Fixture::new(&format!("snapshot-material-{case}"));
+            let ext = fixture.root.join("main/CommonModules/X/Ext");
+            let module = ext.join("Module.bsl");
+            match case {
+                "symlink" => {
+                    fs::remove_file(&module).unwrap();
+                    symlink("../outside.bsl", &module).unwrap();
+                }
+                "special" => {
+                    fs::remove_file(&module).unwrap();
+                    let c = std::ffi::CString::new(module.as_os_str().as_encoded_bytes()).unwrap();
+                    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
+                }
+                "unreadable" => {
+                    fs::set_permissions(module, fs::Permissions::from_mode(0o0)).unwrap();
+                }
+                _ => unreachable!(),
+            }
+            let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+            let service = FilesystemSourceSnapshots::new(&fixture.root).unwrap();
+            let error = service
+                .capture_authoritative(&selection.analysis, &[], 1)
+                .unwrap_err();
+            if matches!(case, "symlink" | "special") {
+                assert_eq!(error.reason, SnapshotCaptureReason::UnsafeSourceTopology);
+                assert!(!error.retryable());
+            } else if case == "unreadable" {
+                assert_eq!(error.reason, SnapshotCaptureReason::TransientSourceIo);
+                assert!(error.retryable());
+            }
+        }
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn edt_absent_marker_rejects_symlinked_ancestor() {
+        use std::os::unix::fs::symlink;
+
+        let root = temp_root("snapshot-edt-absent-symlink-ancestor");
+        write(
+            &root.join("v8project.yaml"),
+            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
+        );
+        write(&root.join("edt/.project"), "project");
+        let external = root.join("external-dt-inf");
+        fs::create_dir_all(&external).unwrap();
+        symlink(&external, root.join("edt/DT-INF")).unwrap();
+        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
+        let service = FilesystemSourceSnapshots::new(&root).unwrap();
+
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(error.reason, SnapshotCaptureReason::UnsafeSourceTopology);
+        assert!(!error.retryable());
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn registered_subtree_error_precedence_is_name_ordered() {
+        use std::os::unix::fs::symlink;
+
+        let mut reasons = Vec::new();
+        for (index, names) in [["z-valid", "a-unsafe"], ["a-unsafe", "z-valid"]]
+            .into_iter()
+            .enumerate()
+        {
+            let root = temp_root(&format!("snapshot-subtree-order-{index}"));
+            let subtree = root.join("subtree");
+            fs::create_dir_all(&subtree).unwrap();
+            for name in names {
+                if name == "a-unsafe" {
+                    symlink(&root, subtree.join(name)).unwrap();
+                } else {
+                    write(&subtree.join(name), "x");
+                }
+            }
+            let mut present = BTreeSet::new();
+            let mut budget = CaptureBudget::new(
+                SnapshotLimits {
+                    max_traversal_entries: 1,
+                    ..SnapshotLimits::default()
+                },
+                Duration::ZERO,
+            );
+            let error =
+                collect_registered_subtree(&root, &subtree, &mut present, &mut budget, &FixedClock)
+                    .unwrap_err();
+            reasons.push(SnapshotCaptureError::classify(error).reason);
+            fs::remove_dir_all(root).unwrap();
+        }
+        assert_eq!(
+            reasons,
+            vec![
+                SnapshotCaptureReason::UnsafeSourceTopology,
+                SnapshotCaptureReason::UnsafeSourceTopology,
+            ]
+        );
+    }
+
+    #[test]
+    fn recognized_edt_configuration_gets_marker_only_diagnostic_snapshot() {
+        let root = temp_root("snapshot-edt");
+        write(
+            &root.join("v8project.yaml"),
+            "format: EDT\nsource-set:\n - { name: main, type: CONFIGURATION, path: edt }\n",
+        );
+        write(&root.join("edt/.project"), "project");
+        write(
+            &root.join("edt/Configuration/Configuration.mdo"),
+            "configuration",
+        );
+        write(&root.join("edt/src/unrelated.bsl"), "ignored");
+        let selection = resolve_source_selection(&root, Some("main"), &[]).unwrap();
+        assert_eq!(selection.analysis.source_format, SourceFormat::Edt);
+        let service = FilesystemSourceSnapshots::new(&root).unwrap();
+        let snapshot = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap();
+        assert_eq!(snapshot.analysis.manifest.entries().len(), 4);
+        assert_eq!(
+            snapshot
+                .analysis
+                .manifest
+                .entries()
+                .values()
+                .filter(|entry| matches!(entry, ManifestEntry::Present(_)))
+                .count(),
+            2
+        );
+        assert!(!snapshot
+            .analysis
+            .manifest
+            .entries()
+            .keys()
+            .any(|path| path.contains("unrelated")));
+    }
+
+    struct Fixture {
+        root: PathBuf,
+    }
+
+    impl Fixture {
+        fn new(name: &str) -> Self {
+            let root = temp_root(name);
+            write(&root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n - { name: main, type: CONFIGURATION, path: main }\n");
+            write_platform_source(&root.join("main"), "X", "AAAA");
+            Self { root }
+        }
+
+        fn add_extension(&self, name: &str, path: &str, object: &str) {
+            let mut yaml = fs::read_to_string(self.root.join("v8project.yaml")).unwrap();
+            yaml.push_str(&format!(
+                " - {{ name: {name}, type: EXTENSION, path: {path} }}\n"
+            ));
+            write(&self.root.join("v8project.yaml"), &yaml);
+            write_platform_source(&self.root.join(path), object, object);
+        }
+
+        fn services(
+            &self,
+        ) -> (
+            crate::domain::source_snapshot::ResolvedSourceSelection,
+            FilesystemSourceSnapshots,
+        ) {
+            (
+                resolve_source_selection(&self.root, Some("main"), &[]).unwrap(),
+                FilesystemSourceSnapshots::new(&self.root).unwrap(),
+            )
+        }
+
+        fn controlled_service(
+            &self,
+            limits: SnapshotLimits,
+            clock: Arc<dyn SnapshotClock>,
+            hook: Arc<dyn CaptureHook>,
+        ) -> FilesystemSourceSnapshots {
+            FilesystemSourceSnapshots {
+                workspace: canonical_workspace(&self.root).unwrap(),
+                limits,
+                clock,
+                hook,
+            }
+        }
+    }
+
+    fn write_platform_source(root: &Path, object: &str, module: &str) {
+        write(&root.join("Configuration.xml"), &format!("<MetaDataObject><Configuration><ChildObjects><CommonModule>{object}</CommonModule></ChildObjects></Configuration></MetaDataObject>"));
+        write(&root.join(format!("CommonModules/{object}.xml")), &format!("<MetaDataObject><CommonModule><Properties><Name>{object}</Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"));
+        write(
+            &root.join(format!("CommonModules/{object}/Ext/Module.bsl")),
+            module,
+        );
+    }
+
+    fn temp_root(name: &str) -> PathBuf {
+        let nonce = SystemTime::now()
+            .duration_since(UNIX_EPOCH)
+            .unwrap()
+            .as_nanos();
+        let root =
+            std::env::temp_dir().join(format!("unica-{name}-{}-{nonce}", std::process::id()));
+        fs::create_dir_all(&root).unwrap();
+        root
+    }
+
+    fn write(path: &Path, text: &str) {
+        write_bytes(path, text.as_bytes());
+    }
+
+    fn write_bytes(path: &Path, bytes: &[u8]) {
+        if let Some(parent) = path.parent() {
+            fs::create_dir_all(parent).unwrap();
+        }
+        fs::write(path, bytes).unwrap();
+    }
+
+    #[derive(Default)]
+    struct FixedClock;
+
+    impl SnapshotClock for FixedClock {
+        fn now(&self) -> Duration {
+            Duration::ZERO
+        }
+    }
+
+    #[derive(Default)]
+    struct AdvancingClock(AtomicU64);
+
+    impl SnapshotClock for AdvancingClock {
+        fn now(&self) -> Duration {
+            Duration::from_millis(self.0.fetch_add(1, Ordering::SeqCst))
+        }
+    }
+
+    #[derive(Debug, Clone, Copy)]
+    enum RaceAction {
+        Add,
+        Remove,
+        Write,
+        Replace,
+        ReplaceAfterOpen,
+        GrowAfterOpen,
+        #[cfg(unix)]
+        ParentSymlinkSwap,
+        #[cfg(unix)]
+        ParentSymlinkSwapBeforeOpen,
+        #[cfg(unix)]
+        FifoSwapBeforeOpen,
+    }
+
+    #[derive(Debug, Clone, Copy)]
+    enum MappingMutation {
+        RenameSource,
+        DeleteMap,
+        MalformedMap,
+        #[cfg(unix)]
+        SymlinkMap,
+    }
+
+    #[derive(Debug, Clone, Copy)]
+    enum FinalValidationMutation {
+        AddParentConfigurations,
+        AddEdtProjectPmf,
+        GrowPresent(u64),
+    }
+
+    struct FinalValidationMutationHook {
+        root: PathBuf,
+        mutation: FinalValidationMutation,
+        fired: std::sync::atomic::AtomicBool,
+    }
+
+    impl FinalValidationMutationHook {
+        fn new(root: PathBuf, mutation: FinalValidationMutation) -> Self {
+            Self {
+                root,
+                mutation,
+                fired: std::sync::atomic::AtomicBool::new(false),
+            }
+        }
+    }
+
+    impl CaptureHook for FinalValidationMutationHook {
+        fn on_event(&self, event: &CaptureEvent) {
+            if !matches!(event, CaptureEvent::BeforeFinalIdentityValidation)
+                || self.fired.swap(true, Ordering::SeqCst)
+            {
+                return;
+            }
+            match self.mutation {
+                FinalValidationMutation::AddParentConfigurations => write(
+                    &self.root.join("main/Ext/ParentConfigurations.bin"),
+                    "appeared",
+                ),
+                FinalValidationMutation::AddEdtProjectPmf => {
+                    write(&self.root.join("edt/DT-INF/PROJECT.PMF"), "appeared")
+                }
+                FinalValidationMutation::GrowPresent(length) => write_bytes(
+                    &self.root.join("main/CommonModules/X/Ext/Module.bsl"),
+                    &vec![b'G'; usize::try_from(length).unwrap()],
+                ),
+            }
+        }
+    }
+
+    struct MappingMutationHook {
+        root: PathBuf,
+        action: MappingMutation,
+        fired: std::sync::atomic::AtomicBool,
+    }
+
+    impl MappingMutationHook {
+        fn new(root: PathBuf, action: MappingMutation) -> Self {
+            Self {
+                root,
+                action,
+                fired: std::sync::atomic::AtomicBool::new(false),
+            }
+        }
+    }
+
+    impl CaptureHook for MappingMutationHook {
+        fn on_event(&self, event: &CaptureEvent) {
+            if !matches!(event, CaptureEvent::InitialPathScansComplete)
+                || self.fired.swap(true, Ordering::SeqCst)
+            {
+                return;
+            }
+            let map = self.root.join("v8project.yaml");
+            match self.action {
+                MappingMutation::RenameSource => {
+                    let yaml = fs::read_to_string(&map).unwrap();
+                    write(&map, &yaml.replace("name: main", "name: renamed"));
+                }
+                MappingMutation::DeleteMap => fs::remove_file(map).unwrap(),
+                MappingMutation::MalformedMap => write(&map, "source-set: ["),
+                #[cfg(unix)]
+                MappingMutation::SymlinkMap => {
+                    use std::os::unix::fs::symlink;
+                    fs::remove_file(&map).unwrap();
+                    let outside = self.root.join("outside-map.yaml");
+                    write(&outside, "source-set: []");
+                    symlink(outside, map).unwrap();
+                }
+            }
+        }
+    }
+
+    struct RaceHook {
+        root: PathBuf,
+        action: RaceAction,
+        fired: std::sync::atomic::AtomicBool,
+    }
+
+    impl RaceHook {
+        fn new(root: PathBuf, action: RaceAction) -> Self {
+            Self {
+                root,
+                action,
+                fired: std::sync::atomic::AtomicBool::new(false),
+            }
+        }
+    }
+
+    impl CaptureHook for RaceHook {
+        fn on_event(&self, event: &CaptureEvent) {
+            let target_event = match self.action {
+                #[cfg(unix)]
+                RaceAction::ParentSymlinkSwapBeforeOpen | RaceAction::FifoSwapBeforeOpen => {
+                    matches!(event, CaptureEvent::BeforeContainedOpen(path) if path.ends_with("Module.bsl"))
+                }
+                RaceAction::ReplaceAfterOpen | RaceAction::GrowAfterOpen => {
+                    matches!(event, CaptureEvent::ContainedOpenEstablished(path) if path.ends_with("Module.bsl"))
+                }
+                _ => {
+                    matches!(event, CaptureEvent::FileHashed(path) if path.ends_with("Module.bsl"))
+                }
+            };
+            if !target_event || self.fired.swap(true, Ordering::SeqCst) {
+                return;
+            }
+            let module = self.root.join("main/CommonModules/X/Ext/Module.bsl");
+            match self.action {
+                RaceAction::Add => write(
+                    &self.root.join("main/CommonModules/X/Ext/ManagerModule.bsl"),
+                    "new",
+                ),
+                RaceAction::Remove => fs::remove_file(module).unwrap(),
+                RaceAction::Write => fs::write(module, "BBBB").unwrap(),
+                RaceAction::Replace => {
+                    let replacement = self.root.join("replacement");
+                    fs::write(&replacement, "CCCC").unwrap();
+                    fs::remove_file(&module).unwrap();
+                    fs::rename(replacement, module).unwrap();
+                }
+                RaceAction::ReplaceAfterOpen => {
+                    let replacement = self.root.join("replacement-after-open");
+                    fs::write(&replacement, "CCCC").unwrap();
+                    fs::remove_file(&module).unwrap();
+                    fs::rename(replacement, module).unwrap();
+                }
+                RaceAction::GrowAfterOpen => {
+                    fs::write(module, vec![b'G'; 1024 * 1024]).unwrap();
+                }
+                #[cfg(unix)]
+                RaceAction::ParentSymlinkSwap => {
+                    use std::os::unix::fs::symlink;
+                    let ext = self.root.join("main/CommonModules/X/Ext");
+                    let saved = self.root.join("main/CommonModules/X/Ext-saved");
+                    fs::rename(&ext, &saved).unwrap();
+                    symlink("Ext-saved", ext).unwrap();
+                }
+                #[cfg(unix)]
+                RaceAction::ParentSymlinkSwapBeforeOpen => {
+                    use std::os::unix::fs::symlink;
+                    let ext = self.root.join("main/CommonModules/X/Ext");
+                    let saved = self.root.join("main/CommonModules/X/Ext-saved");
+                    fs::rename(&ext, &saved).unwrap();
+                    symlink("Ext-saved", ext).unwrap();
+                }
+                #[cfg(unix)]
+                RaceAction::FifoSwapBeforeOpen => {
+                    use std::os::unix::ffi::OsStrExt;
+                    fs::remove_file(&module).unwrap();
+                    let path = std::ffi::CString::new(module.as_os_str().as_bytes()).unwrap();
+                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
+                }
+            }
+        }
+    }
+
+    fn assert_retryable_race(action: RaceAction) {
+        let fixture = Fixture::new(&format!("snapshot-race-{action:?}"));
+        let selection = resolve_source_selection(&fixture.root, Some("main"), &[]).unwrap();
+        let hook = Arc::new(RaceHook::new(fixture.root.clone(), action));
+        let service =
+            fixture.controlled_service(SnapshotLimits::default(), Arc::new(FixedClock), hook);
+        let error = service
+            .capture_authoritative(&selection.analysis, &[], 1)
+            .unwrap_err();
+        assert_eq!(
+            error.reason,
+            SnapshotCaptureReason::SourceChangedDuringCapture,
+            "{action:?}: {error:?}"
+        );
+        assert!(error.retryable());
+    }
+
+    #[cfg(unix)]
+    fn unix_timestamps(path: &Path) -> (libc::timespec, libc::timespec) {
+        use std::os::unix::fs::MetadataExt;
+        let metadata = fs::metadata(path).unwrap();
+        (
+            libc::timespec {
+                tv_sec: metadata.atime(),
+                tv_nsec: metadata.atime_nsec(),
+            },
+            libc::timespec {
+                tv_sec: metadata.mtime(),
+                tv_nsec: metadata.mtime_nsec(),
+            },
+        )
+    }
+
+    #[cfg(unix)]
+    fn restore_unix_timestamps(path: &Path, times: (libc::timespec, libc::timespec)) {
+        use std::os::unix::ffi::OsStrExt;
+        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
+        let values = [times.0, times.1];
+        assert_eq!(
+            unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), values.as_ptr(), 0) },
+            0
+        );
+    }
+}
