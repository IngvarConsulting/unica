//! Модули: текст, строки, тела и владельцы модулей.

use super::*;
use std::borrow::Cow;
use unica_code_intelligence::model::{ProviderSearchHit, SearchSnippetEvidence};

impl<'a> LogicalViewReadAuthority<'a> {
    /// Resolve names the node's kind and its own source lines. In a BSL
    /// module `Method` spans the whole declaration through its closing line,
    /// and its `Body` the lines `view` returns as its text: the doc comment
    /// above the method through the closing line. The module `Body` counts the
    /// file stream like the view fast path, without projecting the module.
    pub fn source_lines(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(String, ResolvedLines), ViewError> {
        let route = route_logical_address(at, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let capability = route
            .module()
            .filter(|_| route.reader() == LogicalReader::Module);
        let Some(capability) = capability else {
            let projected = self.read_exact(at, admitted)?;
            return Ok((projected.kind().to_string(), ResolvedLines::NotLineBased));
        };
        let (module_at, prefix_len) = module_prefix(at, self.profile, capability)?;
        let suffix = &at.segments()[prefix_len..];
        let is_named =
            |segment: &AddressSegment, kind| segment.kind() == kind && segment.name().is_some();
        let is_branch =
            |segment: &AddressSegment, kind| segment.kind() == kind && segment.name().is_none();
        let module_body = matches!(suffix, [body] if is_branch(body, NodeKind::Body));
        let method = matches!(suffix, [method] if is_named(method, NodeKind::Method));
        let variable = matches!(suffix, [variable] if is_named(variable, NodeKind::Variable));
        let method_body = matches!(
            suffix,
            [method, body] if is_named(method, NodeKind::Method) && is_branch(body, NodeKind::Body)
        );
        let region = !suffix.is_empty()
            && suffix
                .iter()
                .all(|segment| is_named(segment, NodeKind::Region));
        if !(module_body || method || method_body || region || variable) {
            let projected = self.read_exact(at, admitted)?;
            return Ok((projected.kind().to_string(), ResolvedLines::NotLineBased));
        }
        self.read_checkpoint()?;
        if admitted.source_set_identity != self.read.source_set_identity()
            || admitted.revision != self.exact_revision()?
        {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "source revision changed before the source line read",
            ));
        }
        self.verify_module_owner(&module_at, capability, admitted)?;
        if module_body {
            let target = module_source_address(&module_at, capability)?;
            let count = self
                .read
                .module_line_count(&target, || self.read_checkpoint())?
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "the BSL module source file does not exist",
                    )
                })?;
            return Ok((
                NodeKind::Body.as_str().to_string(),
                selected_lines(at, line_range(1, count))?,
            ));
        }
        let projections = self.module_projection(&module_at, capability, admitted)?;
        let (kind, lines) = if variable {
            let variable = find_variable(&projections, suffix[0].name().unwrap_or_default())?;
            (
                NodeKind::Variable,
                line_range(variable.from_line, variable.to_line),
            )
        } else if region {
            let region = projections
                .region(&at.to_string())
                .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
            (
                NodeKind::Region,
                region.end_line.map_or(ResolvedLines::NotLineBased, |end| {
                    line_range(region.line, end)
                }),
            )
        } else {
            let name = suffix[0].name().unwrap_or_default();
            let method = find_method(&projections, name)?;
            if method_body {
                (
                    NodeKind::Body,
                    line_range(method.text_from_line, method.to_line),
                )
            } else {
                (
                    NodeKind::Method,
                    line_range(method.from_line, method.to_line),
                )
            }
        };
        self.read_checkpoint()?;
        Ok((kind.as_str().to_string(), selected_lines(at, lines)?))
    }

    /// One retained projection of the module that `module` names, fenced by
    /// the source identity it was admitted with: a change while `inspect`
    /// reads it refuses the whole answer instead of mixing two revisions.
    pub(super) fn with_module_projection<T>(
        &self,
        module: &QualifiedAddress,
        inspect: impl FnOnce(
            &QualifiedAddress,
            ModuleCapability,
            &ModuleProjectionSet,
        ) -> Result<T, ViewError>,
    ) -> Result<T, ViewError> {
        self.read_checkpoint()?;
        let admitted = self.snapshot(module)?;
        let route = route_logical_address(module, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let capability = route.module().ok_or_else(|| {
            ViewError::new(
                RefusalCode::BadValue,
                "the address does not name a BSL module",
            )
        })?;
        let (module_at, _) = module_prefix(module, self.profile, capability)?;
        self.verify_module_owner(&module_at, capability, &admitted)?;
        let projections = self.module_projection(&module_at, capability, &admitted)?;
        let result = inspect(&module_at, capability, &projections)?;
        self.read_checkpoint()?;
        if self.snapshot(module)? != admitted {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "source changed while module line boundaries were read",
            ));
        }
        Ok(result)
    }

    /// Places each physical file line of a BSL module: inside the method
    /// whose own range holds it — the range `resolve` names for `Method`,
    /// from its first annotation through the closing line — outside every
    /// method, or past the end of the file. One projection answers all lines.
    pub fn module_lines(
        &self,
        module: &QualifiedAddress,
        lines: &[usize],
    ) -> Result<Vec<ModuleLine>, ViewError> {
        self.with_module_projection(module, |_, _, projections| {
            let count = projections.source_text().lines().count();
            lines
                .iter()
                .map(|&line| {
                    if line == 0 || line > count {
                        return Ok(ModuleLine::PastEnd { lines: count });
                    }
                    projections
                        .methods()
                        .iter()
                        .find(|method| method.from_line <= line && line <= method.to_line)
                        .map_or(Ok(ModuleLine::OutsideMethod), |method| {
                            QualifiedAddress::parse(&method.at)
                                .map(ModuleLine::Method)
                                .map_err(|error| {
                                    ViewError::new(RefusalCode::BadValue, error.to_string())
                                })
                        })
                })
                .collect()
        })
    }

    /// Resolves semantic containment from the same projection as Method
    /// views. Provider symbols are never accepted as boundary evidence.
    pub fn narrowest_body_for_lines(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
    ) -> Result<QualifiedAddress, ViewError> {
        self.with_module_projection(module, |module, _, projection| {
            let body = method_body_address(module, containing_method(projection, from, to));
            QualifiedAddress::parse(&body)
                .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))
        })
    }

    /// The module address a source-root-relative BSL file path has by the
    /// source layout, for a place a symbol provider names by file.
    pub(crate) fn module_address_for_path(&self, path: &str) -> Option<QualifiedAddress> {
        crate::infrastructure::event_sources::module_address_for_relative(
            self.read.source_set(),
            self.read.source_set_kind(),
            self.profile,
            std::path::Path::new(path),
        )
    }

    /// The saved text of a module, read by the same projection as `view`.
    pub fn module_text(&self, module: &QualifiedAddress) -> Result<String, ViewError> {
        self.with_module_projection(module, |_, _, projections| {
            Ok(projections.source_text().to_string())
        })
    }

    /// The saved text of the module at a source-root-relative path, read as
    /// is, without projecting it.
    pub(crate) fn module_source_at_path(&self, path: &str) -> Option<String> {
        let bytes = self
            .read
            .read_optional_relative_whole(std::path::Path::new(path))
            .ok()??;
        let text = String::from_utf8(bytes).ok()?;
        Some(text.trim_start_matches('\u{feff}').to_string())
    }

    /// Whether a common module is global, by its descriptor.
    pub(crate) fn common_module_is_global(&self, module: &QualifiedAddress) -> Option<bool> {
        self.with_module_projection(module, |_, _, projections| {
            Ok(projections
                .summary()
                .props
                .common_module
                .as_ref()
                .and_then(|properties| properties.global))
        })
        .ok()
        .flatten()
    }

    /// The source-root-relative paths of the modules that spell `name`, from
    /// the set's name index; `None` until its first build has finished.
    pub(crate) fn modules_naming(
        &self,
        name: &str,
    ) -> Option<unica_code_intelligence::symbols::NamedModules> {
        let source_set = self.read.source_set().to_string();
        let kind = self.read.source_set_kind();
        let profile = self.profile;
        let modules: crate::infrastructure::name_index::ModulePaths =
            std::sync::Arc::new(move |path: &std::path::Path| {
                crate::infrastructure::event_sources::module_address_for_relative(
                    &source_set,
                    kind,
                    profile,
                    path,
                )
                .is_some()
            });
        self.read
            .artifacts()
            .names()
            .modules_naming(name, self.read.retained_root(), &modules)
    }

    pub fn module_address_for_source_target(
        &self,
        target: &MetadataAddress,
    ) -> Option<QualifiedAddress> {
        let path = unica_source_designer::module_layout::module_relative(
            target,
            self.read.source_set_kind(),
        )
        .ok()?;
        crate::infrastructure::event_sources::module_address_for_relative(
            self.read.source_set(),
            self.read.source_set_kind(),
            self.profile,
            &path,
        )
    }

    pub fn bodies_for_search_hits<'hit>(
        &self,
        module: &QualifiedAddress,
        hits: impl IntoIterator<Item = &'hit ProviderSearchHit>,
    ) -> Result<Vec<Option<String>>, ViewError> {
        self.with_module_projection(module, |module, capability, projection| {
            let source = projection.source_text();
            let line_count = source.lines().count();
            let mut addresses = Vec::new();
            for hit in hits {
                self.read_checkpoint()?;
                addresses.push(proven_search_address(module, projection, line_count, hit));
            }
            if addresses.iter().any(Option::is_some) {
                self.read_checkpoint()?;
                let target = module_source_address(module, capability)?;
                // A retained revision fences the actor/root. Recheck the bytes
                // once per matching module for writes outside the actor.
                if self.read.module_source(&target)?.as_deref() != Some(source) {
                    return Err(ViewError::new(
                        RefusalCode::ConcurrentChange,
                        "module source changed while proving search hits",
                    ));
                }
            }
            Ok(addresses)
        })
    }

    pub(super) fn module_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let Some(capability) = route.module() else {
            return self.module_branch_view(route.at(), admitted);
        };
        let (module_at, prefix_len) = module_prefix(route.at(), self.profile, capability)?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        if self.read.source_set_kind() == SourceSetKind::Configuration
            && route.at().segments().last().is_some_and(|segment| {
                segment.kind() == NodeKind::Event && segment.name().is_some()
            })
        {
            let LogicalEventSource::Platform(source) = resolve_event_source(
                self.read.source_set(),
                self.read.source_set_kind(),
                self.profile,
                route.at(),
            )
            .map_err(|error| ViewError::new(error.code(), error.to_string()))?
            else {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "module event did not resolve to a Platform source",
                ));
            };
            let projection = self.module_projection(&source.module_at, capability, admitted)?;
            let event = project_platform_event_from_projection(&source.event_at, &projection)
                .map_err(|error| ViewError::new(error.code(), error.to_string()))?;
            return Ok(NodeViewData::Node(event_node(&event)));
        }
        let projections = self.module_projection(&module_at, capability, admitted)?;
        let suffix = &route.at().segments()[prefix_len..];
        let mut binding_items = match suffix {
            [method, rest @ ..]
                if capability.role() == ModuleRole::Form
                    && method.kind() == NodeKind::Method
                    && (rest.is_empty()
                        || matches!(rest, [binding] if binding.kind() == NodeKind::Binding))
                    && projections.methods().iter().any(|projection| {
                        method.name().is_none_or(|name| projection.name == name)
                            && !projection.handles.is_empty()
                    }) =>
            {
                self.form_method_bindings(&module_at, &projections, admitted)?
            }
            _ => BTreeMap::new(),
        };
        if let [method, binding] = suffix {
            if method.kind() == NodeKind::Method && binding.kind() == NodeKind::Binding {
                let method = find_method(&projections, method.name().unwrap_or_default())?;
                let declarative = binding_items.remove(&method.at).unwrap_or_default();
                return self.method_binding_view(
                    route.at(),
                    &module_at,
                    &projections,
                    method,
                    declarative,
                    admitted,
                );
            }
        }
        let mut view = module_projection_view(route.at(), prefix_len, &projections, binding_items)?;
        if capability.role() == ModuleRole::Common && route.at() == &module_at {
            let (synonym, limits) = self.object_synonym(&module_at)?;
            if let Some(synonym) = synonym {
                view = view.with_prop("synonym", synonym);
            }
            view = view.with_additional_limits(limits);
        }
        // Узел общего модуля — и узел объекта метаданных: его ветвь `Relation`
        // (подсистемы и библиотеки) читается по адресу, поэтому узел её
        // называет; непрочитанный состав подсистем узел называет пределом.
        if capability.role() == ModuleRole::Common && route.at().segments().len() == 1 {
            let target = route.at().segments()[0].name().and_then(|name| {
                MetadataAddress::parse_logical(&format!(
                    "{}.{name}",
                    NodeKind::CommonModule.as_str()
                ))
                .ok()
            });
            if let (NodeViewData::Node(node), Some(target)) = (&view, target.as_ref()) {
                // Состав подсистемы может назвать модуль идентификатором
                // из его описания, а не именем.
                let uuid = self
                    .read
                    .metadata_descriptor(target)
                    .ok()
                    .and_then(|descriptor| {
                        crate::infrastructure::v13_read_port::descriptor_uuid(&descriptor)
                    });
                match self.subsystem_relations(target, uuid) {
                    Ok((memberships, libraries)) => {
                        let count = memberships.len() + libraries.len();
                        if count > 0 {
                            return Ok(NodeViewData::Node(node.clone().with_branch(
                                BranchRef::new(
                                    format!("{}.{}", route.at(), NodeKind::Relation.as_str()),
                                    count,
                                ),
                            )));
                        }
                    }
                    // Ветвь, которую нельзя сосчитать, узел называет пределом.
                    Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                        return Ok(
                            view.with_limit(format!("relations.subsystems не прочитано: {error}"))
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(view)
    }

    pub(super) fn object_synonym(
        &self,
        at: &QualifiedAddress,
    ) -> Result<(Option<Value>, Vec<String>), ViewError> {
        self.read_checkpoint()?;
        self.read.object_synonym(at)
    }

    pub(super) fn module_projection(
        &self,
        module_at: &QualifiedAddress,
        capability: ModuleCapability,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<ModuleProjectionSet>, ViewError> {
        let key = ModuleProjectionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
            module_at: module_at.to_string(),
        };
        let mut cache = self.module_projections.lock().map_err(|_| {
            ViewError::detailed(RefusalDetail::CachePoisoned, "module cache is poisoned")
        })?;
        if let Some(projection) = cache.get(&key) {
            return Ok(Arc::clone(projection));
        }
        let metadata_path = module_source_address(module_at, capability)?;
        let source = self.read.module_source(&metadata_path)?;
        let common_module = if capability.role() == ModuleRole::Common {
            let owner = metadata_path
                .as_str()
                .rsplit_once('.')
                .map(|(owner, _)| owner)
                .ok_or_else(|| {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "common module owner is invalid",
                    )
                })?;
            let owner = MetadataAddress::parse_logical(owner).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            let descriptor = self.read.metadata_descriptor(&owner)?;
            Some(common_module_properties(
                &descriptor,
                self.read.source_set_kind(),
            )?)
        } else {
            None
        };
        let handles = if capability.role() == ModuleRole::Form {
            self.form_bindings(module_at, admitted)?
        } else {
            Vec::new()
        };
        let projection = Arc::new(
            project_module(ModuleProjectionRequest {
                at: module_at,
                platform: self.platform_profile(),
                capability,
                title: module_title(module_at, capability),
                rev: &admitted.revision,
                source: source.as_deref(),
                common_module,
                handles: &handles,
                declarative_bindings: &[],
                extension_targets: &[],
                platform_event_write: match self.read.source_set_kind() {
                    SourceSetKind::Extension => PlatformEventWriteCapability::Unproved,
                    _ => PlatformEventWriteCapability::Proven,
                },
            })
            .map_err(|error| self.module_projection_failure(module_at, source.as_deref(), error))?,
        );
        cache.insert(key, Arc::clone(&projection));
        Ok(projection)
    }

    pub(super) fn module_branch_view(
        &self,
        branch: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if matches!(
            branch.segments(),
            [root] if root.kind() == NodeKind::Module
        ) && matches!(
            self.read.source_set_kind(),
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "external source sets have no configuration runtime module branch",
            ));
        }
        if let Some(owner) = module_branch_owner(branch)? {
            self.verify_registered_module_owner(&owner, admitted)?;
        }
        project_module_branch(branch, self.profile)
    }

    pub(super) fn verify_module_owner(
        &self,
        module_at: &QualifiedAddress,
        capability: ModuleCapability,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        if capability.route_kind() == ModuleRouteKind::Root {
            return match self.read.source_set_kind() {
                // An extension extends the configuration runtime modules from
                // its own root (`Ext/ManagedApplicationModule.bsl` and the
                // like), so its `Configuration.xml` owns them exactly as the
                // main configuration does.
                SourceSetKind::Configuration | SourceSetKind::Extension => Ok(()),
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
                    Err(ViewError::new(
                        RefusalCode::NotFound,
                        "external source sets have no configuration runtime modules",
                    ))
                }
            };
        }
        let owner = module_owner_address(module_at, capability)?;
        self.verify_registered_module_owner(&owner, admitted)
    }

    pub(super) fn verify_registered_module_owner(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        self.verify_registered_owner(target, admitted)
    }
}

