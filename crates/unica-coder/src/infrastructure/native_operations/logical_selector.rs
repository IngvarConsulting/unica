//! Logical target → attached resource for subject readers (ADR-0049).
//!
//! The derivation rule is one line of the 8.3.27 layout, verified against a
//! real Designer dump: a descriptor `<…>/<Stem>.xml` owns its body under
//! `<…>/<Stem>/Ext/<Resource>`. Everything else in this module is re-proving
//! that the derived path is still a direct regular file inside the selected
//! source set — the resolver proves the descriptor, not what hangs off it.

use serde_json::{Map, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::application::source_navigation::{LocateRejection, SourceLocateRequest};
use crate::domain::cancellation::CancellationToken;
use crate::domain::project_sources::SourceFormat;
use crate::domain::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, SourceTargetErrorCode, TargetKind,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::string_arg;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::{
    metadata_is_link_or_reparse_point, path_starts_with_host_root,
    strip_windows_extended_length_prefix,
};
use crate::infrastructure::platform_xml_source_targets::{
    locate_platform_xml_reader_path, platform_xml_resource_evidence, resolve_platform_xml_target,
    PlatformXmlResourceEvidence, TargetKindPolicy,
};
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::{
    normalize_contained_source_root, normalize_path_identity,
    select_unique_deepest_source_set_match,
};

/// What a subject reader opens once its logical target is resolved. The kind of
/// resource belongs to the tool, not to the address: `Report.X.Template.Y`
/// names one file whether `unica.dcs.info` or `unica.mxl.info` asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachedResource {
    /// `Configuration.xml` at the root of the selected source set.
    ConfigurationRoot,
    /// The descriptor itself is what the reader parses.
    Descriptor,
    Rights,
    Form,
    Template,
}

