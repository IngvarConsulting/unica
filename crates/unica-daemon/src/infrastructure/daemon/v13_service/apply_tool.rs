//! `unica.apply`: исполнение операций правки.

use super::*;

impl CanonicalV13ReadService {
    pub(super) fn execute_apply(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        if let Some(token) = invocation.arguments().get("executionToken") {
            if invocation.arguments().len() != 1 || token.as_str().is_none_or(str::is_empty) {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "execution requires only a nonempty executionToken",
                );
            }
            let deadline = unica_work_control::operation_deadline::OperationDeadline::NoDeadline;
            return invocation.execute_saved_apply(token.as_str().unwrap(), cancellation, deadline, |mut result, publication| {
                match publication {
                    Ok(publication) => {
                        if publication.effects().disposition() == ApplyEffectDisposition::Committed
                            && !publication.effects().events().is_empty()
                        {
                            use crate::application::ports::ApplicationPorts;
                            self.ports.notify_committed_changes(
                                invocation.workspace_context(),
                                publication.effects().events(),
                                cancellation,
                                deadline,
                            );
                        }
                        result.summary = "metadata apply published atomically".into();
                        result.rev = Some(publication.rev().to_owned());
                        if let Some(data) = result.data.as_mut() {
                            data["mode"] = json!("published");
                        }
                        result.next.clear();
                        if !publication.cleanup_diagnostics().is_empty() {
                            result.warnings.push(json!({"code": "retained_cleanup_incomplete", "count": publication.cleanup_diagnostics().len(), "message": "published apply left bounded internal recovery cleanup diagnostics"}));
                        }
                        result
                    }
                    Err(error) => error_result(result.at.clone(), apply_publication_error_code(error.kind()), format!("{error}; request a fresh plan with apply(at, ops)")),
                }
            }).unwrap_or_else(saved_apply_execution_error_result);
        }
        if invocation.arguments().contains_key("dryRun")
            || invocation.arguments().contains_key("ifRev")
        {
            return error_result(
                None,
                RefusalCode::BadValue,
                "apply accepts at and ops to prepare a plan, or only executionToken to execute it",
            );
        }
        let source_sets = invocation.admitted_source_set_names();
        // A person's answer to the question about a vendor-support lock comes
        // back as the same call with the door's confirmation mark.
        let call =
            unica_execution_contract::invocation::without_confirmation(invocation.arguments());
        let answered = call.len() != invocation.arguments().len();
        let mut planning_arguments = call.clone();
        planning_arguments.insert("dryRun".into(), json!(true));
        let request = match parse_apply_request(&planning_arguments, &source_sets) {
            Ok(request) => request,
            Err(error) => {
                return error_result(
                    Some(error.location().to_string()),
                    error.code(),
                    error.to_string(),
                )
            }
        };
        // A line selector is a read address. Refuse before write admission
        // creates staging, even if an op overrides the top-level target.
        if let Some(ranged) = std::iter::once(request.at())
            .chain(request.ops().iter().map(|op| op.at()))
            .find(|at| at.line_range().is_some())
        {
            let mut refusal = error_result(Some(ranged.to_string()), RefusalCode::BadValue, "Body line ranges are read-only; replace a whole containing Method or use the module Body");
            if let Ok(source) = invocation.apply_range_refusal_source(ranged) {
                if let Ok(authority) = source.logical_view_read_authority(cancellation) {
                    let range = ranged.line_range().expect("selected ranged address");
                    if let Ok(body) =
                        authority.narrowest_body_for_lines(ranged, range.from, range.to)
                    {
                        let segments = body.segments();
                        let target = if segments.len() >= 2
                            && segments[segments.len() - 2].kind() == NodeKind::Method
                        {
                            body.prefix(segments.len() - 1)
                        } else {
                            body
                        };
                        refusal.next.push(json!({"tool":"unica.view","args":{"at":target.to_string()},"reason":"прочитать целый метод или Body модуля перед правкой"}));
                    }
                }
            }
            if refusal.next.is_empty() {
                let segments = ranged.segments();
                let module_body = if segments.len() >= 2
                    && segments[segments.len() - 2].kind() == NodeKind::Method
                {
                    format!("{}.Body", ranged.prefix(segments.len() - 2))
                } else {
                    ranged.without_line_range().to_string()
                };
                refusal.next.push(json!({"tool":"unica.view","args":{"at":module_body},"reason":"прочитать Body модуля и выбрать семантическую цель правки"}));
            }
            return refusal;
        }
        let (binding, mut admission) = match invocation.admit_apply(&request, cancellation) {
            Ok(admitted) => admitted,
            Err(ApplyAdmissionError::Other(error)) => {
                return error_result(
                    Some(request.at().to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        let traits = crate::domain::apply::source_set_traits_of_source(
            binding.source_kind(),
            binding.source_format(),
            binding.source_profile(),
        );
        let template_format = |template: &QualifiedAddress| {
            crate::infrastructure::native_operations::apply_families::staged_template_format(
                &admission, &binding, template,
            )
        };
        let form_observation = |item: &QualifiedAddress| {
            crate::infrastructure::native_operations::apply_families::staged_form_observation(
                &admission, &binding, item,
            )
        };
        let color = |at: &QualifiedAddress| {
            crate::infrastructure::native_operations::apply_families::staged_style_item_properties_hint(
                &admission, &binding, at,
            )
        };
        if let Err(refusal) = preflight_apply_operations_observed(
            &request,
            traits,
            &template_format,
            &form_observation,
            &color,
        ) {
            return with_unknown_argument_contract(*refusal, &request, traits);
        }
        if let Err(refusal) =
            validate_style_item_apply_capabilities(&request, traits, &admission, &binding)
        {
            return with_unknown_argument_contract(*refusal, &request, traits);
        }
        admission.bind_request_identity(
            &request.at().to_string(),
            request.ops().iter().map(|operation| {
                (
                    operation.name(),
                    operation.at().to_string(),
                    operation.args(),
                )
            }),
        );
        // Each refusal the person answered is lifted in the same plan; the
        // next one asks its own question, and the call keeps every answer.
        use crate::infrastructure::native_operations::apply_families as families;
        let source_set = request.at().source_set().to_string();
        let mut mode_raise: Option<(
            unica_source_contract::proposal::ReadOnlyMode,
            crate::domain::apply::ApplyRequest,
        )> = None;
        let mut support: Option<(
            unica_source_contract::proposal::SupportLock,
            Vec<crate::domain::apply::ApplyOp>,
        )> = None;
        let mut remove_support = false;
        let planned = loop {
            let planned = families::plan_hidden_v13_apply_with_person_choices(
                &request,
                &binding,
                &admission,
                families::PersonChosenChanges {
                    mode_raise: mode_raise.as_ref().map(|(read_only, request)| {
                        families::PersonChosenModeRaise { read_only, request }
                    }),
                    support: support.as_ref().map(|(lock, operations)| {
                        families::PersonChosenSupportChange { lock, operations }
                    }),
                    remove_support,
                },
            );
            let Err(error) = &planned else {
                break planned;
            };
            if !answered {
                break planned;
            }
            if let Some(read_only) = error.read_only_mode().filter(|_| mode_raise.is_none()) {
                if support_choice::mode_raise_chosen(read_only, &source_set, invocation.arguments())
                {
                    let raise = support_choice::mode_raise_request(read_only, &source_set);
                    match parse_apply_request(&raise, &source_sets) {
                        Ok(raise) => mode_raise = Some((read_only.clone(), raise)),
                        Err(error) => {
                            return error_result(
                                Some(error.location().to_string()),
                                error.code(),
                                error.to_string(),
                            )
                        }
                    }
                    continue;
                }
            }
            if error.support_state_unreadable()
                && !remove_support
                && support_choice::remove_support_chosen(&source_set, invocation.arguments())
            {
                remove_support = true;
                continue;
            }
            if let Some(lock) = error.support_lock().filter(|_| support.is_none()) {
                if let Some(answer) = support_choice::chosen_answer(lock, invocation.arguments()) {
                    let mut operations = Vec::new();
                    for change in support_choice::support_requests(lock, &answer) {
                        match parse_apply_request(&change, &source_sets) {
                            Ok(change) => operations.extend(change.ops().iter().cloned()),
                            Err(error) => {
                                return error_result(
                                    Some(error.location().to_string()),
                                    error.code(),
                                    error.to_string(),
                                )
                            }
                        }
                    }
                    support = Some((lock.clone(), operations));
                    continue;
                }
            }
            break planned;
        };
        let (staged, mut effects) = match planned {
            Err(error)
                if error.kind() == ApplyPlanErrorKind::ProviderUnavailable
                    && error.to_string().contains("not implemented") =>
            {
                return error_result(
                    error.path().map(str::to_string),
                    RefusalCode::UnsupportedOperation,
                    "canonical v0.13 apply operation is not implemented",
                )
            }
            Err(error) => {
                let refusal = apply_plan_error_result(&error);
                let node_exists = |at: &QualifiedAddress| {
                    crate::infrastructure::native_operations::apply_families::staged_node_existence(
                        &admission, &binding, at,
                    )
                };
                let refusal = with_missing_target_parent(refusal, &request, &error, &node_exists);
                let refusal = with_unknown_argument_contract(refusal, &request, traits);
                let mut refusal =
                    with_argument_dictionary(refusal, &request, &error, traits, &node_exists);
                // The question carries the answers the person already gave
                // about this call, so the plan comes out once, with all of them.
                let choice = match (error.read_only_mode(), error.support_lock()) {
                    (Some(read_only), _) => Some(support_choice::mode_choice(
                        read_only,
                        &source_set,
                        invocation.arguments(),
                    )),
                    (None, Some(lock)) => {
                        support_choice::support_choice(lock, invocation.arguments())
                    }
                    (None, None) if error.support_state_unreadable() => Some(
                        support_choice::remove_support_choice(&source_set, invocation.arguments()),
                    ),
                    (None, None) => error.format_reexport().map(|reexport| {
                        support_choice::format_choice(reexport, invocation.arguments())
                    }),
                };
                if let Some(choice) = choice {
                    refusal.human_choice = Some(Box::new(choice));
                }
                return refusal;
            }
            Ok(planned) => planned,
        };
        let borrowing = effects.borrowing().to_vec();
        let plan_details = effects
            .details()
            .map(|(key, rows)| (key, rows.to_vec()))
            .collect::<Vec<_>>();
        let has_changes = !staged.planned_changes().is_empty();
        if !has_changes {
            effects = Default::default();
        }
        let plan_warnings = effects.warnings().to_vec();
        // The answer does not echo the request: only an operation that
        // attached a warning is named, so the answer does not grow with `ops`.
        let operations = effects
            .warned_operations()
            .iter()
            .filter_map(|index| {
                request
                    .ops()
                    .get(*index)
                    .map(|operation| (index, operation))
            })
            .map(|(index, operation)| {
                serde_json::json!({
                    "index": index,
                    "op": operation.name(),
                    "at": operation.at(),
                })
            })
            .collect::<Vec<_>>();
        let plan_hash = apply_plan_hash(&staged, &effects);
        let changed = effects
            .events()
            .iter()
            .map(|event| {
                let at = if event.artifact.contains(':') {
                    event.artifact.clone()
                } else {
                    format!("{}:{}", request.at().source_set(), event.artifact)
                };
                serde_json::json!({
                    "at": at,
                    "event": event.name(),
                })
            })
            .collect::<Vec<_>>();
        let prepared = match admission.prepare_with_effects(staged, effects) {
            Ok(prepared) => prepared,
            Err(error) => {
                return apply_staging_refusal(
                    Some("ops".to_string()),
                    error.kind(),
                    error.to_string(),
                )
            }
        };
        let publication = match invocation.preview_prepared_apply(&prepared) {
            Ok(publication) => publication,
            Err(error) => {
                return error_result(
                    Some(request.at().to_string()),
                    apply_publication_error_code(error.kind()),
                    error.to_string(),
                )
            }
        };
        let disposition = match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "preview",
            ApplyEffectDisposition::Committed => "published",
        };
        let mut result = DomainResult::success(match publication.effects().disposition() {
            ApplyEffectDisposition::Projected => "metadata apply plan prepared without publication",
            ApplyEffectDisposition::Committed => "metadata apply published atomically",
        });
        result.at = Some(request.at().to_string());
        result.data = Some(serde_json::json!({
            "validated": true,
            "mode": disposition,
            "executable": true,
            "operationCount": request.ops().len(),
            "planHash": plan_hash,
            "effects": publication.effects().events().len(),
            "cache": publication.effects().cache(),
        }));
        if !operations.is_empty() {
            result.data.as_mut().expect("apply data exists")["operations"] =
                serde_json::Value::Array(operations);
        }
        if !borrowing.is_empty() {
            result.data.as_mut().expect("apply data exists")["borrowing"] =
                serde_json::json!(borrowing);
        }
        for (key, rows) in plan_details {
            debug_assert!(
                PLAN_DETAIL_KEYS.contains(&key),
                "plan detail section `{}` is not registered",
                key.as_str()
            );
            result.data.as_mut().expect("apply data exists")[key.as_str()] =
                serde_json::Value::Array(rows);
        }
        if has_changes {
            result.changed = changed;
        }
        result.warnings.extend(plan_warnings);
        if !publication.cleanup_diagnostics().is_empty() {
            result.warnings.push(serde_json::json!({
                "code": "retained_cleanup_incomplete",
                "count": publication.cleanup_diagnostics().len(),
                "message": "published apply left bounded internal recovery cleanup diagnostics"
            }));
        }
        result.rev = Some(publication.rev().to_string());
        match invocation.save_prepared_apply(prepared, result.clone()) {
            Ok(token) => {
                result.data.as_mut().expect("apply plan data exists")["executionToken"] =
                    json!(token);
                result
                    .next
                    .push(json!({"tool": "unica.diff", "arguments": {"executionToken": token}}));
                result
                    .next
                    .push(json!({"tool": "unica.apply", "arguments": {"executionToken": token}}));
                result
            }
            Err(error) => error_result(
                result.at.clone(),
                RefusalCode::InvalidState,
                error.to_string(),
            ),
        }
    }
}

/// Аргумент, которого нет в контракте операции на адресованном узле,
/// отклоняется `bad_value`, и `next` ведёт к этому контракту,
/// `<адрес>.Can.<op>` (INV.WIRE.OPERATION-TARGET-ADDRESS). Отказ любого слоя — предварительной
/// проверки, разбора семейства или планировщика — получает один и тот же путь.
pub(in crate::infrastructure::daemon) fn with_unknown_argument_contract(
    refusal: DomainResult,
    request: &crate::domain::apply::ApplyRequest,
    source: unica_source_contract::context::SourceSetTraits,
) -> DomainResult {
    let bad_value = refusal
        .diagnostics
        .first()
        .and_then(|diagnostic| diagnostic["code"].as_str())
        == Some(RefusalCode::BadValue.as_str());
    if refusal.ok || !bad_value {
        return refusal;
    }
    let Some((index, segments)) = refusal.at.as_deref().and_then(argument_place) else {
        return refusal;
    };
    let Some(operation) = request.ops().get(index) else {
        return refusal;
    };
    let Some(site) = crate::domain::apply::operation_site(operation.at(), source) else {
        return refusal;
    };
    let Some(descriptor) =
        crate::domain::apply::operation_registry().lookup_at(operation.name(), &site)
    else {
        return refusal;
    };
    // Ключ из контракта на этом узле — плохое значение, а не лишний
    // аргумент; без подробного контракта их не различить. Слой, уже
    // назвавший продолжение, не дополняется.
    let Some(contract) = descriptor.argument_contract_at(&site) else {
        return refusal;
    };
    if published_path(&contract.schema, &segments) || !refusal.next.is_empty() {
        return refusal;
    }
    let mut refusal = refusal;
    refusal.next.push(json!({
        "tool": "unica.view",
        "args": {"at": format!("{}.Can.{}", operation.at(), descriptor.name())},
        "reason": format!("аргументы операции `{}` на этом узле", descriptor.name()),
    }));
    refusal
}

/// `ops[<i>].args.<путь>` → номер операции и шаги пути: ключи и номера.
fn argument_place(path: &str) -> Option<(usize, Vec<Option<&str>>)> {
    let (index, rest) = path.strip_prefix("ops[")?.split_once(']')?;
    let rest = rest.strip_prefix(".args.")?;
    let mut segments = Vec::new();
    for part in rest.split('.') {
        let (key, indexes) = part
            .split_once('[')
            .map_or((part, ""), |(key, tail)| (key, tail));
        if !key.is_empty() {
            segments.push(Some(key));
        }
        segments.extend(
            indexes
                .split('[')
                .filter(|tail| !tail.is_empty())
                .map(|_| None),
        );
    }
    Some((index.parse().ok()?, segments))
}

/// Путь, который схема контракта публикует: ключ — свойство уровня (любой
/// ветви `oneOf` или `anyOf`) или его псевдоним, номер — элемент `items`.
/// Открытый уровень (`additionalProperties`) и уровень без описания
/// принимают любой путь: лишним ключ признаётся, только если схема его
/// уровня закрыта и ключа не называет.
fn published_path(schema: &Value, segments: &[Option<&str>]) -> bool {
    let Some((step, rest)) = segments.split_first() else {
        return true;
    };
    let levels = schema_branches(schema);
    match step {
        None => {
            let items = levels
                .iter()
                .filter_map(|level| level.get("items"))
                .collect::<Vec<_>>();
            items.is_empty() || items.iter().any(|item| published_path(item, rest))
        }
        Some(key) => {
            if levels.iter().any(|level| {
                level
                    .get("additionalProperties")
                    .is_some_and(|open| open != &Value::Bool(false))
                    || level["x-unica-aliases"]
                        .as_array()
                        .is_some_and(|aliases| aliases.iter().any(|alias| alias == key))
            }) || levels.iter().all(|level| level.get("properties").is_none())
            {
                return true;
            }
            levels
                .iter()
                .filter_map(|level| level["properties"].get(*key))
                .any(|child| published_path(child, rest))
        }
    }
}

/// Уровень схемы вместе с ветвями `oneOf` и `anyOf` на любой глубине.
fn schema_branches(schema: &Value) -> Vec<&Value> {
    let mut levels = vec![schema];
    for choice in ["oneOf", "anyOf"] {
        for branch in schema[choice].as_array().into_iter().flatten() {
            levels.extend(schema_branches(branch));
        }
    }
    levels
}

/// Enum errors may need the full domain after a bounded preview. Keep
/// a planner's more specific continuation and never point to an unknown node.
pub(in crate::infrastructure::daemon) fn with_argument_dictionary(
    result: DomainResult,
    request: &crate::domain::apply::ApplyRequest,
    error: &unica_source_contract::proposal::ApplyPlanError,
    source: unica_source_contract::context::SourceSetTraits,
    node_exists: &dyn Fn(&QualifiedAddress) -> Option<bool>,
) -> DomainResult {
    if error.kind() != ApplyPlanErrorKind::BadValue || !result.next.is_empty() {
        return result;
    }
    let Some((index, tail)) = error.path().and_then(|path| {
        let rest = path.strip_prefix("ops[")?;
        let (index, tail) = rest.split_once(']')?;
        Some((index.parse::<usize>().ok()?, tail))
    }) else {
        return result;
    };
    if tail != ".args" && !tail.starts_with(".args.") && !tail.starts_with(".args[") {
        return result;
    }
    let Some(operation) = request.ops().get(index) else {
        return result;
    };
    let Some(site) = crate::domain::apply::operation_site(operation.at(), source) else {
        return result;
    };
    let Some(descriptor) =
        crate::domain::apply::operation_registry().lookup_at(operation.name(), &site)
    else {
        return result;
    };
    let Some(contract) = descriptor.argument_contract_at(&site) else {
        return result;
    };
    // Reuse the schema at the addressed node to distinguish enum failures
    // from other bad values, such as a duplicate member name.
    let cause = contract.normalize(
        &Value::Object(operation.args().clone()),
        &format!("ops[{index}].args"),
    );
    if !cause.is_err_and(|cause| cause.enum_value && Some(cause.path.as_str()) == error.path())
        || node_exists(operation.at()) != Some(true)
    {
        return result;
    }
    let mut result = result;
    result.next.push(json!({
        "tool":"unica.view",
        "args":{"at":format!("{}.Can.{}", operation.at(), operation.name())},
        "reason":"полный контракт аргументов операции",
    }));
    result
}

#[cfg(test)]
mod tests {
    use super::{argument_place, published_path};
    use serde_json::json;

    #[test]
    fn an_argument_path_names_the_operation_keys_and_items() {
        assert_eq!(
            argument_place("ops[2].args.scope"),
            Some((2, vec![Some("scope")]))
        );
        assert_eq!(
            argument_place("ops[0].args.items[0].into"),
            Some((0, vec![Some("items"), None, Some("into")]))
        );
        assert_eq!(argument_place("ops[0].at"), None);
        assert_eq!(argument_place("ops[x].args.a"), None);
    }

    /// rule: INV.WIRE.OPERATION-TARGET-ADDRESS
    #[test]
    fn a_key_is_unknown_only_where_its_level_is_closed_and_does_not_name_it() {
        let schema = json!({
            "type": "object",
            "properties": {
                "values": {"type": "object", "properties": {"name": {"type": "string"}}},
                "items": {"type": "array", "items": {"properties": {"name": {}}}},
                "meta": {"type": "object", "additionalProperties": true},
            },
            "oneOf": [{"properties": {"scope": {}}}],
            "x-unica-aliases": ["alias"],
        });
        for (path, published) in [
            (vec![Some("values"), Some("name")], true),
            (vec![Some("values"), Some("dataSet")], false),
            (vec![Some("items"), None, Some("name")], true),
            (vec![Some("items"), None, Some("group")], false),
            (vec![Some("meta"), Some("anything")], true),
            (vec![Some("scope")], true),
            (vec![Some("alias")], true),
            (vec![Some("unknown")], false),
        ] {
            assert_eq!(published_path(&schema, &path), published, "{path:?}");
        }
    }
}
