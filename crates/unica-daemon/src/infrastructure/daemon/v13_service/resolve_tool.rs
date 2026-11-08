//! `unica.resolve`: адреса, пути и точки метаданных.

use super::*;

impl CanonicalV13ReadService {
    /// Аварийный двусторонний мост в файловую раскладку.
    ///
    /// Ответ точный или его нет: догадок здесь не делают. Ранжирование живёт
    /// в `search`, и путать эти два вопроса одним именем было главной бедой
    /// снятого `find`.
    pub(super) fn execute_resolve(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let arguments = invocation.arguments();
        for key in arguments.keys() {
            if !["at", "path", "paths"].contains(&key.as_str()) {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    format!("resolve does not accept argument `{key}`"),
                );
            }
        }
        let at = match arguments.get("at") {
            None => None,
            Some(Value::String(at)) => Some(at.as_str()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "resolve at must be a string")
            }
        };
        let path = match arguments.get("path") {
            None => None,
            Some(Value::String(path)) => Some(path.as_str()),
            Some(_) => {
                return error_result(None, RefusalCode::BadValue, "resolve path must be a string")
            }
        };
        let paths = match arguments.get("paths") {
            None => None,
            Some(Value::Array(items)) => {
                match items.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
                    Some(items) => Some(items),
                    None => {
                        return error_result(
                            None,
                            RefusalCode::BadValue,
                            "resolve paths must be an array of strings",
                        )
                    }
                }
            }
            Some(_) => {
                return error_result(
                    None,
                    RefusalCode::BadValue,
                    "resolve paths must be an array of strings",
                )
            }
        };
        let request = match ResolveRequest::new(at, path, paths.as_deref()) {
            Ok(request) => request,
            Err(error) => return error_result(None, error.code(), error.to_string()),
        };
        match request {
            ResolveRequest::Path(path) => {
                match with_resolve_layout(invocation, |layout, deadline| {
                    self.locate_path(invocation, layout, deadline, &path, cancellation)
                }) {
                    Ok(Ok(source)) => resolve_result(source.source),
                    Ok(Err(refusal)) | Err(refusal) => *refusal,
                }
            }
            ResolveRequest::Paths(items) => {
                match with_resolve_layout(invocation, |layout, deadline| {
                    self.resolve_paths(invocation, layout, deadline, &items, cancellation)
                }) {
                    Ok(result) => result,
                    Err(refusal) => *refusal,
                }
            }
            ResolveRequest::Address(address) => {
                self.resolve_address(invocation, &address, cancellation)
            }
        }
    }

    /// Пакет путей и строк: каждый различный путь ищется один раз, каждый
    /// модуль читается одной проекцией на все его строки. Ответ — по
    /// элементу на позицию входа, в том же порядке; отказ элемента не
    /// роняет пакет. Отмена и срок останавливают весь вызов: публикация
    /// чтения раскладки всё равно заменила бы ими ответ.
    pub(super) fn resolve_paths(
        &self,
        invocation: &ActorBoundExecution,
        layout: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        items: &[Result<PathLine, ResolveError>],
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let mut located = BTreeMap::<String, Result<LocatedSource, Box<DomainResult>>>::new();
        let mut lines = BTreeMap::<String, BTreeSet<usize>>::new();
        for item in items.iter().flatten() {
            let key = item.key();
            if !located.contains_key(&key) {
                let source =
                    self.locate_path(invocation, layout, deadline, item.path(), cancellation);
                if let Err(refusal) = &source {
                    if call_stopping_refusal(refusal) {
                        return *refusal.clone();
                    }
                }
                located.insert(key.clone(), source);
            }
            if let (Some(line), Ok(source)) = (item.line(), &located[&key]) {
                // Строчный источник — файл BSL; у модуля целиком диапазона
                // нет, но строки у файла есть.
                if source.source.path().to_ascii_lowercase().ends_with(".bsl") {
                    lines.entry(key).or_default().insert(line);
                }
            }
        }
        let mut placed =
            BTreeMap::<String, Result<BTreeMap<usize, ModuleLine>, Box<DomainResult>>>::new();
        for (key, requested) in lines {
            let Ok(located_source) = &located[&key] else {
                continue;
            };
            let source = &located_source.source;
            let requested = requested.into_iter().collect::<Vec<_>>();
            let module = QualifiedAddress::parse(source.address())
                .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))
                .and_then(|module| {
                    let authority = invocation
                        .layout_read_source(module.source_set())
                        .and_then(|read| read.logical_view_read_authority(cancellation))
                        .map_err(|error| {
                            ViewError::detailed(RefusalDetail::SourceUnreadable, error)
                        })?;
                    let authority = match &located_source.external_binding {
                        Some(binding) => authority.with_external_point_binding(binding.clone()),
                        None => authority,
                    };
                    authority.module_lines(&module, &requested)
                });
            let answer = match module {
                Ok(found) => Ok(requested.into_iter().zip(found).collect()),
                Err(error)
                    if matches!(
                        error.code(),
                        RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
                    ) =>
                {
                    return view_error_result(Some(source.address().to_string()), error)
                }
                Err(error) => Err(Box::new(view_error_result(
                    Some(source.address().to_string()),
                    error,
                ))),
            };
            placed.insert(key, answer);
        }
        let mut resolved = 0_usize;
        let answers = items
            .iter()
            .map(|item| {
                let item = match item {
                    Ok(item) => item,
                    Err(error) => {
                        return batch_refusal(&error_result(None, error.code(), error.to_string()))
                    }
                };
                let source = match &located[&item.key()] {
                    Ok(source) => &source.source,
                    Err(refusal) => return batch_refusal(refusal),
                };
                let answer = match (item.line(), placed.get(&item.key())) {
                    (None, _) => ResolvedPath::new(source.address(), source.kind(), None),
                    (Some(_), None) => ResolvedPath::new(
                        source.address(),
                        source.kind(),
                        Some(LinePlace::NotLineBased),
                    ),
                    (Some(_), Some(Err(refusal))) => return batch_refusal(refusal),
                    (Some(line), Some(Ok(module))) => match &module[&line] {
                        ModuleLine::Method(method) => ResolvedPath::new(
                            method.to_string(),
                            NodeKind::Method.as_str(),
                            Some(LinePlace::InMethod),
                        ),
                        ModuleLine::OutsideMethod => ResolvedPath::new(
                            source.address(),
                            source.kind(),
                            Some(LinePlace::OutsideMethod),
                        ),
                        ModuleLine::PastEnd { lines } => {
                            return batch_refusal(&error_result(
                                Some(source.address().to_string()),
                                RefusalCode::NotFound,
                                format!(
                                    "line {line} is past the end of `{}`, which has {lines} lines",
                                    source.path()
                                ),
                            ))
                        }
                    },
                };
                resolved += 1;
                serde_json::to_value(answer).expect("the closed resolve model always serializes")
            })
            .collect::<Vec<_>>();
        let mut result =
            DomainResult::success(format!("{resolved} of {} paths resolved", items.len()));
        result.data = Some(json!({ "items": answers }));
        result
    }

    /// Путь пришёл снаружи: точный предмет раскладки или отказ, который
    /// одиночный `path` отдаёт целиком, а пакет — элементу.
    pub(super) fn locate_path(
        &self,
        invocation: &ActorBoundExecution,
        layout: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        path: &str,
        cancellation: &CancellationToken,
    ) -> Result<LocatedSource, Box<DomainResult>> {
        let entry = self
            .find_builder
            .locate_path(layout, path, deadline, cancellation)
            .map_err(|error| Box::new(find_build_error_result(None, error)))?;
        let Some(entry) = entry else {
            return Err(Box::new(error_result(
                None,
                RefusalCode::NotFound,
                format!("no admitted source set places `{path}`"),
            )));
        };
        // Кандидаты на присоединённые файлы подтверждаются тем же читателем,
        // что и view: регистрацию владельца и точный ресурс нельзя вывести
        // только из правдоподобной раскладки.
        if matches!(entry.kind(), "Module" | "Metadata" | "Definition")
            || entry
                .placed_path()
                .is_some_and(|path| path.ends_with("/Ext/Form.xml"))
        {
            let address = QualifiedAddress::parse(entry.at()).map_err(|error| {
                Box::new(error_result(None, RefusalCode::BadValue, error.to_string()))
            })?;
            let authority = invocation
                .layout_read_source(address.source_set())
                .and_then(|source| source.logical_view_read_authority(cancellation))
                .map_err(|error| {
                    Box::new(error_result_detailed(
                        Some(entry.at().to_owned()),
                        RefusalDetail::SourceUnreadable,
                        error,
                    ))
                })?;
            let (authority, external_binding) = if let Some(descriptor) = entry.owner_descriptor() {
                let source = invocation
                    .layout_read_source(address.source_set())
                    .map_err(|message| {
                        Box::new(error_result_detailed(
                            Some(entry.at().to_owned()),
                            RefusalDetail::SourceUnreadable,
                            message,
                        ))
                    })?;
                if matches!(
                    source.source_kind(),
                    unica_source_contract::SourceSetKind::ExternalProcessor
                        | unica_source_contract::SourceSetKind::ExternalReport
                ) {
                    let name = address
                        .segments()
                        .first()
                        .and_then(|owner| owner.name())
                        .ok_or_else(|| {
                            Box::new(error_result(
                                Some(entry.at().to_owned()),
                                RefusalCode::InvalidSource,
                                "external target has no owner name",
                            ))
                        })?;
                    let binding =
                        unica_source_designer::external_artifact::ExternalArtifactBinding::new(
                            source.source_kind(),
                            name,
                            std::path::Path::new(descriptor),
                        )
                        .map_err(|message| {
                            Box::new(error_result(
                                Some(entry.at().to_owned()),
                                RefusalCode::InvalidSource,
                                message,
                            ))
                        })?;
                    (
                        authority.with_external_point_binding(binding.clone()),
                        Some(binding),
                    )
                } else {
                    (authority, None)
                }
            } else {
                (authority, None)
            };
            return match Self::resolve_with_authority(&authority, &address, entry.placed_path()) {
                Ok(source) if Some(source.path()) == entry.placed_path() => Ok(LocatedSource {
                    source,
                    external_binding,
                }),
                Ok(_) => Err(Box::new(error_result(
                    Some(entry.at().to_owned()),
                    RefusalCode::InvalidSource,
                    "the logical reader does not place the requested resource at this path",
                ))),
                Err(error) => Err(Box::new(view_error_result(
                    Some(entry.at().to_owned()),
                    error,
                ))),
            };
        }
        Ok(LocatedSource {
            source: ResolvedSource::new(
                entry.at(),
                entry.kind(),
                entry.placed_path().expect("located path has a place"),
                ResolvedLines::NotLineBased,
            ),
            external_binding: None,
        })
    }

    pub(super) fn resolve_address(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> DomainResult {
        let form_owner = address.form_metadata_owner().or_else(|| {
            matches!(address.segments().last(), Some(last) if last.name().is_some() && matches!(last.kind(), NodeKind::Form | NodeKind::CommonForm))
                .then(|| address.clone())
        });
        if let Some(owner) = form_owner {
            // Preserve resolve's descriptor/owner identity proof before reading
            // content. A missing required owner is invalid source; a successful
            // descriptor proof does not substitute that file for the form body.
            if let Some(mut proof) = self.resolve_metadata_point(invocation, &owner, cancellation) {
                if !proof.ok {
                    proof.at = Some(address.to_string());
                    return proof;
                }
            }
        }
        if matches!(address.segments().last(), Some(last) if last.name().is_some() && matches!(last.kind(), NodeKind::Form | NodeKind::CommonForm))
            || address
                .segments()
                .iter()
                .any(|segment| !segment.kind().is_metadata_kind())
        {
            return match self.resolve_projected_source(invocation, address, cancellation) {
                Ok(source) => resolve_result(source),
                Err(error) => view_error_result(Some(address.to_string()), error),
            };
        }
        let at = address.to_string();
        if let Some(result) = self.resolve_metadata_point(invocation, address, cancellation) {
            return result;
        }
        let directory = match self.layout_directory(
            invocation,
            Some(at.clone()),
            address.source_set(),
            cancellation,
        ) {
            Ok(directory) => directory,
            Err(refusal) => return *refusal,
        };
        let Some(entry) = directory.locate_address(&at) else {
            return error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            );
        };
        let Some(placed) = entry.placed_path() else {
            return error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            );
        };
        let kind = entry.kind().to_string();
        let placed = placed.to_string();
        resolve_result(ResolvedSource::new(
            at,
            kind,
            placed,
            ResolvedLines::NotLineBased,
        ))
    }

    /// Resolves one metadata object by its own layout place: the target file
    /// and its necessary owners, the same proof as `resolve(path)`. It never
    /// builds the full name directory, so foreign descriptors are not read and
    /// their damage does not decide this answer (#1457).
    pub(super) fn resolve_metadata_point(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Option<DomainResult> {
        let at = address.to_string();
        let source = match admitted_read_source(
            invocation.read_sources(),
            |source| source.source_set_name(),
            Some(&at),
            address.source_set(),
        ) {
            Ok(source) => source,
            Err(refusal) => return Some(*refusal),
        };
        let relative = metadata_layout_path(address, source.source_kind())?;
        let relative = relative.as_str();
        let root = source.retained_root();
        let layout = [LayoutFindSource::new(
            source.source_set_name(),
            source.source_kind(),
            root.as_ref(),
        )];
        let entry =
            match self
                .find_builder
                .locate_path(&layout, relative, source.deadline(), cancellation)
            {
                Ok(entry) => entry,
                Err(error) => return Some(find_build_error_result(Some(at), error)),
            };
        let external = matches!(
            source.source_kind(),
            unica_source_contract::SourceSetKind::ExternalProcessor
                | unica_source_contract::SourceSetKind::ExternalReport
        );
        if external
            && entry.as_ref().is_none_or(|entry| {
                entry.at() != at
                    && QualifiedAddress::parse(entry.at())
                        .ok()
                        .and_then(|located| located.form_metadata_owner())
                        .as_ref()
                        != Some(address)
            })
        {
            return None;
        }
        let Some((kind, placed)) = entry.as_ref().and_then(|entry| {
            (entry.at() == at
                || QualifiedAddress::parse(entry.at())
                    .ok()
                    .and_then(|located| located.form_metadata_owner())
                    .as_ref()
                    == Some(address))
            .then(|| entry.placed_path().map(|placed| (entry.kind(), placed)))
            .flatten()
        }) else {
            return Some(error_result(
                Some(at.clone()),
                RefusalCode::NotFound,
                format!("the source layout does not place `{at}`"),
            ));
        };
        Some(resolve_result(ResolvedSource::new(
            at.clone(),
            kind.to_string(),
            placed.to_string(),
            ResolvedLines::NotLineBased,
        )))
    }

    pub(super) fn resolve_projected_source(
        &self,
        invocation: &ActorBoundExecution,
        address: &QualifiedAddress,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedSource, ViewError> {
        let source = invocation
            .read_sources()
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?
            .into_iter()
            .find(|source| source.source_set_name() == address.source_set())
            .ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "resolve source set was not admitted")
            })?;
        let authority = source
            .logical_view_read_authority(cancellation)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        Self::resolve_with_authority(&authority, address, None)
    }

    pub(super) fn resolve_with_authority(
        authority: &crate::infrastructure::v13_read::LogicalViewReadAuthority<'_>,
        address: &QualifiedAddress,
        requested_path: Option<&str>,
    ) -> Result<ResolvedSource, ViewError> {
        let snapshot = authority.snapshot(address)?;
        let canonical = authority.canonical_address(address, &snapshot)?;
        let (kind, lines) = authority.source_lines(&canonical, &snapshot)?;
        let path = authority.existing_export_path(&canonical, requested_path)?;
        if authority.snapshot(&canonical)?.revision != snapshot.revision {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed during source location resolution",
            ));
        }
        authority.read_checkpoint()?;
        Ok(ResolvedSource::new(
            canonical.to_string(),
            kind,
            path,
            lines,
        ))
    }
}