impl AttachedResource {
    /// The file name under `Ext/`. `None` means the descriptor is the resource.
    const fn file_name(self) -> Option<&'static str> {
        match self {
            Self::ConfigurationRoot | Self::Descriptor => None,
            Self::Rights => Some("Rights.xml"),
            Self::Form => Some("Form.xml"),
            // `TemplateType` decides the extension in a real dump: the
            // reference corpus holds 625 `Template.xml`, 150 `Template.bin`
            // and 30 `Template.txt`. The readers that ask for a template parse
            // XML, so a binary or text template is `resource_absent` — the
            // object is addressable, this body is not the one asked for.
            Self::Template => Some("Template.xml"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedReadTarget {
    pub(crate) target: ResolvedTarget,
    pub(crate) resource_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalSelectorFailure {
    code: &'static str,
    reason: &'static str,
}

impl LogicalSelectorFailure {
    const fn new(code: &'static str, reason: &'static str) -> Self {
        Self { code, reason }
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for LogicalSelectorFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Through the accessor, so the stable code has one definition and the
        // rendered message cannot drift from what a caller matches on.
        write!(formatter, "{}: {}", self.code(), self.reason)
    }
}

/// `None` means the caller did not use the logical selector at all, so the
/// caller resolves its legacy path exactly as before. `Some(Err(_))` means the
/// caller did use it and it failed — never a reason to fall back to a path.
///
/// A `sourceSet` that is present but unusable — empty, blank or not a string —
/// is the second case, not the first. Treating it as absent would answer a
/// deliberate logical call with a complaint about a missing path, and any
/// caller branching on key presence alone would disagree with this function
/// about which selector was even used.
pub(crate) fn logical_selection(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    want: AttachedResource,
    accepted_kinds: &[&str],
) -> Option<Result<ResolvedReadTarget, LogicalSelectorFailure>> {
    if string_arg(args, &["sourceSet"]).is_none() {
        if args.contains_key("sourceSet") {
            return Some(Err(LogicalSelectorFailure::new(
                "source_set_unknown",
                "sourceSet must be a non-empty string naming a project source set",
            )));
        }
        return None;
    }
    Some(resolve(args, context, want, accepted_kinds))
}

fn resolve(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    want: AttachedResource,
    accepted_kinds: &[&str],
) -> Result<ResolvedReadTarget, LogicalSelectorFailure> {
    let source_set = string_arg(args, &["sourceSet"])
        .ok_or_else(|| {
            LogicalSelectorFailure::new("source_set_unknown", "sourceSet must name a source set")
        })?
        .to_string();
    let metadata_path = match string_arg(args, &["metadataPath"]) {
        Some(raw) => Some(
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw)
                .map_err(|error| selector_failure(error.code))?,
        ),
        None => None,
    };
    if let Some(address) = metadata_path.as_ref() {
        ensure_kind_is_read_by_this_tool(address, accepted_kinds)?;
    }

    let target = SourceTarget {
        source_set,
        metadata_path,
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| selector_failure(error.code))?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        LogicalSelectorFailure::new("provider_unavailable", "the target evidence is unavailable")
    })?;

    let resource_path =
        resource_for_target(resolution.resolved.target_kind, &evidence, want, context)?;

    Ok(ResolvedReadTarget {
        target: resolution.resolved,
        resource_path,
    })
}

/// Recovers the logical owner of a temporary path selector without deriving an
/// address from directory names. `source.locate` supplies the inverse mapping;
/// the normal resolver and resource proof then have to reproduce the exact
/// file the caller supplied.
pub(crate) fn physical_selection(
    resource_path: &Path,
    context: &WorkspaceContext,
    want: AttachedResource,
) -> Result<ResolvedReadTarget, LogicalSelectorFailure> {
    let lexical_resource = WorkspacePathPolicy::new(context)
        .resolve_write(resource_path.to_path_buf())
        .map_err(|_| {
            LogicalSelectorFailure::new(
                "containment_denied",
                "the physical selector is outside the workspace boundary",
            )
        })?;
    let resource_identity = normalize_path_identity(&lexical_resource).map_err(|_| {
        LogicalSelectorFailure::new(
            "provider_unavailable",
            "the physical selector identity is unavailable",
        )
    })?;
    let source_map = discover_project_source_map(&context.workspace_root).map_err(|_| {
        LogicalSelectorFailure::new(
            "provider_unavailable",
            "the project source map is unavailable",
        )
    })?;
    let mut containing = Vec::new();
    for source_set in &source_map.source_sets {
        let root = normalize_contained_source_root(&context.workspace_root, &source_set.path)
            .map_err(|_| {
                LogicalSelectorFailure::new(
                    "provider_unavailable",
                    "a project source root identity is unavailable",
                )
            })?;
        if path_starts_with_host_root(&resource_identity, &root) {
            containing.push((source_set, root));
        }
    }
    let (source_set, source_root) =
        select_unique_deepest_source_set_match(&resource_identity, containing)
            .map_err(|_| {
                LogicalSelectorFailure::new(
                    "provider_unavailable",
                    "the physical selector has ambiguous source-set ownership",
                )
            })?
            .ok_or_else(|| {
                LogicalSelectorFailure::new(
                    "target_not_found",
                    "the physical selector is outside every registered source set",
                )
            })?;
    if source_set.source_format != SourceFormat::PlatformXml {
        return Err(LogicalSelectorFailure::new(
            "provider_unavailable",
            "the selected source format has no physical target locator",
        ));
    }
    let lexical_source_root = WorkspacePathPolicy::new(context)
        .resolve_write(context.workspace_root.join(&source_set.path))
        .map_err(|_| {
            LogicalSelectorFailure::new(
                "provider_unavailable",
                "the lexical source root is unavailable",
            )
        })?;
    let component_root = if path_starts_with_host_root(&lexical_resource, &lexical_source_root) {
        &lexical_source_root
    } else if path_starts_with_host_root(&resource_identity, &source_root) {
        &source_root
    } else {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the physical selector escaped the selected source set",
        ));
    };
    ensure_no_link_components(component_root, &lexical_resource)?;

    let metadata_path = if want == AttachedResource::ConfigurationRoot {
        let configuration = normalize_path_identity(&source_root.join("Configuration.xml"))
            .map_err(|_| {
                LogicalSelectorFailure::new(
                    "provider_unavailable",
                    "the configuration descriptor identity is unavailable",
                )
            })?;
        if resource_identity != configuration {
            return Err(LogicalSelectorFailure::new(
                "target_not_found",
                "the physical selector is not the source-root descriptor",
            ));
        }
        None
    } else {
        let located = locate_platform_xml_reader_path(
            context,
            &SourceLocateRequest {
                source_set: source_set.name.clone(),
                path: lexical_resource.display().to_string(),
            },
            &CancellationToken::new(),
        )
        .map_err(|_| {
            LogicalSelectorFailure::new(
                "provider_unavailable",
                "the physical target locator is unavailable",
            )
        })?;
        match want {
            AttachedResource::Descriptor if located.rejection.is_none() => located.metadata_path,
            AttachedResource::Rights | AttachedResource::Form | AttachedResource::Template
                if located.rejection == Some(LocateRejection::NotAddressable) =>
            {
                located.owner_metadata_path
            }
            _ => None,
        }
        .ok_or_else(|| locate_failure(located.rejection))?
        .into()
    };
    let source_target = SourceTarget {
        source_set: source_set.name.clone(),
        metadata_path,
    };
    let resolution = resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any)
        .map_err(|error| selector_failure(error.code))?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        LogicalSelectorFailure::new("provider_unavailable", "the target evidence is unavailable")
    })?;
    let proven = resource_for_target(resolution.resolved.target_kind, &evidence, want, context)?;
    let proven_identity = normalize_path_identity(&proven).map_err(|_| {
        LogicalSelectorFailure::new(
            "provider_unavailable",
            "the proven resource identity is unavailable",
        )
    })?;
    if proven_identity != resource_identity {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the physical selector did not reproduce the proven target resource",
        ));
    }
    Ok(ResolvedReadTarget {
        target: resolution.resolved,
        resource_path: proven_identity,
    })
}

