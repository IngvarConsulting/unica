//! Модули: текст, строки, тела и владельцы модулей.

use super::*;

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
        let method_body = matches!(
            suffix,
            [method, body] if is_named(method, NodeKind::Method) && is_branch(body, NodeKind::Body)
        );
        let region = !suffix.is_empty()
            && suffix
                .iter()
                .all(|segment| is_named(segment, NodeKind::Region));
        if !(module_body || method || method_body || region) {
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
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
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
        let (kind, lines) = if region {
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

    /// Inspect containment and source bytes in one retained projection, with
    /// one final source identity/cancellation fence for the entire answer.
    pub(super) fn with_body_line_projection<T>(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
        inspect: impl FnOnce(QualifiedAddress, &ModuleProjectionSet) -> T,
    ) -> Result<T, ViewError> {
        self.with_module_projection(module, |module_at, projections| {
            let body = projections
                .methods()
                .iter()
                .filter(|method| from >= method.text_from_line && to <= method.to_line)
                .min_by_key(|method| method.to_line - method.text_from_line)
                .map_or_else(
                    || format!("{module_at}.Body"),
                    |method| format!("{}.Body", method.at),
                );
            let body = QualifiedAddress::parse(&body)
                .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
            Ok(inspect(body, projections))
        })
    }

    /// One retained projection of the module that `module` names, fenced by
    /// the source identity it was admitted with: a change while `inspect`
    /// reads it refuses the whole answer instead of mixing two revisions.
    pub(super) fn with_module_projection<T>(
        &self,
        module: &QualifiedAddress,
        inspect: impl FnOnce(&QualifiedAddress, &ModuleProjectionSet) -> Result<T, ViewError>,
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
        let result = inspect(&module_at, &projections)?;
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
        self.with_module_projection(module, |_, projections| {
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
        self.with_body_line_projection(module, from, to, |body, _| body)
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
        self.with_module_projection(module, |_, projections| {
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
        self.with_module_projection(module, |_, projections| {
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

    pub fn body_for_search_hit(
        &self,
        module: &QualifiedAddress,
        from: usize,
        to: usize,
        snippet: &str,
    ) -> Result<Option<QualifiedAddress>, ViewError> {
        self.with_body_line_projection(module, from, to, |body, projection| {
            let source = projection.source_text();
            if from == 0 || to < from || to > source.lines().count() {
                return None;
            }
            let start = source
                .split_inclusive('\n')
                .take(from - 1)
                .map(str::len)
                .sum::<usize>();
            let end = source
                .split_inclusive('\n')
                .take(to)
                .map(str::len)
                .sum::<usize>();
            source[start..end].contains(snippet).then_some(body)
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
        if capability.role() == ModuleRole::WebSocketClient {
            return Err(ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "WebSocketClient source layout is not specified for platform profile 8.3.27",
            ));
        }
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
        let mut view = module_projection_view(route.at(), prefix_len, projections.as_ref())?;
        if capability.role() == ModuleRole::Common && route.at() == &module_at {
            let (synonym, limits) = self.common_module_synonym(&module_at)?;
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
                MetadataAddress::parse(
                    PLATFORM_XML_8_3_27_FORMAT_2_20,
                    &format!("{}.{name}", NodeKind::CommonModule.as_str()),
                )
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

    pub(super) fn common_module_synonym(
        &self,
        at: &QualifiedAddress,
    ) -> Result<(Option<Value>, Vec<String>), ViewError> {
        self.read_checkpoint()?;
        let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &at.logical_path())
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        let descriptor = self.read.metadata_descriptor(&owner)?;
        let text = std::str::from_utf8(&descriptor).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let properties = crate::infrastructure::family_codecs::metadata_read_codec()
            .common_module_properties(text)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let observation = properties.info("Synonym");
        let mut props = Map::new();
        let mut limits = Vec::new();
        match observation {
            Ok(Some(unica_metadata::MetaInfoPropertyValue::String(text))) if !text.is_empty() => {
                crate::infrastructure::v13_read_projection::place_prop(
                    &mut props,
                    &mut limits,
                    at,
                    "synonym",
                    &json!(text),
                    false,
                    "common module Synonym",
                );
            }
            Ok(None | Some(unica_metadata::MetaInfoPropertyValue::String(_))) => {}
            Ok(Some(_)) => unreachable!("the closed Synonym profile yields a string"),
            Err(error) => limits.push(format!(
                "properties.Synonym is unreadable: {}",
                error.message
            )),
        }
        Ok((props.remove("synonym"), limits))
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
            let owner = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
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
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error))?,
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
        if let Some(owner) = branch
            .segments()
            .first()
            .filter(|owner| owner.kind() == NodeKind::WebSocketClient)
        {
            self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient owner name is absent",
                    )
                })?,
                admitted,
            )?;
        } else if let Some(owner) = module_branch_owner(branch)? {
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
        if capability.role() == ModuleRole::WebSocketClient {
            let owner = module_at.segments().first().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "WebSocketClient owner is absent")
            })?;
            return self.ensure_owner_registered_parts(
                NodeKind::WebSocketClient.as_str(),
                owner.name().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient owner name is absent",
                    )
                })?,
                admitted,
            );
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
