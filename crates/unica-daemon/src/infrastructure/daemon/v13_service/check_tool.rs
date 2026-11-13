//! `unica.check`, `unica.diff` и `unica.run`.

use super::*;

impl CanonicalV13ReadService {
    pub(super) fn execute_check(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        if arguments.contains_key("filter") {
            return error_result(
                None,
                RefusalCode::BadValue,
                "check does not accept a validator filter; validators follow from the node kind",
            );
        }
        if let Some(at) = arguments.get("at") {
            let Some(at) = at.as_str() else {
                return error_result(None, RefusalCode::BadValue, "check at must be a string");
            };
            let limit = match arguments.get("limit") {
                None => DEFAULT_CHECK_LIMIT,
                Some(value) => match bounded_usize(value).filter(|limit| *limit <= MAX_CHECK_LIMIT)
                {
                    Some(limit) => limit,
                    None => {
                        let message = format!(
                            "check limit must be an integer from 1 through {MAX_CHECK_LIMIT}"
                        );
                        return error_result(Some(at.to_string()), RefusalCode::BadValue, message);
                    }
                },
            };
            let cursor = match arguments.get("cursor") {
                None => None,
                Some(Value::String(cursor)) => Some(cursor.as_str()),
                Some(_) => {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "check cursor must be a string",
                    )
                }
            };
            if let Some(cursor) = cursor {
                let binding = ViewCursorBinding {
                    canonical_at: at.to_string(),
                    projection: "check".into(),
                    source_set_identity: format!(
                        "{}:{}",
                        invocation.workspace_identity_hash().as_str(),
                        at.split_once(':').map_or("", |(name, _)| name)
                    ),
                    snapshot_id: String::new(),
                    page_limit: limit,
                };
                return crate::application::v13::check::page_diagnostics(
                    &self.cursors,
                    binding,
                    None,
                    Some(cursor),
                );
            }
            let view_arguments =
                Map::from_iter([("at".to_string(), Value::String(at.to_string()))]);
            let (viewed, mut prepared_xdto) = match QualifiedAddress::parse(at) {
                Ok(address) if matches!(address.segments(), [owner] if owner.kind() == NodeKind::XdtoPackage && owner.name().is_some()) => {
                    prepare_xdto_check(invocation, &address, cancellation)
                }
                _ => (
                    self.execute_view_arguments(invocation, &view_arguments, cancellation),
                    None,
                ),
            };
            if !viewed.ok {
                if let Some(refusal) = unreadable_target_format_refusal(invocation, at) {
                    return refusal;
                }
                return viewed;
            }
            let Some(revision) = viewed.rev.as_deref() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::InvalidState,
                    "check could not establish its result snapshot",
                );
            };
            let binding = ViewCursorBinding {
                canonical_at: at.to_string(),
                projection: "check".to_string(),
                source_set_identity: format!(
                    "{}:{}",
                    invocation.workspace_identity_hash().as_str(),
                    at.split_once(':').map_or("", |(source_set, _)| source_set)
                ),
                snapshot_id: revision.to_string(),
                page_limit: limit,
            };
            let kind = viewed
                .data
                .as_ref()
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut result = run_node_checks(
                &self.ports,
                invocation,
                at,
                &kind,
                viewed.data.as_ref(),
                cancellation,
                prepared_xdto.take(),
            );
            if kind == "Configuration" {
                result
                    .warnings
                    .extend(compatibility_mode_warnings(invocation, at));
            }
            if result.ok {
                if cancellation.is_cancelled() {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::Cancelled,
                        "check was cancelled before publishing its result",
                    );
                }
                result.rev = Some(revision.to_string());
                return crate::application::v13::check::page_diagnostics(
                    &self.cursors,
                    binding,
                    Some(result),
                    None,
                );
            }
            return result;
        }
        // Вердикт по рабочему пространству отвечается до допуска наборов:
        // он и объясняет, почему допуска нет. Сюда управление доходит только
        // при сломанной маршрутизации, и молчать об этом нельзя.
        error_result(
            None,
            RefusalCode::InvalidState,
            "workspace check was routed past the pre-admission root answer",
        )
    }

    pub(super) fn execute_diff(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        if arguments.contains_key("executionToken") {
            let Some(token) = arguments
                .get("executionToken")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
            else {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "diff executionToken must be a nonempty string",
                );
            };
            if arguments
                .keys()
                .any(|key| !matches!(key.as_str(), "executionToken" | "limit" | "cursor"))
            {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "saved-plan diff accepts only executionToken, limit and cursor",
                );
            }
            let limit = match arguments.get("limit") {
                None => SOURCE_NAVIGATION_LIMIT_DEFAULT,
                Some(value) => match bounded_usize(value)
                    .filter(|limit| *limit <= SOURCE_NAVIGATION_LIMIT_MAX)
                {
                    Some(limit) => limit,
                    None => {
                        return error_result(
                            None,
                            RefusalCode::BadValue,
                            "saved-plan diff limit must be an integer from 1 through 50",
                        )
                    }
                },
            };
            let cursor = match arguments.get("cursor") {
                None => None,
                Some(Value::String(cursor)) => Some(cursor.as_str()),
                Some(_) => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "diff cursor must be a string",
                    )
                }
            };
            return match invocation.read_saved_apply_diff(token, limit, cursor, cancellation) {
                Ok(page) => {
                    let mut result =
                        DomainResult::success("exact saved source ranges read without execution");
                    if let Some(cursor) = page["cursor"].as_str() {
                        result.cursor = Some(cursor.to_owned());
                        result.next.push(json!({"tool": "unica.diff", "arguments": {"executionToken": token, "limit": limit, "cursor": cursor}}));
                    }
                    let mut page = page;
                    let stopped_by = page
                        .as_object_mut()
                        .unwrap()
                        .remove("stoppedBy")
                        .expect("saved source page has a stop reason");
                    result.page = Some(json!({"stoppedBy": stopped_by}));
                    result.data = Some(page);
                    result
                }
                Err(error) => saved_apply_execution_error_result(error),
            };
        }
        if let Some(unknown) = arguments
            .keys()
            .find(|name| !matches!(name.as_str(), "left" | "right" | "limit" | "cursor"))
        {
            return error_result(
                None,
                RefusalCode::BadValue,
                format!("canonical invocation has unknown argument `{unknown}`"),
            );
        }
        let Some(left) = arguments.get("left").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "diff requires string argument `left`",
            );
        };
        let Some(right) = arguments.get("right").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "diff requires string argument `right`",
            );
        };
        // Контракт операции вычисляется из реестра, а не читается из
        // исходников: сравнение его никогда не увидело бы изменения.
        if let Some(computed) = [left, right].into_iter().find(|side| {
            QualifiedAddress::parse(side)
                .ok()
                .is_some_and(|address| address.operation_query().is_some())
        }) {
            return error_result(
                Some(computed.to_string()),
                RefusalCode::TargetKindUnsupported,
                "diff compares source-backed nodes; an operation contract at `.Can.<op>` is computed from the registry, read it with view",
            );
        }
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= MAX_DIFF_LIMIT) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        format!("diff limit must be an integer from 1 through {MAX_DIFF_LIMIT}"),
                    )
                }
            },
            None => DEFAULT_DIFF_LIMIT,
        };
        let cursor = match arguments.get("cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if cursor.is_empty() => None,
            Some(Value::String(cursor)) => Some(cursor.clone()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "diff cursor must be a string")
            }
        };
        if let Some(cursor) = cursor {
            let request = match DiffRequest::new(left, right) {
                Ok(request) => request
                    .with_limit(limit)
                    .in_workspace(invocation.workspace_identity_hash().as_str())
                    .with_cursor(cursor),
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
            return diff_page_result(&request, self.diff.continue_saved(&request));
        }
        let left_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(left.to_string()))]),
            cancellation,
        );
        if !left_result.ok {
            return left_result;
        }
        let right_result = self.execute_view_arguments(
            invocation,
            &Map::from_iter([("at".to_string(), Value::String(right.to_string()))]),
            cancellation,
        );
        if !right_result.ok {
            return right_result;
        }
        let request = match DiffRequest::new(left, right) {
            Ok(request) => request
                .with_limit(limit)
                .in_workspace(invocation.workspace_identity_hash().as_str()),
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let side = |at: &QualifiedAddress, result: &DomainResult| {
            let mut data = result.data.clone().expect("successful view has data");
            source_backed(&mut data);
            let kind = data
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            DiffSource::new(at.clone(), kind, data)
        };
        let left = side(request.left(), &left_result);
        let right = side(request.right(), &right_result);
        diff_page_result(&request, self.diff.compare(&request, &left, &right))
    }

    pub(super) fn execute_run(
        &self,
        invocation: &ActorBoundExecution,
        _cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
        let Some(op) = arguments.get("op") else {
            return super::super::v13_run_dictionary::run_dictionary_result();
        };
        let Some(op) = op.as_str() else {
            return error_result(None, RefusalCode::BadValue, "run op must be a string");
        };
        if arguments.get("args").is_some_and(|args| !args.is_object()) {
            return error_result(None, RefusalCode::BadValue, "run args must be an object");
        }
        if !catalog
            .run_dictionary
            .iter()
            .any(|operation| operation.name() == op)
        {
            return error_result(
                Some(op.to_string()),
                RefusalCode::UnsupportedOperation,
                format!("unknown canonical run operation `{op}`"),
            );
        }
        error_result(
            Some(op.to_string()),
            RefusalCode::UnsupportedOperation,
            format!("canonical run operation `{op}` is not implemented yet"),
        )
    }
}