fn locate_failure(rejection: Option<LocateRejection>) -> LogicalSelectorFailure {
    match rejection {
        Some(LocateRejection::OutsideSourceSet) => LogicalSelectorFailure::new(
            "containment_denied",
            "the physical selector escaped the selected source set",
        ),
        Some(LocateRejection::OwnerUnproven) => LogicalSelectorFailure::new(
            "target_not_found",
            "the physical selector owner could not be proven",
        ),
        Some(LocateRejection::NotAddressable) | None => LogicalSelectorFailure::new(
            "target_not_found",
            "the physical selector has no logical owner",
        ),
    }
}

fn resource_for_target(
    target_kind: TargetKind,
    evidence: &PlatformXmlResourceEvidence,
    want: AttachedResource,
    context: &WorkspaceContext,
) -> Result<PathBuf, LogicalSelectorFailure> {
    Ok(match (target_kind, want) {
        (TargetKind::SourceRoot, AttachedResource::ConfigurationRoot) => prove_regular_file(
            evidence,
            evidence.target_path.join("Configuration.xml"),
            context,
        )?,
        (TargetKind::MetadataObject, AttachedResource::Descriptor) => {
            prove_regular_file(evidence, evidence.target_path.clone(), context)?
        }
        // A configuration root is not an object, so asking for one by address
        // is a caller mistake. Matching it here keeps `file_name` total for
        // every arm that reaches it, instead of leaving a panic behind an
        // `expect` that only the current schema happens to prevent.
        (TargetKind::MetadataObject, AttachedResource::ConfigurationRoot) => {
            return Err(LogicalSelectorFailure::new(
                "target_kind_unsupported",
                "metadataPath does not identify what this tool reads",
            ))
        }
        (TargetKind::MetadataObject, resource) => {
            let Some(file_name) = resource.file_name() else {
                return Err(LogicalSelectorFailure::new(
                    "target_kind_unsupported",
                    "metadataPath does not identify what this tool reads",
                ));
            };
            prove_attached_resource(evidence, file_name, context)?
        }
        _ => {
            return Err(LogicalSelectorFailure::new(
                "target_kind_unsupported",
                "metadataPath does not identify what this tool reads",
            ))
        }
    })
}

/// The kind is decided by the leading segment for a top-level object and by the
/// nested segment for a child, because `Catalog.Items.Form.Order` is a form and
/// `Catalog.Items` is not.
fn ensure_kind_is_read_by_this_tool(
    address: &MetadataAddress,
    accepted_kinds: &[&str],
) -> Result<(), LogicalSelectorFailure> {
    if accepted_kinds.is_empty() {
        return Ok(());
    }
    let segments = address.as_str().split('.').collect::<Vec<_>>();
    let leading = segments.first().copied().unwrap_or_default();
    let nested = segments.get(2).copied().unwrap_or_default();
    if accepted_kinds.contains(&leading) || accepted_kinds.contains(&nested) {
        return Ok(());
    }
    Err(LogicalSelectorFailure::new(
        "target_kind_unsupported",
        "metadataPath does not identify what this tool reads",
    ))
}

