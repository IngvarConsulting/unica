//! `unica.view`: узел, его ветви и граф вызовов.

use super::*;

impl CanonicalV13ReadService {
    pub(super) fn execute_view(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        self.execute_view_arguments(invocation, invocation.arguments(), cancellation)
    }

    pub(super) fn execute_view_arguments(
        &self,
        invocation: &ActorBoundExecution,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let Some(at) = arguments.get("at").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "view requires string argument `at`",
            );
        };
        // `<узел>.Can.<op>` спрашивает контракт одной операции словаря узла:
        // читается сам узел, а ответ — его операция.
        let mut operation = None;
        // `<узел>.Props.<имя>` читает одно свойство узла целиком (#2088):
        // узел проецируется без предела размера, ответ — это свойство.
        let mut property = None;
        let read_at = match QualifiedAddress::parse(at) {
            Ok(address) if address.property_query().is_some() => {
                let (owner, name) = address.property_query().expect("checked above");
                // A property is one value: like any single node it accepts a
                // page size it does not need, but it has no continuation.
                if arguments.contains_key("cursor") {
                    return error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "a node property is one value and has no pages; omit cursor",
                    );
                }
                property = Some(name.to_string());
                owner.to_string()
            }
            Ok(address) => match address.operation_query() {
                Some((owner, Some(op))) => {
                    // A contract is one node: like a source node it accepts a
                    // page size it does not need, but it has no continuation.
                    if arguments.contains_key("cursor") {
                        return error_result(
                            Some(at.to_string()),
                            RefusalCode::BadValue,
                            "an operation contract is one node and has no pages; omit cursor",
                        );
                    }
                    operation = Some(op.to_string());
                    owner.to_string()
                }
                Some((owner, None)) => {
                    let mut refusal = error_result(
                        Some(at.to_string()),
                        RefusalCode::BadValue,
                        "Can names one operation: `<node>.Can.<op>`; the node answer lists its operations in can",
                    );
                    refusal.next.push(json!({
                        "tool": "unica.view",
                        "args": {"at": owner.to_string()},
                        "reason": "операции узла перечислены в его can",
                    }));
                    return refusal;
                }
                None => at.to_string(),
            },
            Err(_) => at.to_string(),
        };
        let mut request = match ViewRequest::new(&read_at) {
            Ok(request) => request,
            Err(error) => return view_error_result(Some(at.to_string()), error),
        };
        if let Some(limit) = arguments.get("limit") {
            let Some(limit) = bounded_usize(limit) else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    "view limit must be a positive integer",
                );
            };
            request = match request.with_limit(limit) {
                Ok(request) => request,
                Err(error) => return view_error_result(Some(at.to_string()), error),
            };
        }
        if let Some(cursor) = arguments.get("cursor") {
            let Some(cursor) = cursor.as_str() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    "view cursor must be a string",
                );
            };
            request = request.with_cursor(cursor.to_string());
        }
        let address = match QualifiedAddress::parse(&read_at) {
            Ok(address) => address,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::BadValue,
                    error.to_string(),
                )
            }
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        let Some(source) = sources
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
        else {
            return error_result(
                Some(at.to_string()),
                RefusalCode::ProviderUnavailable,
                "view source set was not admitted by the workspace actor",
            );
        };
        #[cfg(test)]
        if request.cursor().is_none() {
            observe_fresh_view_capability_for_test(&address.to_string(), || {
                source.revision_identity()
            });
        }
        let authority = match source.logical_view_read_authority(cancellation) {
            Ok(authority) => authority,
            Err(error) => {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::ProviderUnavailable,
                    error,
                )
            }
        };
        // Ветвь графа вызовов не живёт в дереве исходников, поэтому читатель о
        // ней не знает: страницу собирает служба, а существование владельца
        // доказывает тот же читатель обычным чтением узла метода.
        if let Some(direction) = branch_direction(&address) {
            let Some(owner) = branch_owner(&address) else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::NotFound,
                    "call graph branch has no owning method",
                );
            };
            if operation.is_some() {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::UnsupportedSection,
                    "a call graph branch has no operation dictionary",
                );
            }
            // Ветвь метода, у которого графа нет (метод HTTP-сервиса, модуль
            // непокрытой роли), отказывает по адресу, до чтения владельца и заказа поставки: ни скачивание,
            // ни наличие движка не меняют этого ответа.
            if let Err(message) = call_graph_support(&owner) {
                return error_result(Some(at.to_string()), RefusalCode::BadValue, message);
            }
            let service = ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors));
            if request.cursor().is_some() {
                return enforce_view_result_limit(
                    service.view_saved_collection(request, &owner),
                    at,
                );
            }
            let owner_result = service.view(match ViewRequest::new(&owner.to_string()) {
                Ok(request) => request,
                Err(error) => return view_error_result(Some(owner.to_string()), error),
            });
            if !owner_result.ok {
                return owner_result;
            }
            let Some(expected_revision) = owner_result.rev.as_deref() else {
                return error_result(
                    Some(at.to_string()),
                    RefusalCode::InvalidResult,
                    "owning method has no source revision",
                );
            };
            // Ветвь — единственный вход в граф, и анализатор, которого нет на
            // машине, она поставляет этим же вызовом, как `check` и `search`:
            // пока поставка идёт, ответ просит повтор, провал поставки — её
            // типизированный отказ, а движок, которого и после поставки нет,
            // — общий отказ «нет поставляемого движка» (#549), а не граф
            // «недоступен».
            // Тестовый поставщик графа заменяет анализатор: заказывать его
            // поставку тогда незачем, и тест не зависит от окружения.
            #[cfg(not(test))]
            let analyzer_engine = true;
            #[cfg(test)]
            let analyzer_engine =
                crate::infrastructure::application_ports::CODE_INTELLIGENCE_PROVIDER_OVERRIDE
                    .with(|slot| slot.borrow().is_none());
            let cwd = &invocation.workspace_context().cwd;
            if analyzer_engine {
                if let Err(refusal) = acquire_engine(
                    invocation,
                    cwd,
                    BSL_ANALYZER_ENGINE,
                    Some(at.to_string()),
                    cancellation,
                ) {
                    return *refusal;
                }
            }
            return match self.call_graph_summary(
                invocation,
                &owner,
                CALL_GRAPH_PAGE_LIMIT,
                cancellation,
            ) {
                Ok(initial) => {
                    let (summary, directory) = match complete_branch(initial, direction, |limit| {
                        self.call_graph_summary(invocation, &owner, limit, cancellation)
                    }) {
                        Ok(answer) => answer,
                        Err(CompleteBranchError::CountTooLarge(message)) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::ResultTooLarge,
                                message,
                            )
                        }
                        Err(CompleteBranchError::Fetch(refusal)) => return *refusal,
                        Err(CompleteBranchError::Changed) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::ConcurrentChange,
                                "call graph changed while collecting its page; retry",
                            )
                        }
                        Err(CompleteBranchError::Incomplete) => {
                            return error_result(
                                Some(at.to_string()),
                                RefusalCode::InvalidResult,
                                "call graph provider did not return the complete branch",
                            )
                        }
                    };
                    // Анализатор не ответил на ветвь: движок, не прошедший
                    // проверку поставки или не запустившийся, — тот же отказ
                    // «нет движка», что на всех маршрутах (#549).
                    if analyzer_engine
                        && summary.result(direction).state == CallGraphState::Unavailable
                    {
                        if let Some(refusal) = engine_failure_refusal(
                            cwd,
                            BSL_ANALYZER_ENGINE,
                            Some(at.to_string()),
                            cancellation,
                        ) {
                            return refusal;
                        }
                    }
                    if summary.result(direction).state != CallGraphState::Ready {
                        return enforce_view_result_limit(
                            unready_branch_result(&address, direction, &summary),
                            at,
                        );
                    }
                    // Сосед, которого анализатор называет файлом, получает адрес
                    // через раскладку: файл модуля → дескриптор → узел → роль
                    // модуля. Раскладка строится один раз и только когда такой
                    // сосед есть; кого раскладка не размещает, тот остаётся в
                    // `limits`.
                    let directory = match directory {
                        Some(directory) => Some(directory),
                        None if summary.names_a_peer_by_file(direction) => {
                            match self.layout_directory(
                                invocation,
                                Some(owner.to_string()),
                                owner.source_set(),
                                cancellation,
                            ) {
                                Ok(directory) => Some(directory),
                                Err(refusal) => return *refusal,
                            }
                        }
                        None => None,
                    };
                    let source_set = owner.source_set();
                    let data = branch_collection(&address, direction, &summary, |path| {
                        let directory = directory.as_ref()?;
                        let (placed, role) = placed_for_module_file(path)?;
                        let entry = directory.locate_path(&placed)?;
                        let entry = entry.owner;
                        if entry.at().split(':').next() != Some(source_set) {
                            return None;
                        }
                        QualifiedAddress::parse(&format!("{}.Module.{role}", entry.at())).ok()
                    });
                    let mut answer = service.view_preloaded_collection(
                        request,
                        &owner,
                        expected_revision,
                        Some(data),
                    );
                    if let Some(reason) = summary.reason {
                        answer.warnings.push(json!({"callGraph": reason}));
                    }
                    enforce_view_result_limit(answer, at)
                }
                Err(refusal) => *refusal,
            };
        }
        let can_context = crate::domain::apply::source_set_traits_for_read(
            source.source_kind(),
            authority.platform_profile(),
        );
        let service = ViewService::with_shared_cursors(authority, Arc::clone(&self.cursors));
        let enrich_and_project = |data: &Value| -> Result<Value, ViewError> {
            let mut data = data.clone();
            if source.source_kind() == unica_source_contract::SourceSetKind::Extension
                && data
                    .get("props")
                    .is_some_and(|props| props.get("belonging").is_none())
            {
                let borrowing = source
                    .logical_view_read_authority(cancellation)
                    .map_err(|message| ViewError::detailed(RefusalDetail::ProviderAbsent, message))?
                    .object_borrowing(&address)?;
                if let Some(borrowing) = borrowing {
                    if let Some(props) = data.get_mut("props").and_then(Value::as_object_mut) {
                        props.extend(crate::infrastructure::v13_read_projection::borrowing_props(
                            &borrowing,
                        ));
                    }
                }
            }
            self.resolve_borrowed_parent(invocation, &address, &mut data, cancellation);
            // Узел метода называет ветви графа, не спрашивая анализатор: граф
            // читается только адресом ветви.
            if let Some(node) = data.as_object_mut() {
                declare_method_branches(node);
            }
            // Read failures are not evidence that this Color profile is absent.
            // Resolve the fallible observation before passing a pure lookup to Can.
            let observed_color = if address.segments().len() == 1
                && address.segments()[0].kind() == NodeKind::StyleItem
                && address.segments()[0].name().is_some()
            {
                source
                    .logical_view_read_authority(cancellation)
                    .map_err(|message| ViewError::detailed(RefusalDetail::ProviderAbsent, message))?
                    .style_item_properties_observation(&address)?
            } else {
                None
            };
            let color = |at: &QualifiedAddress| {
                if at == &address {
                    observed_color
                } else {
                    None
                }
            };
            match operation.as_deref() {
                Some(op) => {
                    let owner_at = data
                        .get("at")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // Роль элемента-предка читается его узлом: маршрут снятой
                    // операции не ведёт в поле, куда элемент не добавить.
                    let ancestor_role = |item: &QualifiedAddress| {
                        if item.segments().last()?.kind() != NodeKind::Item {
                            return None;
                        }
                        let read = service.view(ViewRequest::new(&item.to_string()).ok()?);
                        read.data?
                            .get("props")?
                            .get("tag")?
                            .as_str()
                            .map(unica_forms::FormItemRole::from_tag)
                    };
                    operation_node_with(&data, op, &owner_at, can_context, &ancestor_role, &color)
                        .map_err(|error| {
                            let mut refused = ViewError::new(error.code(), error.to_string());
                            // Снятая операция ведёт к контракту заменившей (#1465).
                            refused.set_next(match error.next() {
                                Some(contract) => json!({
                                    "tool": "unica.view",
                                    "args": {"at": contract},
                                    "reason": "контракт операции, заменившей снятую",
                                }),
                                None => json!({
                                    "tool": "unica.view",
                                    "args": {"at": owner_at},
                                    "reason": "операции, применимые к этому узлу",
                                }),
                            });
                            refused
                        })
                }
                None => {
                    attach_can_with_color(&mut data, can_context, &color);
                    Ok(data)
                }
            }
        };
        let mut result = match property.as_deref() {
            Some(name) => property_view(
                crate::infrastructure::v13_read_projection::with_whole_prop(name, || {
                    service.view_projected(request, &enrich_and_project)
                }),
                at,
                name,
            ),
            None => service.view_projected(request, &enrich_and_project),
        };
        // Ответ операции адресован ею самой, а не узлом, который читался.
        if let Some(op) = operation.as_deref() {
            let owner = result.at.clone();
            match (result.ok, owner) {
                (true, Some(owner)) => {
                    let can_at = format!("{owner}.Can.{op}");
                    if let Some(data) = result.data.as_mut().and_then(Value::as_object_mut) {
                        data.insert("at".to_string(), Value::String(can_at.clone()));
                    }
                    result.at = Some(can_at);
                }
                _ => result.at = Some(at.to_string()),
            }
        }
        enforce_view_result_limit(result, at)
    }

    /// Сводка графа вызовов для одного метода.
    ///
    /// Личность узла строится из адреса; модуль формы или команды анализатор
    /// называет файлом, и путь к нему даёт раскладка — та же, которой живёт
    /// `resolve`. Раскладка строится только для таких адресов и возвращается
    /// вызывающему, чтобы соседей, названных файлом, не переводить второй
    /// сборкой.
    pub(super) fn call_graph_summary(
        &self,
        invocation: &ActorBoundExecution,
        method_at: &QualifiedAddress,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<(CallGraphSummary, Option<FindIndex>), Box<DomainResult>> {
        let mut directory = None;
        let identity = match CallGraphIdentity::from_address(method_at, None) {
            Err(CallGraphIdentityError::PathRequired) => {
                let built = self.layout_directory(
                    invocation,
                    Some(method_at.to_string()),
                    method_at.source_set(),
                    cancellation,
                )?;
                let owner = owning_metadata_address(method_at);
                let path = built
                    .locate_address(&owner)
                    .and_then(|entry| module_file_for_placed(entry.kind(), entry.placed_path()?))
                    .ok_or_else(|| {
                        Box::new(error_result(
                            Some(method_at.to_string()),
                            RefusalCode::NotFound,
                            format!(
                                "the source layout does not place the module file of `{owner}`"
                            ),
                        ))
                    })?;
                let identity = CallGraphIdentity::from_address(method_at, Some(&path));
                directory = Some(built);
                identity
            }
            other => other,
        };
        let identity = identity.map_err(|error| {
            Box::new(error_result(
                Some(method_at.to_string()),
                RefusalCode::BadValue,
                match error {
                    CallGraphIdentityError::PathRequired => {
                        "the analyzer names this module by file, and its path could not be resolved"
                            .to_string()
                    }
                    CallGraphIdentityError::UnsupportedRole(role) => {
                        format!("call graph does not cover module role `{role}` yet")
                    }
                    CallGraphIdentityError::NotAMethod => {
                        "call graph answers for a method node only".to_string()
                    }
                    CallGraphIdentityError::Malformed(detail) => detail,
                },
            ))
        })?;
        let workspace = invocation.workspace_context();
        let operational = crate::infrastructure::operational_config::load_operational_config(
            &workspace.workspace_root,
        )
        .map_err(|diagnostic| {
            Box::new(error_result(
                Some(method_at.to_string()),
                RefusalCode::InvalidState,
                format!("operational config is unreadable: {diagnostic}"),
            ))
        })?;
        let summary = fetch_summary(
            self.ports.as_ref(),
            workspace,
            method_at.source_set(),
            &identity,
            // Первая попытка берёт не более 50 соседей. Ветвь с большим
            // счётом повторяет запрос для полного снимка до публикации курсора.
            limit,
            operational.code_intelligence().provider_read_timeout(),
            cancellation,
        )
        .map_err(|error| {
            let (code, message) = match error {
                CallGraphFetchError::Provider(message) => {
                    (RefusalCode::ProviderUnavailable, message)
                }
                CallGraphFetchError::Changed => (
                    RefusalCode::ConcurrentChange,
                    "call graph changed between directions; retry the question".to_string(),
                ),
            };
            Box::new(error_result(Some(method_at.to_string()), code, message))
        })?;
        Ok((summary, directory))
    }

    pub(super) fn resolve_borrowed_parent(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        data: &mut Value,
        cancellation: &CancellationToken,
    ) {
        let Some(data) = data.as_object_mut() else {
            return;
        };
        let Some(owner) = address
            .segments()
            .first()
            .filter(|_| address.segments().len() == 1)
        else {
            return;
        };
        let kind = owner.kind().as_str();
        let Some(props) = data.get_mut("props").and_then(Value::as_object_mut) else {
            return;
        };
        let Some(parent_id) = props
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let mut matches = Vec::new();
        let sources = match invocation
            .admit_borrowing_parent_sources(cancellation)
            .and_then(|()| {
                invocation.read_sources().map(|sources| {
                    sources
                        .into_iter()
                        .filter(|source| {
                            source.source_kind()
                                == unica_source_contract::SourceSetKind::Configuration
                        })
                        .collect::<Vec<_>>()
                })
            }) {
            Ok(sources) if !sources.is_empty() => sources,
            _ => {
                props.insert("parentStatus".into(), json!("unavailable"));
                return;
            }
        };
        for source in sources {
            let candidate = source
                .logical_view_read_authority(cancellation)
                .map_err(|_| ())
                .and_then(|authority| {
                    authority
                        .borrowed_parent_matches(&parent_id, kind)
                        .map_err(|_| ())
                });
            match candidate {
                Ok(found) => matches.extend(found),
                Err(_) => {
                    props.insert("parentStatus".into(), json!("unavailable"));
                    return;
                }
            }
        }
        let status = match matches.as_slice() {
            [parent] => {
                props.insert("extends".into(), json!(parent));
                "resolved"
            }
            [] => "not_found",
            _ => "ambiguous",
        };
        props.insert("parentStatus".into(), json!(status));
    }

    /// Справочник раскладки одного допущенного набора — тот же, что строит
    /// `resolve`: адрес ↔ место в исходниках.
    pub(super) fn layout_directory(
        &self,
        invocation: &ActorBoundExecution,
        at: Option<String>,
        source_set: &str,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, Box<DomainResult>> {
        let source = admitted_read_source(
            invocation.read_sources(),
            |source| source.source_set_name(),
            at.as_deref(),
            source_set,
        )?;
        let root = source.retained_root();
        let layout = vec![LayoutFindSource::new(
            source.source_set_name(),
            source.source_kind(),
            root.as_ref(),
        )];
        self.find_builder
            .build(&layout, source.deadline(), cancellation)
            .map_err(|error| Box::new(find_build_error_result(at, error)))
    }
}