/// The root properties of `Configuration.xml` sit before its child objects;
/// this prefix holds them in every export the corpora have.
const ROOT_PROPERTIES_PREFIX_BYTES: usize = 64 * 1024;

/// The text of `<tag>` in the root prefix of a set, if it is there.
fn root_mode(root: &unica_source_contract::read::SourceDirectory, tag: &str) -> Option<String> {
    let bytes = root
        .read_relative_regular_prefix(
            std::path::Path::new("Configuration.xml"),
            ROOT_PROPERTIES_PREFIX_BYTES,
        )
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = start + text[start..].find('<')?;
    Some(text[start..end].trim().to_string())
}

/// `unica.check` of a set root warns as an edit does when an extension is
/// above its configuration: the platform does not apply it. `DontUse` is
/// not compared (INV.SOURCE.WRITABLE-PROFILE).
fn compatibility_mode_warnings(invocation: &ActorBoundExecution, at: &str) -> Vec<Value> {
    let Some((set, _)) = at.split_once(':') else {
        return Vec::new();
    };
    compatibility_mode_warnings_of(set, &invocation.admitted_roots())
}

/// The warnings of the root of `set` against the other admitted `roots`.
fn compatibility_mode_warnings_of(
    set: &str,
    roots: &[(
        String,
        unica_source_contract::SourceSetKind,
        std::sync::Arc<unica_source_contract::read::SourceDirectory>,
    )],
) -> Vec<Value> {
    use unica_metadata::configuration::{
        extension_mode_above_base_warning, extension_mode_is_above_base,
    };
    use unica_source_contract::SourceSetKind;
    const MODE: &str = "CompatibilityMode";
    const EXTENSION_MODE: &str = "ConfigurationExtensionCompatibilityMode";
    let Some((_, kind, own)) = roots.iter().find(|(name, _, _)| name == set) else {
        return Vec::new();
    };
    let warning =
        |message: String| json!({"code": "extensionCompatibilityAboveBase", "message": message});
    let mut warnings = Vec::new();
    match kind {
        SourceSetKind::Extension => {
            let Some(extension_mode) = root_mode(own, EXTENSION_MODE) else {
                return Vec::new();
            };
            for (name, kind, root) in roots {
                if *kind != SourceSetKind::Configuration {
                    continue;
                }
                if let Some(base_mode) = root_mode(root, MODE) {
                    if extension_mode_is_above_base(&extension_mode, &base_mode) {
                        warnings.push(warning(extension_mode_above_base_warning(
                            &extension_mode,
                            None,
                            &base_mode,
                            name,
                        )));
                    }
                }
            }
        }
        SourceSetKind::Configuration => {
            let Some(base_mode) = root_mode(own, MODE) else {
                return Vec::new();
            };
            for (name, kind, root) in roots {
                if *kind != SourceSetKind::Extension {
                    continue;
                }
                if let Some(extension_mode) = root_mode(root, EXTENSION_MODE) {
                    if extension_mode_is_above_base(&extension_mode, &base_mode) {
                        warnings.push(warning(extension_mode_above_base_warning(
                            &extension_mode,
                            Some(name),
                            &base_mode,
                            set,
                        )));
                    }
                }
            }
        }
        _ => {}
    }
    warnings
}