fn selector_failure(code: SourceTargetErrorCode) -> LogicalSelectorFailure {
    match code {
        SourceTargetErrorCode::SourceSetRequired | SourceTargetErrorCode::SourceSetNotFound => {
            LogicalSelectorFailure::new(
                "source_set_unknown",
                "the requested source set is unavailable",
            )
        }
        SourceTargetErrorCode::MetadataAddressNotFound => LogicalSelectorFailure::new(
            "target_not_found",
            "the logical target was not found in the selected source set",
        ),
        SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::MetadataAddressInvalid => LogicalSelectorFailure::new(
            "target_kind_unsupported",
            "metadataPath does not identify what this tool reads",
        ),
        SourceTargetErrorCode::ContainmentDenied => LogicalSelectorFailure::new(
            "containment_denied",
            "the logical target failed its containment checks",
        ),
        SourceTargetErrorCode::AddressProfileUnsupported => LogicalSelectorFailure::new(
            "profile_unsupported",
            "the logical address profile is unsupported",
        ),
        SourceTargetErrorCode::SourceRootNotAddressable => LogicalSelectorFailure::new(
            "provider_unavailable",
            "the logical source provider is unavailable",
        ),
    }
}

/// `<…>/<Stem>.xml` → `<…>/<Stem>/Ext/<file_name>`.
pub(crate) fn prove_attached_resource(
    evidence: &PlatformXmlResourceEvidence,
    file_name: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, LogicalSelectorFailure> {
    let stem = evidence
        .target_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            LogicalSelectorFailure::new("provider_unavailable", "the descriptor has no file stem")
        })?;
    let parent = evidence.target_path.parent().ok_or_else(|| {
        LogicalSelectorFailure::new(
            "provider_unavailable",
            "the descriptor has no containing directory",
        )
    })?;
    prove_regular_file(
        evidence,
        parent.join(stem).join("Ext").join(file_name),
        context,
    )
}

fn prove_regular_file(
    evidence: &PlatformXmlResourceEvidence,
    candidate: PathBuf,
    context: &WorkspaceContext,
) -> Result<PathBuf, LogicalSelectorFailure> {
    let candidate = WorkspacePathPolicy::new(context)
        .resolve_write(candidate)
        .map_err(|_| {
            LogicalSelectorFailure::new(
                "containment_denied",
                "the resource is outside the workspace boundary",
            )
        })?;
    ensure_no_link_components(&evidence.source_root, &candidate)?;
    let normalized_root = normalize_path_identity(&evidence.source_root).map_err(|_| {
        LogicalSelectorFailure::new(
            "provider_unavailable",
            "the source root identity is unavailable",
        )
    })?;
    let normalized = normalize_path_identity(&candidate).map_err(|_| {
        LogicalSelectorFailure::new("resource_absent", "the requested resource is not present")
    })?;
    if !path_starts_with_host_root(&normalized, &normalized_root) {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the resource escaped the selected source set",
        ));
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|_| {
        LogicalSelectorFailure::new("resource_absent", "the requested resource is not present")
    })?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the resource is not a direct regular file",
        ));
    }
    Ok(candidate)
}

