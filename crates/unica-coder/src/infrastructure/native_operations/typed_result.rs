use super::{code, form, meta, registry, NativeOperationAdapter};
use crate::{
    application::{metadata_navigation_command, AdapterOutcome},
    domain::{
        project_sources::{SourceFormat, SourceSetKind},
        source_adapters::source_id_for_configured_source_set,
        workspace::WorkspaceContext,
    },
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_application::{
    CurrentSourceAuthorization, LocatedSource, MetadataNavigationService,
    SourceRegistrationResolver,
};
use unica_format_core::source::{
    ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext,
    SourceFamily, SourceId, SourceLocation, TargetIdentity,
};

pub(crate) struct NativeOperationResult {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

impl NativeOperationAdapter {
    pub(crate) fn invoke_with_data(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<NativeOperationResult, String> {
        if !mutating && operation == "meta-info" {
            let command = metadata_navigation_command(args)?;
            let resolver = HostMetadataSourceResolver { context };
            let navigation = metadata_navigation_service().inspect(command, &resolver);
            return Ok(NativeOperationResult {
                adapter: AdapterOutcome::ok("semantic metadata navigation inspected"),
                data: Some(json!({ "navigation": navigation })),
            });
        }

        if mutating {
            match registry::typed_mutation_handler(operation) {
                Some(registry::TypedMutationHandler::CodePatch) => {
                    let execution = if dry_run {
                        code::preview_with_data(args, context)
                    } else {
                        code::apply_with_data(args, context)
                    };
                    return typed_mutation_result(execution.outcome, execution.data, "code patch");
                }
                Some(registry::TypedMutationHandler::FormEdit) if form::has_edit_payload(args) => {
                    let execution = if dry_run {
                        form::preview_with_data(args, context)
                    } else {
                        form::apply_with_data(args, context)
                    };
                    return typed_mutation_result(execution.outcome, execution.data, "form edit");
                }
                Some(registry::TypedMutationHandler::FormEdit) => {}
                None => {}
            }
        }

        Self::invoke(operation, tool_name, args, context, dry_run, mutating).map(|adapter| {
            NativeOperationResult {
                adapter,
                data: None,
            }
        })
    }
}

static METADATA_NAVIGATION_SERVICE: OnceLock<MetadataNavigationService> = OnceLock::new();

fn metadata_navigation_service() -> &'static MetadataNavigationService {
    METADATA_NAVIGATION_SERVICE
        .get_or_init(|| MetadataNavigationService::new(uuid::Uuid::new_v4().as_bytes().to_vec()))
}

struct HostMetadataSourceResolver<'a> {
    context: &'a WorkspaceContext,
}

impl SourceRegistrationResolver for HostMetadataSourceResolver<'_> {
    fn locate(&self, object_path: &str) -> Result<LocatedSource, SourceAdapterError> {
        let requested = PathBuf::from(object_path);
        let requested = if requested.is_absolute() {
            requested
        } else {
            self.context.cwd.join(requested)
        };
        let target = meta::resolve_meta_info_path(requested)
            .map_err(|_| source_unavailable("metadata target cannot be resolved"))?;
        let source_map = crate::infrastructure::project_sources::discover_project_source_map(
            &self.context.workspace_root,
        )
        .map_err(|_| source_unavailable("project source map cannot be resolved"))?;
        let canonical_target =
            crate::infrastructure::source_roots::normalize_contained_source_root(
                &self.context.workspace_root,
                &target,
            )
            .map_err(|_| {
                source_unavailable("metadata target cannot be resolved inside the workspace")
            })?;
        let config_path = source_map.config_path.clone();
        let configured_format_raw = source_map.configured_format_raw.clone();
        let (canonical_root, source_set) = source_map
            .source_sets
            .into_iter()
            .filter_map(|source_set| {
                let root = crate::infrastructure::source_roots::normalize_contained_source_root(
                    &self.context.workspace_root,
                    &source_set.path,
                )
                .ok()?;
                canonical_target
                    .starts_with(&root)
                    .then_some((root, source_set))
            })
            .max_by_key(|(root, _)| root.as_os_str().len())
            .ok_or_else(|| {
                source_unavailable("metadata target is not in a configured source set")
            })?;
        require_platform_xml(source_set.source_format)?;
        let expected_source_id = source_id_for_configured_source_set(&source_set.name)?;
        let authorization_scope = configured_binding_scope(
            &expected_source_id,
            &source_set.name,
            source_set.kind,
            source_set.source_format,
            &source_set.path,
            &canonical_root,
            &source_set.format_evidence,
            &config_path,
            &configured_format_raw,
            self.context.workspace_epoch,
        )?;
        let target_path = source_relative_target(&canonical_root, &canonical_target)?;
        let source = SourceContext::new(
            SourceLocation::new(
                self.context.workspace_root.clone(),
                canonical_root,
                canonical_target,
            ),
            Some(source_set.name),
            SourceFamily::PlatformXml,
            None,
        )
        .with_configured_source_set_kind(Some(core_source_set_kind(source_set.kind)));
        Ok(LocatedSource {
            source,
            expected_source_id,
            target_identity: TargetIdentity::from_normalized_relative_path(&target_path)?,
            authorization_scope,
            registration: PlatformXmlAdapterFactory::new().registration(),
        })
    }

    fn authorize_continuation(
        &self,
        source_id: &SourceId,
    ) -> Result<CurrentSourceAuthorization, SourceAdapterError> {
        let source_map = crate::infrastructure::project_sources::discover_project_source_map(
            &self.context.workspace_root,
        )
        .map_err(|_| source_unavailable("project source map cannot be resolved"))?;
        let config_path = source_map.config_path.clone();
        let configured_format_raw = source_map.configured_format_raw.clone();
        let source_set = source_map
            .source_sets
            .into_iter()
            .find_map(|source_set| {
                let expected = source_id_for_configured_source_set(&source_set.name).ok()?;
                (expected == *source_id).then_some(source_set)
            })
            .ok_or_else(|| {
                source_unavailable("navigation source is unavailable from the project source map")
            })?;
        require_platform_xml(source_set.source_format)?;
        let canonical_root = crate::infrastructure::source_roots::normalize_contained_source_root(
            &self.context.workspace_root,
            &source_set.path,
        )
        .map_err(|_| {
            source_unavailable("navigation source root cannot be resolved inside the workspace")
        })?;
        let authorization_scope = configured_binding_scope(
            source_id,
            &source_set.name,
            source_set.kind,
            source_set.source_format,
            &source_set.path,
            &canonical_root,
            &source_set.format_evidence,
            &config_path,
            &configured_format_raw,
            self.context.workspace_epoch,
        )?;
        Ok(CurrentSourceAuthorization {
            source_id: source_id.clone(),
            authorization_scope,
        })
    }
}

