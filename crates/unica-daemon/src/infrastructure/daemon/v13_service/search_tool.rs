//! `unica.search`: текст, имена и роли.

use super::*;

impl CanonicalV13ReadService {
    pub(super) fn execute_search(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        let Some(query) = arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                None,
                RefusalCode::BadValue,
                "search requires string argument `query`",
            );
        };
        if query.trim().is_empty() {
            return error_result(
                None,
                RefusalCode::BadValue,
                "search query must not be blank",
            );
        }
        // Корпус выбирает, где искать, а не как: имена метаданных и текст
        // BSL — это один вопрос над разными сводами. Умолчание оставлено
        // текстовым, потому что таким `search` был до появления второго
        // корпуса.
        let corpus = match arguments.get("corpus") {
            None | Some(Value::String(_)) => match arguments
                .get("corpus")
                .and_then(Value::as_str)
                .unwrap_or("text")
            {
                "text" => SearchCorpus::Text,
                "names" => SearchCorpus::Names,
                other => {
                    let mut refusal = error_result(
                        None,
                        RefusalCode::BadValue,
                        format!("search corpus `{other}` is unknown; use `text` or `names`"),
                    );
                    refusal.next.push(serde_json::json!({
                        "tool": "unica.search",
                        "args": {"query": query, "corpus": "names"},
                        "reason": "поиск по именам и синонимам метаданных",
                    }));
                    return refusal;
                }
            },
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search corpus must be a string",
                )
            }
        };
        if corpus == SearchCorpus::Names {
            if arguments.contains_key("role") {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "name search does not accept a provider role",
                );
            }
            return self.execute_search_names(invocation, query, arguments, cancellation);
        }
        // Роль выбирает, чем искать по тексту: точным совпадением своими
        // силами или провайдером — символьным индексом, смысловым поиском.
        // Без роли поиск остаётся буквальным, каким был.
        if let Some(role) = arguments.get("role") {
            let Some(role) = role.as_str() else {
                return error_result(None, RefusalCode::BadValue, "search role must be a string");
            };
            return self.execute_search_role(invocation, query, role, arguments, cancellation);
        }
        let flag = |name: &str| match arguments.get(name) {
            None | Some(Value::Bool(false)) => Ok(false),
            Some(Value::Bool(true)) => Ok(true),
            Some(_) => Err(format!("search {name} must be a boolean")),
        };
        let (regex, ignore_case) = match (flag("regex"), flag("ignoreCase")) {
            (Ok(regex), Ok(ignore_case)) => (regex, ignore_case),
            (Err(error), _) | (_, Err(error)) => {
                return error_result(None, RefusalCode::BadValue, error)
            }
        };
        let text_query = match TextQuery::new(query, regex, ignore_case) {
            Ok(text_query) => text_query,
            Err(error) => return error_result(None, RefusalCode::BadValue, error),
        };
        let limit = match arguments.get("limit") {
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "search limit must be an integer from 1 through 50",
                    )
                }
            },
            None => 20,
        };
        let scope = match arguments.get("scope") {
            None => None,
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(address) => Some(address),
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
        };
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let selected = sources
            .iter()
            .filter(|source| {
                scope
                    .as_ref()
                    .is_none_or(|scope| source.source_set_name() == scope.source_set())
            })
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return error_result(
                scope.map(|scope| scope.to_string()),
                RefusalCode::NotFound,
                "search scope does not name an admitted source set",
            );
        }
        let route = match scope.as_ref() {
            None => None,
            Some(scope) => match self.admit_text_scope(scope, selected[0], cancellation) {
                Ok(route) => Some(route),
                Err(refusal) => return *refusal,
            },
        };
        let mode = text_query.mode;
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope.as_ref().map(ToString::to_string),
            mode: text_search_binding_mode(mode, ignore_case),
            kind: None,
            source_sets: selected
                .iter()
                .map(|source| source.source_set_name().to_owned())
                .collect(),
            result_fingerprint: None,
            page_limit: limit,
        };
        let cursor_token = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        let (offset, continuation) = match cursor_token {
            None => (0, TextContinuation::Start),
            Some(token) => match self.search_cursors.resume_position(token, &binding) {
                Ok((offset, resume)) if !resume.file.is_empty() => {
                    (offset, TextContinuation::Resume(resume))
                }
                // Every continuation names the file it re-enters.
                Ok(_) => return SearchCursorError::Stale.into_result(None),
                Err(error) => return error.into_result(None),
            },
        };
        let text_sources = selected
            .iter()
            .map(|source| {
                let root = source.retained_root().path().to_path_buf();
                TextSource {
                    source,
                    scope: scope.as_ref().map_or_else(
                        || format!("{}:Configuration", source.source_set_name()),
                        ToString::to_string,
                    ),
                    prefix: route.as_ref().and_then(|route| route.prefix.clone()),
                    external_binding: route
                        .as_ref()
                        .and_then(|route| route.external_binding.clone()),
                    // A nested source set owns its files; its parent skips them.
                    excluded: sources
                        .iter()
                        .filter_map(|other| {
                            let other = other.retained_root();
                            let nested = other.path().strip_prefix(&root).ok()?;
                            (!nested.as_os_str().is_empty())
                                .then(|| nested.to_string_lossy().replace('\\', "/"))
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        // Git reads a first page fastest; a continuation re-enters its file
        // through the walk, which does not read the delivered part again.
        let engine = match continuation {
            TextContinuation::Start => choose_engine(&text_query, &text_sources, cancellation),
            TextContinuation::Resume(_) => TextEngine::Walk,
        };
        #[cfg(test)]
        let engine = self
            .text_engine
            .filter(|_| text_query.candidates.is_some() && continuation == TextContinuation::Start)
            .unwrap_or(engine);
        // The first page counts the whole corpus and so never ends by time:
        // a long one becomes a Task. A continuation ends at a share of the
        // direct answer's budget.
        let count_totals = continuation == TextContinuation::Start;
        let walk_budget = {
            let remaining =
                invocation.remaining_response_budget() / WALK_PAGE_RESPONSE_BUDGET_DIVISOR;
            (!count_totals && !remaining.is_zero()).then_some(remaining)
        };
        #[cfg(test)]
        let walk_budget = self.walk_budget.filter(|_| !count_totals).or(walk_budget);
        let request = TextPageRequest {
            query: &text_query,
            sources: text_sources,
            module: route
                .as_ref()
                .and_then(|route| route.module.as_ref())
                .map(ToString::to_string),
            wanted: limit + 1,
            engine,
            continuation,
            walk_budget,
            count_totals,
            #[cfg(test)]
            cancel_past_page: self.cancel_past_page,
        };
        let scan = match scan_text_page(&request, cancellation) {
            Ok(scan) => scan,
            Err(TextSearchError::Interrupted(error)) => return view_error_result(None, error),
            Err(TextSearchError::Engine(error)) => {
                return error_result(None, RefusalCode::ProviderUnavailable, error)
            }
        };
        let stored = match cursor_token {
            None => None,
            Some(token) => {
                // The traversal must still reach the file the cursor
                // re-enters and the matches it delivered there.
                let Some(passed) = scan.passed.clone() else {
                    return SearchCursorError::Stale.into_result(None);
                };
                let mut answered = binding.clone();
                answered.result_fingerprint = Some(passed);
                match self.search_cursors.read(token, &answered) {
                    Ok(stored) => Some(stored),
                    Err(error) => return error.into_result(None),
                }
            }
        };
        self.text_search_page(&sources, scan, binding, stored, offset, mode, cancellation)
    }

    /// Checks a text-search scope against the admitted source set before
    /// any source is read, and routes it to a subtree and a module.
    pub(super) fn admit_text_scope(
        &self,
        scope: &QualifiedAddress,
        source: &super::super::server::ActorReadSourceCapability,
        cancellation: &CancellationToken,
    ) -> Result<SearchScopeRoute, Box<DomainResult>> {
        let refuse = |result: DomainResult| Box::new(result);
        let mut route = search_scope_route(scope, source.source_kind()).map_err(|error| {
            refuse(error_result(
                Some(scope.to_string()),
                error.code(),
                error.to_string(),
            ))
        })?;
        // A module is admitted through its owner: the registered object, or
        // the configuration for its own application modules.
        let owner = match route.module {
            None => scope.clone(),
            Some(_) => {
                let first = &scope.segments()[0];
                let owner = match first.name() {
                    Some(name) if first.kind() != NodeKind::Module => {
                        format!("{}:{}.{name}", scope.source_set(), first.kind().as_str())
                    }
                    _ => format!("{}:Configuration", scope.source_set()),
                };
                QualifiedAddress::parse(&owner).expect("a module owner is a canonical address")
            }
        };
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| {
                refuse(error_result_detailed(
                    Some(scope.to_string()),
                    RefusalDetail::ProviderAbsent,
                    error,
                ))
            })?;
        let snapshot = authority
            .snapshot(&owner)
            .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
        authority
            .validate_search_scope_owner(&owner, &snapshot)
            .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
        if matches!(
            source.source_kind(),
            unica_source_contract::SourceSetKind::ExternalProcessor
                | unica_source_contract::SourceSetKind::ExternalReport
        ) {
            if let Some(name) = owner.segments()[0].name() {
                let descriptor = authority
                    .existing_export_path(&owner, None)
                    .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
                let binding =
                    unica_source_designer::external_artifact::ExternalArtifactBinding::new(
                        source.source_kind(),
                        name,
                        std::path::Path::new(&descriptor),
                    )
                    .map_err(|error| {
                        refuse(error_result(
                            Some(scope.to_string()),
                            RefusalCode::InvalidSource,
                            error,
                        ))
                    })?;
                route.prefix = Some(binding.payload().to_string_lossy().replace('\\', "/"));
                route.external_binding = Some(binding);
            }
        }
        if owner.segments()[0].kind() == NodeKind::Configuration
            || owner.segments()[0].name().is_none()
        {
            // The root and an unnamed kind branch both depend on the
            // configuration descriptor, but neither needs its projected
            // child collection before a text-search page is returned.
            let root_at = QualifiedAddress::parse(&format!("{}:Configuration", scope.source_set()))
                .expect("Configuration is a canonical root address");
            let descriptor = authority
                .identity_export_path(&root_at)
                .map_err(|error| refuse(view_error_result(Some(scope.to_string()), error)))?;
            if let Some(descriptor) = descriptor {
                if let Err(error) = source
                    .retained_root()
                    .read_relative_regular_prefix(std::path::Path::new(&descriptor), 1)
                {
                    let message = format!("search scope descriptor is unavailable: {error}");
                    return Err(refuse(if error.kind() == std::io::ErrorKind::NotFound {
                        error_result(Some(scope.to_string()), RefusalCode::InvalidState, message)
                    } else {
                        error_result_detailed(
                            Some(scope.to_string()),
                            RefusalDetail::SourceUnreadable,
                            message,
                        )
                    }));
                }
            }
        }
        Ok(route)
    }

    /// One page of a text-search scan: method-narrowed addresses, the byte
    /// and count bounds, and the continuation after the last delivered hit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text_search_page(
        &self,
        sources: &[super::super::server::ActorReadSourceCapability],
        scan: TextScan,
        mut binding: SearchCursorBinding,
        stored: Option<StoredSearchCursor>,
        offset: usize,
        mode: &str,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        use crate::application::v13::view::PREFERRED_PAGE_BYTES;
        use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

        let limit = binding.page_limit;
        let mut values = scan
            .hits
            .iter()
            .take(limit)
            .map(|hit| hit.value.clone())
            .collect::<Vec<_>>();
        // A page retains its hits, not every full module projection: each
        // module is parsed once and released before the next one.
        let mut groups = std::collections::BTreeMap::<(String, String), Vec<usize>>::new();
        for (index, hit) in scan.hits.iter().take(limit).enumerate() {
            if let Some(at) = hit.value.get("at").and_then(Value::as_str) {
                groups
                    .entry((hit.source_set.clone(), at.to_owned()))
                    .or_default()
                    .push(index);
            }
        }
        for ((source_set, raw), indices) in groups {
            let Ok(module) = QualifiedAddress::parse(&raw) else {
                continue;
            };
            let Some(source) = sources
                .iter()
                .find(|source| source.source_set_name() == source_set)
            else {
                continue;
            };
            let Ok(authority) = source.logical_view_read_authority(cancellation) else {
                continue;
            };
            let authority = match scan.hits[indices[0]].external_binding.as_ref() {
                Some(proof) => {
                    if let Err(error) = proof.validate(source) {
                        return view_error_result(Some(module.to_string()), error);
                    }
                    authority.with_external_point_binding(proof.binding.clone())
                }
                None => authority,
            };
            for index in indices {
                let hit = &mut values[index];
                let Some(line) = hit
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| usize::try_from(line).ok())
                else {
                    continue;
                };
                let body = match authority.narrowest_body_for_lines(&module, line, line) {
                    Ok(body) => body.to_string(),
                    Err(error) => {
                        if matches!(
                            error.code(),
                            RefusalCode::Cancelled
                                | RefusalCode::DeadlineExceeded
                                | RefusalCode::ConcurrentChange
                        ) {
                            return view_error_result(Some(module.to_string()), error);
                        }
                        hit["limits"] = json!(["Method boundaries could not be established; this address opens the module Body line"]);
                        format!("{module}.Body")
                    }
                };
                hit["at"] = json!(format!("{body}#L{line}"));
            }
        }
        let scan_complete = scan.end == TextScanEnd::Complete;
        let state = if scan.uncovered > 0 {
            "partial"
        } else if scan_complete {
            "completed"
        } else {
            "in progress"
        };
        let summary = format!("{mode} BSL search {state}");
        let mut extra_data = Map::new();
        extra_data.insert("dataFreshness".into(), json!("unknown"));
        extra_data.insert("pageConsistency".into(), json!("live"));
        extra_data.insert(
            "fileCoverage".to_owned(),
            json!({
                "complete": scan_complete && scan.uncovered == 0,
                "scanComplete": scan_complete,
                "uncovered": scan.uncovered,
                "uncoveredIsLowerBound": !scan_complete || scan.uncovered_lower_bound,
                "detailsTruncated": scan.uncovered > scan.details.len(),
                "details": scan.details,
            }),
        );
        // The volume of the whole corpus travels with the first page only.
        if let Some(totals) = &scan.totals {
            extra_data.insert(
                "totals".to_owned(),
                totals.to_value(MAX_TEXT_TOTALS_LEADING_MODULES),
            );
        }
        // A cursor also carries the path it re-enters.
        let resume_bytes = scan
            .hits
            .iter()
            .map(|hit| &hit.next.0)
            .chain(scan.time_next.as_ref().map(|next| &next.0))
            .map(|next| match next {
                TextContinuation::Resume(resume) => 2 * (16 + resume.file.len()),
                TextContinuation::Start => 0,
            })
            .max()
            .unwrap_or(0);
        let measured_bytes = |items: &[Value]| {
            search_page_probe_bytes(mode, items, &summary, &extra_data, None) + resume_bytes
        };
        if measured_bytes(&[]) > MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "search page metadata exceeds the transport limit",
            );
        }
        let mut page = Vec::new();
        let mut byte_stop = false;
        for item in &values {
            let mut candidate = page.clone();
            candidate.push(item.clone());
            let bytes = measured_bytes(&candidate);
            if bytes > MAX_CANONICAL_RESULT_BYTES {
                if page.is_empty() {
                    return error_result(
                        None,
                        RefusalCode::ResultTooLarge,
                        "one search result exceeds the transport limit",
                    );
                }
                byte_stop = true;
                break;
            }
            if bytes > PREFERRED_PAGE_BYTES && !page.is_empty() {
                byte_stop = true;
                break;
            }
            page.push(item.clone());
        }
        let consumed = page.len();
        let next = if consumed < scan.hits.len() {
            Some(scan.hits[consumed - 1].next.clone())
        } else if scan.end == TextScanEnd::Time {
            scan.time_next.clone()
        } else {
            None
        };
        let stopped_by = match &next {
            None => "complete",
            Some(_) if byte_stop => "bytes",
            Some(_) if consumed < scan.hits.len() => "limit",
            Some(_) => "deadline",
        };
        let mut result = DomainResult::success(summary);
        result.data = Some(search_page_data(mode, &page, &extra_data));
        result.page = Some(json!({"stoppedBy": stopped_by}));
        if let Some((continuation, fingerprint)) = next {
            let next_offset = match SearchCursorStore::next_offset(offset, consumed) {
                Ok(offset) => offset,
                Err(error) => return error.into_result(None),
            };
            let resume = match continuation {
                TextContinuation::Resume(resume) => resume,
                TextContinuation::Start => SearchResume::START,
            };
            binding.result_fingerprint = Some(fingerprint);
            let issued = match stored {
                Some(stored) => {
                    self.search_cursors
                        .insert_next_answer(&stored, &binding, next_offset, &resume)
                }
                None => self
                    .search_cursors
                    .insert_first_resumable(binding, next_offset, &resume),
            };
            match issued {
                Ok(token) => result.cursor = Some(token),
                Err(error) => return error.into_result(None),
            }
        }
        result
    }

    pub(super) fn search_page_with_data(
        &self,
        matches: Vec<Value>,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        summary: String,
        extra_data: Map<String, Value>,
    ) -> DomainResult {
        self.search_page_keyed(
            matches,
            None,
            &SearchCursorStore::token_probe(),
            cursor,
            binding,
            summary,
            extra_data,
        )
    }

    /// One search page. With `keys`, the rank of each match, the next
    /// continuation resumes after the last match shown instead of at an
    /// offset into an answer held whole.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_page_keyed(
        &self,
        matches: Vec<Value>,
        keys: Option<&[SearchAfter]>,
        cursor_probe: &str,
        cursor: Option<(&str, StoredSearchCursor)>,
        binding: SearchCursorBinding,
        summary: String,
        extra_data: Map<String, Value>,
    ) -> DomainResult {
        use crate::application::v13::view::PREFERRED_PAGE_BYTES;
        use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

        let mode = binding.mode.as_str();
        let limit = binding.page_limit;
        let offset = cursor.as_ref().map_or(0, |(_, cursor)| cursor.offset);
        let revision: Option<String> = None;
        let mut page_matches = Vec::new();
        let mut byte_stop = false;
        let measured_bytes = |items: &[Value]| {
            search_page_probe_bytes_with(
                mode,
                items,
                &summary,
                &extra_data,
                revision.as_deref(),
                cursor_probe,
            )
        };
        if measured_bytes(&[]) > MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "search page metadata exceeds the transport limit",
            );
        }
        for item in matches.iter().take(limit) {
            let mut candidate = page_matches.clone();
            candidate.push(item.clone());
            let bytes = measured_bytes(&candidate);
            if bytes > MAX_CANONICAL_RESULT_BYTES {
                if page_matches.is_empty() {
                    return error_result(
                        None,
                        RefusalCode::ResultTooLarge,
                        "one search result exceeds the transport limit",
                    );
                }
                byte_stop = true;
                break;
            }
            if bytes > PREFERRED_PAGE_BYTES && !page_matches.is_empty() {
                byte_stop = true;
                break;
            }
            page_matches.push(item.clone());
        }
        let consumed = page_matches.len();
        let more = matches.len() > consumed;
        let stopped_by = if !more {
            "complete"
        } else if byte_stop {
            "bytes"
        } else {
            "limit"
        };
        let mut result = DomainResult::success(summary);
        result.data = Some(search_page_data(mode, &page_matches, &extra_data));
        result.page = Some(json!({"stoppedBy": stopped_by}));
        result.rev = revision;
        if more {
            let next_offset = match SearchCursorStore::next_offset(offset, consumed) {
                Ok(offset) => offset,
                Err(error) => return error.into_result(None),
            };
            let after =
                keys.and_then(|keys| consumed.checked_sub(1).and_then(|last| keys.get(last)));
            let issued = match cursor {
                Some((_, stored)) => {
                    self.search_cursors
                        .insert_next_after(&stored, next_offset, after)
                }
                None => self
                    .search_cursors
                    .insert_first_after(binding, next_offset, after),
            };
            let issued = match issued {
                Ok(token) => token,
                Err(error) => return error.into_result(None),
            };
            result.cursor = Some(issued);
        }
        result
    }

    /// Поиск по именам и синонимам метаданных.
    ///
    /// Свод берётся из того же справочника, что обслуживает разрешение
    /// локатора, — он уже держит имена, синонимы и адреса. Наружу отдаётся
    /// доказательство совпадения по имени: `at`, `kind`, `title` и `reason`.
    /// Пути тут нет намеренно: `search` — частый ответ, а путь в частом
    /// ответе зовёт читать файл мимо адреса.
    /// Поиск по тексту силами провайдера выбранной роли.
    ///
    /// `symbol` — символьный индекс, `semantic` — смысловая близость;
    /// буквальный поиск роли не имеет. Роли и провайдеры под ними построены
    /// давно; недоставало провода от канонической службы, потому что до
    /// провайдеров ходил лишь legacy-диспетчер, которого на проводе нет.
    pub(super) fn execute_search_role(
        &self,
        invocation: &ActorBoundExecution,
        query: &str,
        role: &str,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        use crate::application::ports::ApplicationPorts;

        let role = match super::super::v13_search_admission::search_provider_role(arguments, role) {
            Ok(role) => role,
            Err(refusal) => return *refusal,
        };

        let context = invocation.workspace_context();
        // Канонический вход — логический адрес; порт разрешения контекста
        // говорит словарём v0.12. Перевод делается здесь и только здесь.
        let mut selector = Map::new();
        match arguments.get("scope") {
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(address) => {
                    selector.insert(
                        "sourceSet".to_string(),
                        Value::String(address.source_set().to_string()),
                    );
                    let owner = if let Some(capability) =
                        unica_bsl::profile::PlatformProfile::v8_3_27().module_capability(&address)
                    {
                        match unica_source_designer::module_layout::module_source_address(
                            &address,
                            capability.owner_kind(),
                            capability.role(),
                            capability.route_kind(),
                        ) {
                            Ok(module) => module.as_str().to_owned(),
                            Err(error) => {
                                return error_result_detailed(
                                    Some(scope.to_string()),
                                    RefusalDetail::ProviderAbsent,
                                    error,
                                );
                            }
                        }
                    } else {
                        address
                            .segments()
                            .iter()
                            .take_while(|segment| segment.name().is_some())
                            .map(|segment| {
                                format!(
                                    "{}.{}",
                                    segment.kind().as_str(),
                                    segment.name().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(".")
                    };
                    if !owner.is_empty() {
                        selector.insert("metadataPath".to_string(), Value::String(owner));
                    }
                }
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
            None => match invocation.admitted_source_set_names().first() {
                Some(name) => {
                    selector.insert("sourceSet".to_string(), Value::String((*name).to_string()));
                }
                None => {
                    return error_result_detailed(
                        None,
                        RefusalDetail::ProviderAbsent,
                        "no admitted source set is available for a role search",
                    )
                }
            },
        }

        let search_context = match self.ports.resolve_code_search_context(context, &selector) {
            Ok((resolved, _)) => resolved,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        let registry = match self.ports.code_intelligence_registry() {
            Ok(registry) => registry,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        #[cfg(test)]
        let registry = self
            .search_providers
            .as_ref()
            .map_or(registry, |providers| {
                CodeIntelligenceRegistry::new(providers.clone())
                    .expect("valid test search providers")
            });
        // Срок берётся из настройки пользователя, а не подменяется умолчанием:
        // человек настроил срок и должен узнать, что настройка не читается.
        let operational = match crate::infrastructure::operational_config::load_operational_config(
            &context.workspace_root,
        ) {
            Ok(config) => config,
            Err(diagnostic) => {
                return error_result(
                    None,
                    RefusalCode::InvalidState,
                    format!("operational config is unreadable: {diagnostic}"),
                )
            }
        };
        let limit = match arguments.get("limit") {
            None => 20,
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                _ => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "provider search page limit must be a positive integer at most 50",
                    )
                }
            },
        };
        let cursor = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => Some(token.as_str()),
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        // An explicit provider role has no neighboring provider to fall back
        // to. Acquire only the selected engine after validating the request,
        // before starting the provider.
        let engine = selected_role_engine(role);
        #[cfg(test)]
        let engine = engine.filter(|_| self.search_providers.is_none());
        if let Some(engine) = engine {
            if let Err(refusal) =
                acquire_engine(invocation, &context.cwd, engine, None, cancellation)
            {
                return *refusal;
            }
        }
        let selected_sources = vec![search_context
            .source_root
            .source_set
            .clone()
            .unwrap_or_default()];
        let binding = SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: arguments
                .get("scope")
                .and_then(Value::as_str)
                .map(str::to_owned),
            mode: role.as_str().to_owned(),
            kind: None,
            source_sets: selected_sources.clone(),
            result_fingerprint: None,
            page_limit: limit,
        };
        // Ranked roles answer one finite window that every page re-reads.
        let request = SearchRequest {
            query: query.to_string(),
            limit: PROVIDER_SEARCH_FETCH_LIMIT,
        };
        let execution = match search_selected_role(
            registry,
            role,
            &request,
            &search_context,
            cancellation,
            operational.code_intelligence(),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                if let Some(refusal) = engine.and_then(|engine| {
                    engine_failure_refusal(&context.cwd, engine, None, cancellation)
                }) {
                    return refusal;
                }
                let code = if error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX)
                {
                    RefusalCode::Cancelled
                } else {
                    RefusalCode::ProviderUnavailable
                };
                return error_result(None, code, error);
            }
        };
        // Поставщик не ответил: если причина — движок, не прошедший проверку
        // поставки (контрольная сумма), отказ называет движок, а не сбой поиска.
        if !execution.ok {
            if let Some(refusal) = engine
                .and_then(|engine| engine_failure_refusal(&context.cwd, engine, None, cancellation))
            {
                return refusal;
            }
        }
        let sources = match invocation.read_sources() {
            Ok(sources) => sources,
            Err(error) => {
                return error_result_detailed(None, RefusalDetail::SourceUnreadable, error)
            }
        };
        let project_hits = |hits: &[unica_code_intelligence::model::ProviderSearchHit]| -> Result<Vec<Map<String, Value>>, ViewError> {
            use unica_source_contract::location::SourceLocation;
            use unica_address::source_target::TargetKind;
            let limited = || Map::from_iter([("limits".into(), json!(["The current source does not prove a Body range containing this provider snippet"]))]);
            let mut projected = vec![Map::new(); hits.len()];
            let mut groups = std::collections::BTreeMap::<String, (usize,QualifiedAddress,Vec<usize>)>::new();
            for (index,hit) in hits.iter().enumerate() {
                let SourceLocation::Addressed { source_set, metadata_path: Some(target), target_kind: TargetKind::Module } = &hit.location else { continue; };
                projected[index] = limited();
                let Some(source_index) = sources.iter().position(|source| source.source_set_name() == source_set) else { continue; };
                let Ok(authority) = sources[source_index].logical_view_read_authority(cancellation) else { continue; };
                let Some(module) = authority.module_address_for_source_target(target) else { continue; };
                groups.entry(module.to_string()).or_insert_with(|| (source_index,module,Vec::new())).2.push(index);
            }
            // Only the current module's semantic projection is retained.
            // Enrichment never changes provider order, ranks or snippets.
            for (_, (source_index,module,indices)) in groups {
                let Ok(authority) = sources[source_index].logical_view_read_authority(cancellation) else { continue; };
                for index in indices {
                    let hit = &hits[index];
                    if hit.line == 0 || hit.snippet.is_empty() { continue; }
                    let Some(to) = hit.end_line.or_else(|| hit.line.checked_add(hit.snippet.lines().count().saturating_sub(1))) else { continue; };
                    match authority.body_for_search_hit(&module, hit.line, to, &hit.snippet) {
                        Ok(Some(body)) => {
                            let at = if to == hit.line { format!("{body}#L{}", hit.line) } else { format!("{body}#L{}-L{to}", hit.line) };
                            projected[index] = Map::from_iter([("at".into(),json!(at))]);
                        }
                        Ok(None) => {}
                        Err(error) if matches!(error.code(), RefusalCode::Cancelled | RefusalCode::DeadlineExceeded | RefusalCode::ConcurrentChange) => return Err(error),
                        Err(_) => {}
                    }
                }
            }
            Ok(projected)
        };
        provider_search_page_projected(
            &self.search_cursors,
            role,
            execution,
            binding,
            cursor,
            cancellation,
            &project_hits,
        )
    }

    pub(super) fn execute_search_names(
        &self,
        invocation: &ActorBoundExecution,
        query: &str,
        arguments: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let mut request = match FindRequest::new(query) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        if let Some(kind) = arguments.get("kind") {
            let Some(kind) = kind.as_str() else {
                return error_result(None, RefusalCode::BadValue, "search kind must be a string");
            };
            request = match request.with_kind(kind) {
                Ok(request) => request,
                Err(error) => return error_result(None, error.code(), error.to_string()),
            };
        }
        let limit = match arguments.get("limit") {
            None => 20,
            Some(value) => match bounded_usize(value).filter(|limit| *limit <= 50) {
                Some(limit) => limit,
                None => {
                    return error_result(
                        None,
                        RefusalCode::BadValue,
                        "search limit must be an integer from 1 through 50",
                    )
                }
            },
        };
        request = match request.with_limit(limit) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        let scope = match arguments.get("scope") {
            None => None,
            Some(Value::String(scope)) => match QualifiedAddress::parse(scope) {
                Ok(scope) => Some(scope),
                Err(error) => {
                    return error_result(
                        Some(scope.to_string()),
                        RefusalCode::BadValue,
                        error.to_string(),
                    )
                }
            },
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "search scope must be a string")
            }
        };
        let sources = match invocation.layout_sources() {
            Ok(sources) => sources,
            Err(error) => return error_result(None, RefusalCode::ProviderUnavailable, error),
        };
        if sources.is_empty() {
            return error_result_detailed(
                None,
                RefusalDetail::ProviderAbsent,
                "name search has no admitted source sets",
            );
        }
        let selected = sources
            .iter()
            .filter(|source| {
                scope
                    .as_ref()
                    .is_none_or(|scope| source.name() == scope.source_set())
            })
            .collect::<Vec<_>>();
        let Some(first) = selected.first() else {
            return error_result(
                scope.as_ref().map(ToString::to_string),
                RefusalCode::NotFound,
                "name search scope does not name an admitted source set",
            );
        };
        let deadline = first.deadline();
        let layout = selected
            .iter()
            .map(|source| LayoutFindSource::new(source.name(), source.kind(), source.root()))
            .collect::<Vec<_>>();
        let scope_binding = scope.as_ref().map(ToString::to_string);
        let binding_for = |result_fingerprint: Option<String>| SearchCursorBinding {
            workspace_identity: invocation.workspace_identity_hash().as_str().to_owned(),
            query: query.to_owned(),
            scope: scope_binding.clone(),
            mode: "names".to_owned(),
            kind: arguments
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned),
            source_sets: selected
                .iter()
                .map(|source| source.name().to_owned())
                .collect(),
            result_fingerprint,
            page_limit: limit,
        };
        // The question half of a continuation is checked before the walk:
        // the rank it resumes after decides which names the page keeps.
        let after = match arguments.get("cursor") {
            None => None,
            Some(Value::String(token)) => {
                match self
                    .search_cursors
                    .question_position(token, &binding_for(None))
                {
                    Ok((_, Some(after))) => match usize::try_from(after.score) {
                        Ok(score) => Some((score, after.at)),
                        Err(_) => return SearchCursorError::Invalid.into_result(None),
                    },
                    // A name continuation always carries its rank.
                    Ok((_, None)) => return SearchCursorError::Invalid.into_result(None),
                    Err(error) => return error.into_result(None),
                }
            }
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "search cursor must be a string",
                )
            }
        };
        // Walking the layout ranks the names and notes whether the scope
        // exists; only this page's candidates and one more are kept.
        let ranked_request = match &scope {
            Some(scope) => request.clone().with_scope(scope.clone()),
            None => request.clone(),
        };
        let watched = scope.as_ref().map(name_scope_witnesses).unwrap_or_default();
        let built = match self.find_builder.search_names(
            &layout,
            NameRanker::new(ranked_request, watched, limit.saturating_add(1), after),
            deadline,
            cancellation,
        ) {
            Ok(built) => built,
            Err(error) => return find_build_error_result(None, error),
        };
        if let Some(scope) = scope {
            if let Err((code, message)) =
                validate_name_scope(&|at| built.present.contains(at), &scope, first.kind())
            {
                if built.omissions.total != 0
                    && matches!(code, RefusalCode::NotFound | RefusalCode::InvalidState)
                {
                    return error_result_detailed(
                        Some(scope.to_string()),
                        RefusalDetail::SourceUnreadable,
                        "name search cannot prove the requested scope in an incomplete source layout",
                    );
                }
                return error_result(Some(scope.to_string()), code, message);
            }
        }
        let found = built.names;
        let summary = if built.omissions.total == 0 {
            "name search completed".to_string()
        } else {
            format!(
                "name search has {} unreadable descriptor candidates",
                built.omissions.total
            )
        };
        let coverage = json!({
            "complete": built.omissions.total == 0,
            "omitted": built.omissions.total,
            "detailsTruncated": built.omissions.total > built.omissions.details.len(),
            "details": built.omissions.details.iter().map(|detail| json!({
                "sourceSet": detail.source_set,
                "reason": detail.reason,
            })).collect::<Vec<_>>(),
        });
        let mut extra_data = Map::new();
        extra_data.insert("sourceCoverage".to_owned(), coverage.clone());
        extra_data.insert("approximate".to_owned(), Value::Bool(found.nearest));
        // A name continuation is keyed by the rank of its last item; the
        // probe reserves a key as long as the longest address.
        let cursor_probe = SearchCursorStore::keyed_token_probe(found.largest_address_bytes.max(1));
        let empty_page_bytes =
            search_page_probe_bytes_with("names", &[], &summary, &extra_data, None, &cursor_probe);
        if empty_page_bytes > unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "name search page metadata exceeds the transport limit",
            );
        }
        // An item after the first page must not receive a continuation that
        // could never carry it. The empty probe includes every fixed response
        // field and its cursor; adding one JSON array item adds exactly its
        // serialized byte length, and the walk kept the largest one.
        if found.matched != 0
            && empty_page_bytes.saturating_add(found.largest_item_bytes)
                > unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES
        {
            return error_result(
                None,
                RefusalCode::ResultTooLarge,
                "one name search result exceeds the transport limit",
            );
        }
        // Names do not have a source revision lease. Every page walks the
        // layout again and binds the cursor to the ranked answer — its count
        // and order-free digest — and coverage instead of a source revision.
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-names-result-v2\0");
        hasher.update(found.fingerprint);
        let evidence = json!({"approximate": found.nearest, "sourceCoverage": coverage.clone()});
        let bytes = serde_json::to_vec(&evidence).expect("name coverage is serializable");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        if let Some(refusal) = name_search_interruption(deadline, cancellation) {
            return refusal;
        }
        let binding = binding_for(Some(format!("names-sha256-v2:{:x}", hasher.finalize())));
        let cursor = match arguments.get("cursor").and_then(Value::as_str) {
            None => None,
            Some(token) => match self.search_cursors.read(token, &binding) {
                Ok(cursor) => Some((token, cursor)),
                Err(error) => return error.into_result(None),
            },
        };
        let keys = found
            .candidates
            .iter()
            .map(|(score, candidate)| SearchAfter {
                score: *score as u64,
                at: candidate.at().to_owned(),
            })
            .collect::<Vec<_>>();
        let matches = found
            .candidates
            .iter()
            .map(|(_, candidate)| public_name_match(candidate))
            .collect();
        let result = self.search_page_keyed(
            matches,
            Some(&keys),
            &cursor_probe,
            cursor,
            binding,
            summary,
            extra_data,
        );
        if let Some(refusal) = name_search_interruption(deadline, cancellation) {
            return refusal;
        }
        result
    }
}