#[cfg(test)]
mod compatibility_mode_check_tests {
    use super::compatibility_mode_warnings_of;
    use std::sync::Arc;
    use unica_source_contract::SourceSetKind;

    fn root(
        dir: &std::path::Path,
        properties: &str,
    ) -> Arc<unica_source_contract::read::SourceDirectory> {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("Configuration.xml"),
            format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration><Properties>{properties}</Properties></Configuration></MetaDataObject>"),
        )
        .unwrap();
        Arc::new(
            unica_workspace::read::open_source_directory(&std::fs::canonicalize(dir).unwrap())
                .unwrap(),
        )
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn check_of_a_root_warns_when_an_extension_is_above_its_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let roots = vec![
            (
                "main".to_string(),
                SourceSetKind::Configuration,
                root(&temp.path().join("cf"), "<ConfigurationExtensionCompatibilityMode>Version8_5_1</ConfigurationExtensionCompatibilityMode><CompatibilityMode>Version8_3_24</CompatibilityMode>"),
            ),
            (
                "ext".to_string(),
                SourceSetKind::Extension,
                root(&temp.path().join("ext"), "<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>"),
            ),
            (
                "same".to_string(),
                SourceSetKind::Extension,
                root(&temp.path().join("same"), "<ConfigurationExtensionCompatibilityMode>Version8_3_24</ConfigurationExtensionCompatibilityMode>"),
            ),
        ];
        let messages = |set: &str| {
            compatibility_mode_warnings_of(set, &roots)
                .into_iter()
                .map(|warning| warning["message"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            messages("ext"),
            ["Extension compatibility mode `Version8_3_27` is above compatibility mode `Version8_3_24` of configuration `main`; the platform does not apply such an extension."]
        );
        assert_eq!(
            messages("main"),
            ["Extension compatibility mode `Version8_3_27` of extension `ext` is above compatibility mode `Version8_3_24` of configuration `main`; the platform does not apply such an extension."]
        );
        assert!(messages("same").is_empty());
    }
}