fn ensure_no_link_components(
    source_root: &Path,
    target: &Path,
) -> Result<(), LogicalSelectorFailure> {
    // Keep the lexical components and remove only Windows' equivalent verbatim
    // spelling. Canonicalizing here would erase a linked parent before it can
    // be rejected by the component walk.
    let source_root = strip_windows_extended_length_prefix(source_root);
    let target = strip_windows_extended_length_prefix(target);
    if !path_starts_with_host_root(&target, &source_root) {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the resource escaped the selected source set",
        ));
    }
    let root_component_count = source_root.components().count();
    let relative = target.components().skip(root_component_count);
    let mut current = source_root;
    for component in relative {
        let std::path::Component::Normal(component) = component else {
            return Err(LogicalSelectorFailure::new(
                "containment_denied",
                "the resource contains a non-normal path component",
            ));
        };
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(LogicalSelectorFailure::new(
                    "resource_absent",
                    "the requested resource is not present",
                ))
            }
            Err(_) => {
                return Err(LogicalSelectorFailure::new(
                    "provider_unavailable",
                    "the resource is unreadable",
                ))
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(LogicalSelectorFailure::new(
                "containment_denied",
                "the resource path traverses a link",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, remove_dir_symlink_for_test,
        windows_extended_length_path_for_test, FileLinkFixtureOutcome,
    };
    use serde_json::{Map, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-logical-selector-{name}-{}-{nanos}-{nonce}",
            std::process::id()
        ))
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// Minimal Designer dump: a role, a catalog with a nested form, a report
    /// with a DCS template, and a common template whose body is binary.
    fn fixture(name: &str) -> WorkspaceContext {
        let root = temp_root(name);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let src = root.join("src");

        write_descriptor(&src, "Roles", "Role", "Sales");
        write_body(&src, "Roles/Sales/Ext/Rights.xml", "<Rights/>");

        write_descriptor(&src, "Catalogs", "Catalog", "Items");
        write_nested_descriptor(&src, "Catalogs", "Catalog", "Items", "Form", "Order");
        write_body(&src, "Catalogs/Items/Forms/Order/Ext/Form.xml", "<Form/>");

        write_descriptor(&src, "Reports", "Report", "Sales");
        write_nested_descriptor(&src, "Reports", "Report", "Sales", "Template", "Main");
        write_body(
            &src,
            "Reports/Sales/Templates/Main/Ext/Template.xml",
            "<Document/>",
        );

        // A `TemplateType` other than an XML one writes `Template.bin`: the
        // descriptor is present and addressable, the XML body is not.
        write_descriptor(&src, "CommonTemplates", "CommonTemplate", "Logo");
        write_body(&src, "CommonTemplates/Logo/Ext/Template.bin", "\u{0}\u{1}");

        write_descriptor(&src, "Subsystems", "Subsystem", "SalesOps");

        context
    }

    fn write_body(source_root: &Path, relative: &str, content: &str) {
        let path = source_root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn write_descriptor(source_root: &Path, directory: &str, kind: &str, name: &str) {
        let directory = source_root.join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{name}.xml")),
            descriptor_image(kind, name),
        )
        .unwrap();
        register_fixture_item(
            &source_root.join("Configuration.xml"),
            "Configuration",
            kind,
            name,
        );
    }

    fn write_nested_descriptor(
        source_root: &Path,
        directory: &str,
        owner_kind: &str,
        owner: &str,
        kind: &str,
        name: &str,
    ) {
        let nested_directory = source_root
            .join(directory)
            .join(owner)
            .join(format!("{kind}s"));
        fs::create_dir_all(&nested_directory).unwrap();
        fs::write(
            nested_directory.join(format!("{name}.xml")),
            descriptor_image(kind, name),
        )
        .unwrap();
        register_fixture_item(
            &source_root.join(directory).join(format!("{owner}.xml")),
            owner_kind,
            kind,
            name,
        );
    }

    fn descriptor_image(kind: &str, name: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name></Properties></{kind}></MetaDataObject>"#
        )
    }

    /// Registration is what makes an address resolvable: a descriptor on disk
    /// that its owner does not list is not a target.
    fn register_fixture_item(owner: &Path, owner_kind: &str, kind: &str, name: &str) {
        let registration = format!("<{kind}>{name}</{kind}>");
        let mut image = fs::read_to_string(owner).unwrap_or_else(|_| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{owner_kind}><ChildObjects></ChildObjects></{owner_kind}></MetaDataObject>"#
            )
        });
        if !image.contains(&registration) {
            if let Some(insertion) = image.rfind("</ChildObjects>") {
                image.insert_str(insertion, &registration);
            } else {
                let closing = format!("</{owner_kind}>");
                let insertion = image
                    .rfind(&closing)
                    .expect("fixture owner has its declared closing element");
                image.insert_str(
                    insertion,
                    &format!("<ChildObjects>{registration}</ChildObjects>"),
                );
            }
        }
        fs::create_dir_all(owner.parent().unwrap()).unwrap();
        fs::write(owner, image).unwrap();
    }

    fn selection(
        context: &WorkspaceContext,
        address: &str,
        want: AttachedResource,
        kinds: &[&str],
    ) -> Result<ResolvedReadTarget, LogicalSelectorFailure> {
        let args = Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            (
                "metadataPath".to_string(),
                Value::String(address.to_string()),
            ),
        ]);
        logical_selection(&args, context, want, kinds).expect("a logical selector was supplied")
    }

    #[test]
    fn logical_selector_derives_attached_resources_from_the_proven_descriptor() {
        let context = fixture("attached-resources");
        let src = context.workspace_root.join("src");

        assert!(
            selection(&context, "Role.Sales", AttachedResource::Rights, &["Role"])
                .unwrap()
                .resource_path
                .ends_with("Roles/Sales/Ext/Rights.xml")
        );
        assert!(selection(
            &context,
            "Catalog.Items.Form.Order",
            AttachedResource::Form,
            &["Form"]
        )
        .unwrap()
        .resource_path
        .ends_with("Catalogs/Items/Forms/Order/Ext/Form.xml"));
        assert!(selection(
            &context,
            "Report.Sales.Template.Main",
            AttachedResource::Template,
            &["Template"]
        )
        .unwrap()
        .resource_path
        .ends_with("Reports/Sales/Templates/Main/Ext/Template.xml"));
        assert!(
            selection(&context, "Catalog.Items", AttachedResource::Descriptor, &[])
                .unwrap()
                .resource_path
                .ends_with("Catalogs/Items.xml")
        );
        assert!(src.join("Roles/Sales/Ext/Rights.xml").is_file());
        cleanup(&context);
    }

    #[test]
    fn logical_selector_reads_the_configuration_descriptor_from_the_source_set_alone() {
        let context = fixture("root-selects-configuration");
        let args = Map::from_iter([("sourceSet".to_string(), Value::String("main".to_string()))]);
        let selection =
            logical_selection(&args, &context, AttachedResource::ConfigurationRoot, &[])
                .expect("sourceSet alone is a logical selector")
                .unwrap();

        assert!(selection.resource_path.ends_with("src/Configuration.xml"));
        assert_eq!(selection.target.metadata_path, None);
        assert_eq!(selection.target.source_set, "main");
        assert_eq!(selection.target.target_kind, TargetKind::SourceRoot);
        cleanup(&context);
    }

    #[test]
    fn logical_selector_canonicalizes_a_russian_kind_alias() {
        let context = fixture("russian-alias");
        let selection = selection(&context, "Роль.Sales", AttachedResource::Rights, &["Role"])
            .expect("a Russian kind alias resolves to the same target");
        assert_eq!(
            selection
                .target
                .metadata_path
                .as_ref()
                .map(|path| path.as_str()),
            Some("Role.Sales")
        );
        cleanup(&context);
    }

    #[test]
    fn physical_support_target_uses_the_owner_of_an_attached_resource() {
        let context = fixture("physical-owners");
        for (address, want, kinds) in [
            ("Role.Sales", AttachedResource::Rights, &["Role"][..]),
            (
                "Catalog.Items.Form.Order",
                AttachedResource::Form,
                &["Form"][..],
            ),
            (
                "Report.Sales.Template.Main",
                AttachedResource::Template,
                &["Template"][..],
            ),
        ] {
            let logical = selection(&context, address, want, kinds).unwrap();
            let physical = physical_selection(&logical.resource_path, &context, want).unwrap();
            assert_eq!(physical.target, logical.target, "{address}");
            assert_eq!(physical.resource_path, logical.resource_path, "{address}");
        }
        cleanup(&context);
    }

    #[test]
    fn physical_support_target_rejects_unregistered_and_ambiguous_paths() {
        let context = fixture("physical-refusals");
        let unregistered = context.workspace_root.join("unregistered.xml");
        fs::write(&unregistered, "<MetaDataObject/>").unwrap();
        assert_eq!(
            physical_selection(&unregistered, &context, AttachedResource::Descriptor)
                .expect_err("a path outside every source set has no logical target")
                .code(),
            "target_not_found"
        );

        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: duplicate\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let descriptor = context.workspace_root.join("src/Subsystems/SalesOps.xml");
        assert_eq!(
            physical_selection(&descriptor, &context, AttachedResource::Descriptor)
                .expect_err("equally specific source sets cannot choose an identity")
                .code(),
            "provider_unavailable"
        );
        cleanup(&context);
    }

    #[test]
    fn configuration_xml_maps_to_the_source_root() {
        let context = fixture("physical-root");
        let resource = context.workspace_root.join("src/Configuration.xml");

        let selection =
            physical_selection(&resource, &context, AttachedResource::ConfigurationRoot).unwrap();

        assert_eq!(selection.target.source_set, "main");
        assert_eq!(selection.target.metadata_path, None);
        assert_eq!(selection.target.target_kind, TargetKind::SourceRoot);
        assert_eq!(
            selection.resource_path,
            normalize_path_identity(&resource).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn descriptor_maps_to_its_own_metadata_address() {
        let context = fixture("physical-descriptor");
        let resource = context.workspace_root.join("src/Subsystems/SalesOps.xml");

        let selection =
            physical_selection(&resource, &context, AttachedResource::Descriptor).unwrap();

        assert_eq!(
            selection
                .target
                .metadata_path
                .as_ref()
                .map(MetadataAddress::as_str),
            Some("Subsystem.SalesOps")
        );
        assert_eq!(selection.target.target_kind, TargetKind::MetadataObject);
        assert_eq!(
            selection.resource_path,
            normalize_path_identity(&resource).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn physical_support_target_accepts_regular_and_verbatim_windows_paths() {
        let context = fixture("physical-windows-verbatim");
        let resource = context.workspace_root.join("src/Configuration.xml");
        let Some(verbatim) = windows_extended_length_path_for_test(&resource) else {
            cleanup(&context);
            return;
        };

        let regular = physical_selection(&resource, &context, AttachedResource::ConfigurationRoot)
            .expect("the regular Windows spelling resolves");
        let extended = physical_selection(&verbatim, &context, AttachedResource::ConfigurationRoot)
            .expect("the equivalent verbatim Windows spelling resolves");

        assert_eq!(extended.target, regular.target);
        assert_eq!(extended.resource_path, regular.resource_path);
        assert!(
            !extended
                .resource_path
                .as_os_str()
                .to_string_lossy()
                .starts_with(r"\\?\"),
            "the public resource identity must not expose a Windows service prefix"
        );
        cleanup(&context);
    }

    #[test]
    fn physical_support_target_rejects_a_parent_directory_link_or_reparse_point() {
        let context = fixture("physical-windows-parent-reparse");
        let src = context.workspace_root.join("src");
        let linked_parent = src.join("Roles/Sales");
        let referent = src.join("linked-sales");
        fs::rename(&linked_parent, &referent).unwrap();
        match create_directory_link_fixture_for_test(&referent, &linked_parent).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                cleanup(&context);
                return;
            }
        }

        let failure = physical_selection(
            &linked_parent.join("Ext/Rights.xml"),
            &context,
            AttachedResource::Rights,
        )
        .expect_err("a resource below a reparse-point parent must be refused");

        assert_eq!(failure.code(), "containment_denied");
        remove_dir_symlink_for_test(&linked_parent).unwrap();
        cleanup(&context);
    }

    #[test]
    fn logical_selector_reports_a_proven_object_without_the_resource_as_absent() {
        let context = fixture("binary-template");
        let failure = selection(
            &context,
            "CommonTemplate.Logo",
            AttachedResource::Template,
            &["CommonTemplate"],
        )
        .expect_err("a .bin template has no Template.xml");
        assert_eq!(failure.code(), "resource_absent");
        cleanup(&context);
    }

    /// A configuration root has no address, so asking for one while naming an
    /// object is a caller mistake — and a mistake must be a typed refusal, not
    /// a panic. The public schema keeps `metadataPath` off `unica.cf.*` today,
    /// but this seam is shared and must not rely on that.
    #[test]
    fn logical_selector_refuses_a_configuration_root_asked_for_by_address() {
        let context = fixture("root-asked-by-address");
        let failure = selection(
            &context,
            "Catalog.Items",
            AttachedResource::ConfigurationRoot,
            &[],
        )
        .expect_err("a metadata object is not a configuration root");
        assert_eq!(failure.code(), "target_kind_unsupported");
        cleanup(&context);
    }

    #[test]
    fn logical_selector_refuses_an_address_of_another_kind() {
        let context = fixture("wrong-kind");
        let failure = selection(
            &context,
            "Catalog.Items",
            AttachedResource::Rights,
            &["Role"],
        )
        .expect_err("a catalog is not a role");
        assert_eq!(failure.code(), "target_kind_unsupported");
        cleanup(&context);
    }

    #[test]
    fn logical_selector_refuses_a_symlinked_resource() {
        let context = fixture("symlinked-resource");
        let src = context.workspace_root.join("src");
        let planted = src.join("planted.xml");
        fs::write(&planted, "<Rights/>").unwrap();
        fs::remove_file(src.join("Roles/Sales/Ext/Rights.xml")).unwrap();
        let Some(created) =
            crate::infrastructure::platform::filesystem::create_file_symlink_for_test(
                &planted,
                src.join("Roles/Sales/Ext/Rights.xml"),
            )
        else {
            // The host cannot create symlinks, so there is nothing to refuse.
            cleanup(&context);
            return;
        };
        created.unwrap();

        let failure = selection(&context, "Role.Sales", AttachedResource::Rights, &["Role"])
            .expect_err("a symlinked resource is not a direct regular file");
        assert_eq!(failure.code(), "containment_denied");
        cleanup(&context);
    }

    #[test]
    fn logical_selector_separates_an_unknown_address_from_an_unknown_source_set() {
        let context = fixture("separate-codes");
        assert_eq!(
            selection(
                &context,
                "Role.Missing",
                AttachedResource::Rights,
                &["Role"]
            )
            .expect_err("an absent role is a missing target")
            .code(),
            "target_not_found"
        );

        let args = Map::from_iter([
            ("sourceSet".to_string(), Value::String("nope".to_string())),
            (
                "metadataPath".to_string(),
                Value::String("Role.Sales".to_string()),
            ),
        ]);
        assert_eq!(
            logical_selection(&args, &context, AttachedResource::Rights, &["Role"])
                .unwrap()
                .expect_err("an absent source set is not a missing target")
                .code(),
            "source_set_unknown"
        );
        cleanup(&context);
    }

    #[test]
    fn logical_selector_refusals_do_not_disclose_a_path() {
        let context = fixture("no-path-in-message");
        for (address, kinds) in [
            ("Role.Missing", &["Role"][..]),
            ("Catalog.Items", &["Role"][..]),
        ] {
            let failure = selection(&context, address, AttachedResource::Rights, kinds)
                .expect_err("a refusal is expected");
            assert!(
                !failure.to_string().contains(std::path::MAIN_SEPARATOR),
                "refusal disclosed a path: {failure}"
            );
        }
        cleanup(&context);
    }

    /// A caller who sent `sourceSet` meant to select logically, so an unusable
    /// value is their mistake to hear about — not a silent fall-back to the
    /// path branch, which would answer with a complaint about a missing path.
    #[test]
    fn logical_selector_reports_an_unusable_source_set_rather_than_stepping_aside() {
        let context = fixture("unusable-source-set");

        for value in [
            Value::String(String::new()),
            Value::String("   ".to_string()),
            Value::from(7),
            Value::Null,
        ] {
            let args = Map::from_iter([("sourceSet".to_string(), value.clone())]);
            let failure = logical_selection(&args, &context, AttachedResource::Rights, &["Role"])
                .unwrap_or_else(|| panic!("{value}: an unusable selector is not an absent one"))
                .expect_err("an unusable source set cannot resolve");
            assert_eq!(failure.code(), "source_set_unknown", "{value}");
        }
        cleanup(&context);
    }

    #[test]
    fn logical_selector_leaves_a_legacy_path_call_alone() {
        let context = fixture("legacy-path");
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            Value::String("src/Roles/Sales/Ext/Rights.xml".to_string()),
        )]);
        assert!(logical_selection(&args, &context, AttachedResource::Rights, &["Role"]).is_none());
        cleanup(&context);
    }
}