fn require_platform_xml(source_format: SourceFormat) -> Result<(), SourceAdapterError> {
    if source_format != SourceFormat::PlatformXml {
        return Err(source_unavailable(
            "navigation source does not authorize the Platform XML adapter",
        ));
    }
    Ok(())
}

fn core_source_set_kind(kind: SourceSetKind) -> ConfiguredSourceSetKind {
    match kind {
        SourceSetKind::Configuration => ConfiguredSourceSetKind::Configuration,
        SourceSetKind::Extension => ConfiguredSourceSetKind::Extension,
        SourceSetKind::ExternalProcessor => ConfiguredSourceSetKind::ExternalProcessor,
        SourceSetKind::ExternalReport => ConfiguredSourceSetKind::ExternalReport,
    }
}

fn source_relative_target(source_root: &Path, target: &Path) -> Result<String, SourceAdapterError> {
    let path = target
        .strip_prefix(source_root)
        .map_err(|_| source_unavailable("source target is outside its source root"))?
        .to_str()
        .ok_or_else(|| source_unavailable("source target path is not UTF-8"))?
        .replace('\\', "/");
    if path.is_empty() {
        return Err(source_unavailable("source target path is empty"));
    }
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
fn configured_binding_scope(
    source_id: &SourceId,
    name: &str,
    kind: SourceSetKind,
    source_format: SourceFormat,
    configured_path: &str,
    canonical_root: &Path,
    format_evidence: &[String],
    config_path: &Option<String>,
    configured_format_raw: &Option<String>,
    workspace_epoch: u64,
) -> Result<String, SourceAdapterError> {
    let mut digest = Sha256::new();
    digest.update(b"unica.meta.navigation.scope.v3\0");
    let tuple = serde_json::to_vec(&(
        source_id,
        name,
        kind,
        source_format,
        configured_path,
        canonical_root.as_os_str().as_encoded_bytes(),
        format_evidence,
        config_path,
        configured_format_raw,
        workspace_epoch,
    ))
    .map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("cannot serialize navigation authorization scope: {error}"),
        )
    })?;
    digest.update(tuple);
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn source_unavailable(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_GATEWAY_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn meta_info_typed_gateway_returns_direct_navigation_for_normal_dry_run_and_unavailable() {
        let (context, path) = fixture("2.20");
        let args = json!({"ObjectPath": path}).as_object().unwrap().clone();
        for dry_run in [false, true] {
            let result = NativeOperationAdapter::invoke_with_data(
                "meta-info",
                "unica.meta.info",
                &args,
                &context,
                dry_run,
                false,
            )
            .unwrap();
            assert!(result.adapter.ok);
            assert!(result.adapter.stdout.is_none());
            let data = result.data.unwrap();
            let navigation = &data["navigation"];
            let keys = navigation
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                keys,
                std::collections::BTreeSet::from([
                    "diagnostics".to_string(),
                    "nodes".to_string(),
                    "relations".to_string(),
                    "root".to_string(),
                    "schemaVersion".to_string(),
                    "snapshot".to_string(),
                    "status".to_string()
                ])
            );
            assert_eq!(navigation["status"], "ready");
            assert!(navigation.get("graph").is_none());
            assert!(data.get("text").is_none());
        }
        std::fs::remove_dir_all(context.workspace_root).unwrap();

        let (unsupported_context, unsupported_path) = fixture("2.19");
        let unavailable = NativeOperationAdapter::invoke_with_data(
            "meta-info",
            "unica.meta.info",
            &json!({"ObjectPath": unsupported_path})
                .as_object()
                .unwrap()
                .clone(),
            &unsupported_context,
            false,
            false,
        )
        .unwrap();
        assert!(unavailable.adapter.ok);
        assert!(unavailable.adapter.stdout.is_none());
        let navigation = &unavailable.data.unwrap()["navigation"];
        assert_eq!(navigation["status"], "unavailable");
        assert!(navigation["snapshot"].is_null());
        assert!(navigation["root"].is_null());
        assert_eq!(navigation["nodes"], json!([]));
        assert_eq!(navigation["relations"], json!([]));
        std::fs::remove_dir_all(unsupported_context.workspace_root).unwrap();
    }

    fn fixture(version: &str) -> (WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-typed-gateway-{}-{}",
            std::process::id(),
            NEXT_GATEWAY_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        let src = root.join("src");
        let path = src.join("Catalogs/Items.xml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        std::fs::write(src.join("Configuration.xml"), format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"#)).unwrap();
        std::fs::write(&path, format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Catalog uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#)).unwrap();
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            path,
        )
    }
}

fn typed_mutation_result<T: Serialize>(
    adapter: AdapterOutcome,
    data: Option<T>,
    operation: &str,
) -> Result<NativeOperationResult, String> {
    let data = data
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("serialize typed {operation} result: {error}"))?;
    Ok(NativeOperationResult { adapter, data })
}