fn containing_method(
    projection: &ModuleProjectionSet,
    from: usize,
    to: usize,
) -> Option<&MethodProjection> {
    projection
        .methods()
        .iter()
        .filter(|method| from >= method.text_from_line && to <= method.to_line)
        .min_by_key(|method| method.to_line - method.text_from_line)
}

fn method_body_address(module: &QualifiedAddress, method: Option<&MethodProjection>) -> String {
    method.map_or_else(
        || format!("{module}.Body"),
        |method| format!("{}.Body", method.at),
    )
}

fn proven_search_address(
    module: &QualifiedAddress,
    projection: &ModuleProjectionSet,
    line_count: usize,
    hit: &ProviderSearchHit,
) -> Option<String> {
    let proven_end = prove_search_hit(projection.source_text(), line_count, hit)?;
    let from = hit.line;
    let method = containing_method(projection, from, proven_end);
    // A Method Body resolves by name and requires one declaration.
    if method.is_some_and(|method| {
        projection
            .methods()
            .iter()
            .filter(|candidate| candidate.name == method.name)
            .nth(1)
            .is_some()
    }) {
        return None;
    }
    let body = method_body_address(module, method);
    Some(if from == proven_end {
        format!("{body}#L{from}")
    } else {
        format!("{body}#L{from}-L{proven_end}")
    })
}

