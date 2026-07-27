use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    ports::{
        AuthorabilityRequest, AuthorabilityRequirement, CompatibilityIssueKind,
        CompatibilityRequest, CompatibilityTarget, OperationCancellation, OwnerResolutionMode,
        PublicationHostPort, PublicationLockResult, PublicationProcessCommand,
        PublicationProcessOutput, ResolvedPublicationTool, ValidationContextRequest,
    },
    source::{SourceContext, SourceFamily, SourceLocation},
};

struct NoopPublicationHost;

impl PublicationHostPort for NoopPublicationHost {
    fn run_process(
        &self,
        _command: &PublicationProcessCommand,
    ) -> Result<PublicationProcessOutput, String> {
        Err("process execution is not expected in guard tests".to_string())
    }

    fn resolve_bundled_tool(
        &self,
        _cwd: &Path,
        _tool: &str,
        _require_executable: bool,
    ) -> Result<ResolvedPublicationTool, String> {
        Err("tool resolution is not expected in guard tests".to_string())
    }

    fn with_exclusive_publication_lock(
        &self,
        _targets: &[PathBuf],
        action: &mut dyn FnMut() -> Result<Vec<String>, String>,
    ) -> Result<PublicationLockResult, String> {
        Ok(PublicationLockResult::Action(action()))
    }

    fn redact(&self, text: &str) -> String {
        text.to_string()
    }
}

fn source(root: &Path, target: &Path) -> SourceContext {
    SourceContext::new(
        SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task7-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn write_owner(root: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let owner = root.join("Configuration.xml");
    fs::write(
        &owner,
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"#
        ),
    )
    .unwrap();
    owner
}

#[test]
fn task7_compatibility_port_classifies_supported_older_newer_and_malformed_profiles() {
    let operations =
        PlatformXmlAdapterFactory::new().operational_registration(Arc::new(NoopPublicationHost));

    for (version, expected) in [
        ("2.20", None),
        ("2.19", Some(CompatibilityIssueKind::Older)),
        ("2.21", Some(CompatibilityIssueKind::Newer)),
        ("latest", Some(CompatibilityIssueKind::Malformed)),
    ] {
        let root = temp_root(version);
        let owner = write_owner(&root, version);
        let result = operations
            .compatibility
            .inspect(&CompatibilityRequest {
                targets: vec![CompatibilityTarget {
                    source: source(&root, &owner),
                    mode: OwnerResolutionMode::Existing,
                }],
            })
            .unwrap();
        assert_eq!(result.issue.map(|issue| issue.kind), expected, "{version}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn task7_authorability_port_fails_closed_for_malformed_support_state() {
    let root = temp_root("support-malformed");
    let owner = write_owner(&root, "2.20");
    fs::create_dir_all(root.join("Ext")).unwrap();
    fs::write(root.join("Ext/ParentConfigurations.bin"), "<broken-support").unwrap();
    let operations =
        PlatformXmlAdapterFactory::new().operational_registration(Arc::new(NoopPublicationHost));

    let result = operations
        .authorability
        .inspect(&AuthorabilityRequest {
            source: source(&root, &owner),
            requirement: AuthorabilityRequirement::Editable,
        })
        .unwrap();

    assert!(result.violation.is_some());
    assert_ne!(
        result.authorability,
        unica_format_core::navigation::Authorability::Authorable
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_validation_context_returns_bounded_diagnostics_for_malformed_metadata() {
    let root = temp_root("validation-malformed");
    fs::create_dir_all(&root).unwrap();
    let object = root.join("Broken.xml");
    fs::write(&object, "<MetaDataObject").unwrap();
    let operations =
        PlatformXmlAdapterFactory::new().operational_registration(Arc::new(NoopPublicationHost));

    let result = operations
        .validation_context
        .inspect(&ValidationContextRequest {
            source: source(&root, &object),
        })
        .unwrap();

    assert!(result.context.is_none());
    assert_eq!(result.diagnostics.len(), 1);
    assert!(!result.diagnostics[0].message.contains("<MetaDataObject"));
    assert_eq!(result.dependencies, vec![object]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_operation_results_never_serialize_source_locations_into_navigation() {
    let cancellation = OperationCancellation::new();
    assert!(!cancellation.is_cancelled());
    let root = temp_root("path-privacy");
    let owner = write_owner(&root, "2.20");
    let registration = PlatformXmlAdapterFactory::new().registration();
    let captured = match registration
        .capture
        .capture(&source(&root, &owner))
        .unwrap()
    {
        unica_format_core::ports::CaptureResult::Captured(captured) => captured,
        unica_format_core::ports::CaptureResult::NoMatch => panic!("expected capture"),
    };
    let navigation = registration
        .read
        .read(&unica_format_core::ports::FormatReadRequest {
            query: unica_format_core::navigation::NavigationQuery {
                target: unica_format_core::navigation::NavigationTarget::CapturedTarget(
                    captured.binding().target_identity.clone(),
                ),
                select: unica_format_core::navigation::NavigationSelection {
                    properties: unica_format_core::navigation::PropertySelection::All,
                    facets: unica_format_core::navigation::FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
            captured,
        })
        .unwrap();

    assert!(!serde_json::to_string(&navigation)
        .unwrap()
        .contains(&root.display().to_string()));
    fs::remove_dir_all(root).unwrap();
}