fn prove_search_hit(source: &str, line_count: usize, hit: &ProviderSearchHit) -> Option<usize> {
    let (reported_end, proven_end, shown) = search_hit_span(hit)?;
    if reported_end > line_count {
        return None;
    }
    let start = source
        .split_inclusive('\n')
        .take(hit.line - 1)
        .map(str::len)
        .sum::<usize>();
    let end = start
        + source
            .split_inclusive('\n')
            .skip(hit.line - 1)
            .take(reported_end - hit.line + 1)
            .map(str::len)
            .sum::<usize>();
    let source = normalize_search_eol(&source[start..end]);
    let shown = normalize_search_eol(shown);
    let proven = if proven_end < reported_end {
        unique_line_prefix(&source, &shown)
    } else if hit.line == reported_end {
        source
            .find(shown.as_ref())
            .zip(shown.chars().next())
            .is_some_and(|(first, character)| {
                let next = first + character.len_utf8();
                !source[next..].contains(shown.as_ref())
            })
    } else {
        source == shown || source.strip_suffix('\n') == Some(shown.as_ref())
    };
    proven.then_some(proven_end)
}

fn search_hit_span(hit: &ProviderSearchHit) -> Option<(usize, usize, &str)> {
    let from = hit.line;
    if from == 0 {
        return None;
    }
    match hit.snippet_evidence {
        SearchSnippetEvidence::Complete => {
            let to = hit
                .end_line
                .or_else(|| from.checked_add(hit.snippet.lines().count().saturating_sub(1)))?;
            (to >= from && !hit.snippet.is_empty()).then_some((to, to, hit.snippet.as_str()))
        }
        SearchSnippetEvidence::Truncated {
            shown_bytes,
            omitted_lines,
        } => {
            let shown = hit.snippet.get(..shown_bytes)?;
            if shown.chars().all(char::is_whitespace) {
                return None;
            }
            let shown_end = from.checked_add(shown.split('\n').count() - 1)?;
            let reported_end = hit.end_line?;
            (shown_end.checked_add(omitted_lines.get())? == reported_end).then_some((
                reported_end,
                shown_end,
                shown,
            ))
        }
        SearchSnippetEvidence::Unavailable => None,
    }
}

fn normalize_search_eol(text: &str) -> Cow<'_, str> {
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn unique_line_prefix(source: &str, prefix: &str) -> bool {
    if !source
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('\n'))
    {
        return false;
    }
    !source.match_indices('\n').any(|(offset, _)| {
        source[offset + 1..]
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('\n'))
    })
}
