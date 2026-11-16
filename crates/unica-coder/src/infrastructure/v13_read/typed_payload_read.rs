//! Типизированные данные метаданных, их владельцы и заимствование.

use super::*;

impl<'a> LogicalViewReadAuthority<'a> {
    /// A capability comes from the same retained owner proof as its value,
    /// never from a truncated property projection or a guessed object name.
    pub fn style_item_properties_observation(
        &self,
        at: &QualifiedAddress,
    ) -> Result<Option<unica_metadata::style_color::StyleItemPropertiesObservation>, ViewError>
    {
        let [owner] = at.segments() else {
            return Ok(None);
        };
        if owner.kind() != NodeKind::StyleItem {
            return Ok(None);
        }
        let Some(name) = owner.name() else {
            return Ok(None);
        };
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let route = route_logical_address(&canonical, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let Some(target) = route.reader_metadata_path() else {
            return Ok(None);
        };
        self.verify_registered_owner(target, &admitted)?;
        let parent_version = self
            .read
            .configuration_root_version_literal(&|| self.read_checkpoint())?;
        if unica_source_designer::format_profile::writable_profile_refusal(
            "Configuration.xml",
            parent_version.as_deref(),
        )
        .is_some()
        {
            return Ok(Some(
                unica_metadata::style_color::StyleItemPropertiesObservation {
                    comment: false,
                    color: None,
                },
            ));
        }
        let raw = self.read.metadata_descriptor(target)?;
        self.read_checkpoint()?;
        let text = std::str::from_utf8(&raw).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let relative = unica_source_designer::layout::metadata_descriptor_relative(
            target,
            self.read.source_set_kind(),
        )
        .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        let observed = crate::infrastructure::family_codecs::metadata_read_codec()
            .style_properties(text, name, &relative)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        self.read_checkpoint()?;
        Ok(Some(observed))
    }

    pub fn object_borrowing(
        &self,
        at: &QualifiedAddress,
    ) -> Result<Option<unica_metadata::borrowing::MetaBorrowing>, ViewError> {
        if self.read.source_set_kind() != SourceSetKind::Extension
            || at.segments().len() != 1
            || at.segments()[0].name().is_none()
            || !at.segments()[0].kind().is_metadata_kind()
        {
            return Ok(None);
        }
        let target = MetadataAddress::parse_logical(
            at.to_string().split_once(':').expect("qualified address").1,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()))?;
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        self.verify_registered_owner(&target, &admitted)?;
        crate::infrastructure::family_codecs::metadata_read_codec()
            .borrowing(&self.read.metadata_descriptor(&target)?)
            .map(Some)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Resolve only registered, readable objects under this retained source.
    /// A matching filename or name alone is not parent identity evidence.
    pub fn borrowed_parent_matches(
        &self,
        parent_uuid: &str,
        kind: &str,
    ) -> Result<Vec<String>, ViewError> {
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        let payload = self
            .read
            .configuration_payload_with_checkpoint(&mut || self.read_checkpoint())?;
        let mut matches = Vec::new();
        for item in payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if item.get("kind").and_then(Value::as_str) != Some(kind) {
                continue;
            }
            self.read_checkpoint()?;
            let name = item.get("name").and_then(Value::as_str).ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "registered parent has no name",
                )
            })?;
            let target =
                MetadataAddress::parse_logical(&format!("{kind}.{name}")).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            self.verify_registered_owner(&target, &admitted)?;
            let bytes = self.read.metadata_descriptor(&target)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "parent descriptor is not UTF-8",
                )
            })?;
            let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(
                |error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string()),
            )?;
            let uuid = doc
                .root_element()
                .children()
                .find(|node| node.is_element())
                .and_then(|node| node.attribute("uuid"));
            if uuid.is_some_and(|uuid| uuid.eq_ignore_ascii_case(parent_uuid)) {
                let at = format!("{}:{}", self.read.source_set(), target.as_str());
                let address = QualifiedAddress::parse(&at).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                // A valid registered descriptor must also be readable through the public reader.
                self.read_exact(&address, &admitted)?;
                matches.push(at);
            }
        }
        if self.exact_revision()? != admitted.revision {
            return Err(ViewError::new(
                RefusalCode::StaleRevision,
                "parent source changed during identity resolution",
            ));
        }
        Ok(matches)
    }

    /// Whether the source set declares the node: the same typed read as `view`.
    pub(crate) fn node_exists(&self, at: &QualifiedAddress) -> bool {
        self.snapshot(at)
            .and_then(|admitted| {
                let canonical = self.canonical_address(at, &admitted)?;
                self.read_exact(&canonical, &admitted)
            })
            .is_ok()
    }

    pub(super) fn typed_payload(&self, route: &LogicalTreeRoute) -> Result<Value, ViewError> {
        let admitted = ViewSourceSnapshot {
            source_set_identity: self.read.source_set_identity().to_string(),
            revision: self.exact_revision()?,
        };
        self.typed_payload_for(route, &admitted)
    }

    pub(super) fn typed_payload_for(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if !matches!(
            route.reader(),
            LogicalReader::Configuration | LogicalReader::Module
        ) {
            if let Some(target) = route.reader_metadata_path() {
                self.verify_registered_owner(target, admitted)?;
                // The companion has its own changing bytes; an owner payload
                // cached by admission identity cannot describe this observation.
                if matches!(route.at().segments(), [owner, schedule]
                    if owner.kind() == NodeKind::ScheduledJob && schedule.kind() == NodeKind::Schedule)
                {
                    let schedule = self
                        .read
                        .job_schedule(target, &mut || self.read_checkpoint())?;
                    return Ok(json!({"properties": schedule}));
                }
                let key = TypedPayloadCacheKey {
                    revision: RevisionCacheKey {
                        source_set_identity: admitted.source_set_identity.clone(),
                        revision: admitted.revision.clone(),
                    },
                    reader: route.reader(),
                    target: target.as_str().to_string(),
                    // Кэш ключуется адресом предмета, а содержимое ячеек
                    // читается тем же предметом другим разбором. Без этого
                    // различия на запрос с текстом вернулась бы нагрузка
                    // без текста, отложенная структурным чтением.
                    detail: match route.reader() {
                        LogicalReader::Xdto => {
                            named_segment(route.at(), NodeKind::Type).map(str::to_string)
                        }
                        LogicalReader::Mxl => mxl_content_requested(route)
                            .then(|| NodeKind::Body.as_str().to_string()),
                        // Вложенная подсистема — другой файл под той же
                        // корневой целью.
                        LogicalReader::Subsystem | LogicalReader::Interface => {
                            (!route.nested_subsystems().is_empty())
                                .then(|| route.nested_subsystems().join("."))
                        }
                        _ => None,
                    },
                };
                let mut cache = self.typed_payloads.lock().map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::CachePoisoned,
                        "typed payload cache is poisoned",
                    )
                })?;
                if let Some(payload) = cache.get(&key) {
                    return Ok(payload.as_ref().clone());
                }
                let payload = Arc::new(self.uncached_typed_payload_for(route, admitted)?);
                cache.insert(key, Arc::clone(&payload));
                return Ok(payload.as_ref().clone());
            }
        }
        self.uncached_typed_payload_for(route, admitted)
    }

    pub(super) fn uncached_typed_payload_for(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if route.reader() == LogicalReader::Configuration {
            // Корень доказывает только `Configuration.xml`: описание
            // объекта проверяют прямое чтение и ветка его вида.
            let payload = self.configuration_payload(admitted)?;
            return Ok(payload.as_ref().clone());
        }
        if route.reader() == LogicalReader::Metadata {
            return self.metadata_payload(route, admitted);
        }
        if route.reader() == LogicalReader::Form {
            return self.read.form_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "form route has no typed target",
                    )
                })?,
                Some(self.platform_profile().line()),
                &mut || self.read_checkpoint(),
            );
        }
        if matches!(route.reader(), LogicalReader::Dcs | LogicalReader::Mxl) {
            if let Some(target) = route
                .reader_metadata_path()
                .filter(|target| target.as_str().starts_with("CommonTemplate."))
            {
                let expected = if route.reader() == LogicalReader::Dcs {
                    MetadataTemplateType::DataCompositionSchema
                } else {
                    MetadataTemplateType::SpreadsheetDocument
                };
                if self.read.metadata_child_profile(target)?
                    != MetadataChildProfile::Template(expected)
                {
                    return Err(ViewError::new(
                        RefusalCode::NotFound,
                        "template kind has no requested interior",
                    ));
                }
            }
        }
        if route.reader() == LogicalReader::Dcs {
            return self.read.dcs_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "DCS route has no typed target",
                    )
                })?,
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Role {
            return self.read.role_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "role route has no typed target",
                    )
                })?,
                &mut || self.read_checkpoint(),
            );
        }
        if matches!(
            route.reader(),
            LogicalReader::Subsystem | LogicalReader::Interface
        ) {
            let target = route.reader_metadata_path().ok_or_else(|| {
                ViewError::new(
                    RefusalCode::ProviderUnavailable,
                    "subsystem route has no typed target",
                )
            })?;
            let mut payload =
                self.read
                    .subsystem_payload(target, route.nested_subsystems(), &mut || {
                        self.read_checkpoint()
                    })?;
            // Корень библиотеки — подсистема верхнего уровня; признак держится
            // на соглашении стандартов разработки и называет свой источник.
            if route.reader() == LogicalReader::Subsystem && route.nested_subsystems().is_empty() {
                let name = target.as_str().split_once('.').map_or("", |(_, name)| name);
                match self.read.subsystem_topology(&mut || self.read_checkpoint()) {
                    Ok(topology) => {
                        if let Some(root) = topology
                            .library_roots()
                            .into_iter()
                            .find(|root| root.address.as_str() == name)
                        {
                            payload["libraryRoot"] = json!({
                                "source": "standardLibraryModule",
                                "module": format!("{}:{}", self.read.source_set(), root.module.as_str()),
                            });
                        }
                    }
                    // Признак корня неизвестен — это предел узла, а не «не корень».
                    Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                        let limit = format!("libraryRoot не прочитано: {error}");
                        match payload
                            .get_mut("propertyLimits")
                            .and_then(Value::as_array_mut)
                        {
                            Some(limits) => limits.push(json!(limit)),
                            None => payload["propertyLimits"] = json!([limit]),
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            return Ok(payload);
        }
        if route.reader() == LogicalReader::Mxl {
            return self.read.mxl_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "MXL route has no typed target",
                    )
                })?,
                mxl_content_requested(route),
                &mut || self.read_checkpoint(),
            );
        }
        if route.reader() == LogicalReader::Xdto {
            return self.read.xdto_payload(
                route.reader_metadata_path().ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "XDTO route has no typed target",
                    )
                })?,
                named_segment(route.at(), NodeKind::Type),
            );
        }
        Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "logical reader requires its dedicated retained adapter",
        ))
    }

    pub(super) fn configuration_payload(
        &self,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<Value>, ViewError> {
        if admitted.source_set_identity != self.read.source_set_identity() {
            return Err(ViewError::new(
                RefusalCode::StaleCursor,
                "configuration payload belongs to another source identity",
            ));
        }
        let key = RevisionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
        };
        let mut cache = self.configuration_payloads.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "configuration cache is poisoned",
            )
        })?;
        if let Some(payload) = cache.get(&key) {
            return Ok(Arc::clone(payload));
        }
        let mut checkpoint = || self.read_checkpoint();
        let payload = self
            .read
            .configuration_payload_with_checkpoint(&mut checkpoint)?;
        cache.insert(key, Arc::clone(&payload));
        Ok(payload)
    }

    /// Подсистемы, в состав которых входит `target` (`direct` у той, что
    /// его перечисляет), и корни его библиотек по соглашению стандартов разработки.
    pub(super) fn subsystem_relations(
        &self,
        target: &MetadataAddress,
        uuid: Option<uuid::Uuid>,
    ) -> Result<(Vec<Value>, Vec<Value>), ViewError> {
        let topology = self
            .read
            .subsystem_topology(&mut || self.read_checkpoint())?;
        let memberships = topology.memberships_of(target, uuid);
        let libraries = topology
            .library_roots()
            .into_iter()
            .filter(|root| {
                memberships
                    .iter()
                    .any(|membership| membership.address == root.address)
            })
            .map(|root| json!({"subsystem": root.address.as_str(), "module": root.module.as_str()}))
            .collect();
        let memberships = memberships
            .iter()
            .map(|membership| {
                json!({"subsystem": membership.address.as_str(), "direct": membership.direct})
            })
            .collect();
        Ok((memberships, libraries))
    }

    pub(super) fn metadata_payload(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Value, ViewError> {
        if route.at().form_metadata_owner().is_some() {
            let target = route.reader_metadata_path().ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "form metadata route has no owner")
            })?;
            self.verify_registered_owner(target, admitted)?;
            return self
                .read
                .form_metadata_payload(target, &mut || self.read_checkpoint());
        }
        if matches!(
            route.at().segments(),
            [owner] if owner.kind() == NodeKind::WebSocketClient && owner.name().is_some()
        ) {
            let owner = &route.at().segments()[0];
            let name = owner.name().expect("matched named WebSocketClient owner");
            self.ensure_owner_registered_parts(NodeKind::WebSocketClient.as_str(), name, admitted)?;
            return Ok(identity_only_metadata_payload(
                NodeKind::WebSocketClient.as_str(),
                name,
            ));
        }
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                "metadata address has no typed reader target",
            )
        })?;
        self.ensure_owner_registered(target, admitted)?;
        if let Some(payload) = self
            .read
            .external_metadata_payload(target, &mut || self.read_checkpoint())?
        {
            self.verify_payload_physical_children(target, &payload, admitted)?;
            return Ok(payload);
        }
        let kind = target.as_str().split('.').next().unwrap_or_default();
        if kind == NodeKind::WsReference.as_str() {
            return self
                .read
                .wsreference_payload(target, &mut || self.read_checkpoint());
        }
        if kind == NodeKind::WebSocketClient.as_str() {
            let name = target.as_str().split('.').nth(1).unwrap_or_default();
            return Ok(identity_only_metadata_payload(kind, name));
        }
        if kind == NodeKind::CommonTemplate.as_str() {
            let facts = self.read.common_template_facts(target)?;
            let mut payload = self.read.identity_metadata_payload(target)?;
            payload["synonym"] = json!(facts.synonym);
            let property =
                json!({"key":"TemplateType", "value":facts.template_type.descriptor_value()});
            if let Some(properties) = payload["properties"].as_array_mut() {
                properties.push(property);
            } else {
                payload["properties"] = json!([property]);
            }
            if let Some(message) = facts.unreadable_synonym {
                let finding = json!({"field":"properties.Synonym", "message":message});
                if let Some(unreadable) = payload["unreadable"].as_array_mut() {
                    unreadable.push(finding);
                } else {
                    payload["unreadable"] = json!([finding]);
                }
            }
            return Ok(payload);
        }
        if MetadataKind::parse(kind).is_err() {
            return self.read.identity_metadata_payload(target);
        }
        let read = self
            .read
            .metadata_local(target, &mut || self.read_checkpoint())?;
        let support_state = read.support_state;
        let local = read.info;
        for (kind, children) in [
            (NodeKind::Form, &local.collections.forms),
            (NodeKind::Template, &local.collections.templates),
            (NodeKind::Command, &local.collections.commands),
        ] {
            for child in children {
                let child = MetadataAddress::parse_logical(&format!(
                    "{}.{}.{}",
                    target.as_str(),
                    kind.as_str(),
                    child.name
                ))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                self.verify_registered_owner(&child, admitted)?;
            }
        }
        let mut payload = Map::new();
        payload.insert("name".to_string(), json!(local.name));
        payload.insert("synonym".to_string(), json!(local.synonym));
        insert_serialized(&mut payload, "kind", &local.kind)?;
        insert_serialized(&mut payload, "details", &local.details)?;
        // Карточка и элемент коллекции говорят одним словарём состояний (#2158).
        insert_serialized(&mut payload, "support", &support_state)?;
        insert_serialized(&mut payload, "properties", &local.properties)?;
        if let Some(profile) = local.member_read_profile {
            insert_serialized(&mut payload, "memberReadProfile", &profile)?;
        }
        insert_serialized(&mut payload, "declarations", &local.declarations)?;
        insert_serialized(&mut payload, "relations", &local.relations)?;
        let collections = serde_json::to_value(&local.collections).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        payload.insert("collections".to_string(), collections);
        // Непрочитанное поле — повреждённое свойство, связь или пофактовая
        // часть вида — узел называет в `limits`, а ветвь без данных отказывает,
        // а не отвечает пустым списком: иначе `view` выдал бы непрочитанное за
        // отсутствующее. Сведения, не зависящие от поля, остаются.
        // Предупреждение о немоделируемом типе объекта тоже ограничивает
        // ответ: без него узел молчал бы о том, что `check` того же объекта
        // называет (#1933). Код в сообщении отличает его от повреждённой
        // записи.
        let mut unreadable = local
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                let field = diagnostic.field.as_deref()?;
                if diagnostic.severity == MetaDiagnosticSeverity::Warning {
                    return (diagnostic.code == MetaDiagnosticCode::ValidationWarning
                        && field.starts_with("details."))
                    .then(|| {
                        json!({
                            "field": field,
                            "message": format!("validation_warning: {}", diagnostic.message),
                        })
                    });
                }
                ((field.starts_with("collections.") && field.ends_with(".type"))
                    || (field.starts_with("collections.") && field.ends_with(".comment"))
                    || field.starts_with("details.")
                    || field.starts_with("properties.")
                    || field.starts_with("relations.")
                    || [
                        "standardAttributes",
                        "characteristics",
                        "standardTabularSections",
                    ]
                    .iter()
                    .any(|prefix| {
                        field
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
                    })
                    || field
                        .strip_prefix("collections.addressingAttributes")
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
                    || (field.starts_with("collections.tabularSections[")
                        && field.contains("].standardAttributes"))
                    || local
                        .member_read_profile
                        .is_some_and(|profile| profile.owns_diagnostic(field)))
                .then(|| {
                    json!({
                        "field": field,
                        "message": diagnostic.message,
                    })
                })
            })
            .collect::<Vec<_>>();
        // Без `WriteMode` неизвестно, подчинён ли регистр регистратору: список
        // документов-регистраторов не утверждается ни полным, ни пустым, и
        // узел с ветвью `Relation` называют это пределом. Повреждённое
        // значение уже названо диагностикой `properties.WriteMode`.
        if local.kind == MetadataKind::InformationRegister
            && !local
                .properties
                .iter()
                .any(|property| property.key == "WriteMode")
            && !unreadable
                .iter()
                .any(|entry| entry["field"] == "properties.WriteMode")
        {
            unreadable.push(json!({
                "field": "relations.registrars",
                "message": "registrar applicability is unknown because WriteMode is absent",
            }));
        }
        // Заимствование отвечает только в наборе расширения; там оно есть у
        // всякого объекта, и «своё» — такой же ответ, как «заимствовано».
        if let Some(borrowing) = read.borrowing {
            payload.extend(crate::infrastructure::v13_read_projection::borrowing_props(
                &borrowing,
            ));
        }

        // Предопределённые элементы — содержимое самого объекта, и писатель у
        // них есть. Без читателя агент, добавивший элемент, не может
        // подтвердить результат: ни счёта, ни списка, ни адреса.
        // Без известного `CodeType` владельца коды элементов не проверить:
        // ветвь предопределённых ограничена, а не выдумана и не роняет узел.
        match self.read.predefined_items(
            target,
            local.kind,
            local.predefined_code_type.as_deref(),
            &mut || self.read_checkpoint(),
        ) {
            Ok(Some(predefined)) => {
                insert_serialized(&mut payload, "predefinedItems", &predefined)?;
            }
            Ok(None) => {}
            Err(error)
                if local.predefined_code_type.is_none()
                    && error.detail() == Some(RefusalDetail::SourceUnreadable) =>
            {
                unreadable.push(json!({"field": "predefinedItems", "message": error.to_string()}));
            }
            Err(error) => return Err(error),
        }
        // Подсистемы, в состав которых входит объект, и корни его библиотек
        // по соглашению стандартов разработки (INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH). Одна
        // топология отвечает всем объектам состояния исходников; непрочитанный
        // состав подсистем — предел ветви, а не пустой ответ.
        match self.subsystem_relations(target, read.uuid) {
            Ok((memberships, libraries)) => {
                payload.insert(
                    "subsystemMemberships".to_string(),
                    Value::Array(memberships),
                );
                payload.insert("libraryRoots".to_string(), Value::Array(libraries));
            }
            Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                unreadable
                    .push(json!({"field": "relations.subsystems", "message": error.to_string()}));
            }
            Err(error) => return Err(error),
        }
        if !unreadable.is_empty() {
            payload.insert("unreadable".to_string(), Value::Array(unreadable));
        }
        Ok(Value::Object(payload))
    }

    pub(super) fn verify_payload_physical_children(
        &self,
        target: &MetadataAddress,
        payload: &Value,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        for (kind, field) in [
            (NodeKind::Form, "forms"),
            (NodeKind::Template, "templates"),
            (NodeKind::Command, "commands"),
        ] {
            for child in payload
                .get("collections")
                .and_then(|collections| collections.get(field))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = child.get("name").and_then(Value::as_str) else {
                    return Err(ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        format!("registered {field} entry has no name"),
                    ));
                };
                let child = MetadataAddress::parse_logical(&format!(
                    "{}.{}.{name}",
                    target.as_str(),
                    kind.as_str()
                ))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                self.verify_registered_owner(&child, admitted)?;
            }
        }
        Ok(())
    }

    pub(super) fn ensure_owner_registered(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        let [owner_kind, owner_name, ..] = parts.as_slice() else {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "metadata owner has no canonical kind and name",
            ));
        };
        self.ensure_owner_registered_parts(owner_kind, owner_name, admitted)
    }

    pub(super) fn ensure_owner_registered_parts(
        &self,
        owner_kind: &str,
        owner_name: &str,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        if let Some(matches) = self
            .read
            .point_external_owner_matches(owner_kind, owner_name)
        {
            return matches.then_some(()).ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    "metadata owner is outside the point-proven external artifact",
                )
            });
        }
        if matches!(
            self.read.source_set_kind(),
            SourceSetKind::Configuration | SourceSetKind::Extension
        ) && self.read.configuration_xml_requires_streaming()
        {
            let index = self.streamed_configuration_registration_index(admitted)?;
            return index
                .contains(owner_kind, owner_name, &|| self.read_checkpoint())?
                .then_some(())
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::NotFound,
                        format!("metadata owner `{owner_kind}.{owner_name}` is not registered"),
                    )
                });
        }
        let payload = self.configuration_payload(admitted)?;
        let registered = payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| {
                item.get("kind").and_then(Value::as_str) == Some(owner_kind)
                    && item.get("name").and_then(Value::as_str) == Some(owner_name)
            });
        registered.then_some(()).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("metadata owner `{owner_kind}.{owner_name}` is not registered"),
            )
        })
    }

    pub(super) fn streamed_configuration_registration_index(
        &self,
        _admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<RegistrationIndex>, ViewError> {
        self.read
            .streamed_configuration_registration_index(&|| self.read_checkpoint())
    }

    pub(super) fn verify_registered_owner(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<(), ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        if self
            .verified_owners
            .lock()
            .map_err(|_| {
                ViewError::detailed(RefusalDetail::CachePoisoned, "owner cache is poisoned")
            })?
            .contains(&key)
        {
            return Ok(());
        }
        self.ensure_owner_registered(target, admitted)?;
        let parts = target.as_str().split('.').collect::<Vec<_>>();
        if parts.len() < 2 || parts.len() % 2 != 0 {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "physical metadata owner must contain complete kind/name pairs",
            ));
        }
        let revision = key.revision.clone();
        let mut parent: Option<(MetadataAddress, Arc<PlatformXmlSourceSetOwnerEvidence>)> = None;
        for pair_count in 1..=parts.len() / 2 {
            let end = pair_count * 2;
            let current_text = parts[..end].join(".");
            let current = MetadataAddress::parse_logical(&current_text).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            if let Some((parent_address, parent_evidence)) = &parent {
                let child_kind = parts[end - 2];
                let child_name = parts[end - 1];
                let edge_key = OwnerEdgeCacheKey {
                    revision: revision.clone(),
                    parent: parent_address.as_str().to_string(),
                    child: current_text.clone(),
                };
                let mut edges = self.verified_owner_edges.lock().map_err(|_| {
                    ViewError::detailed(
                        RefusalDetail::CachePoisoned,
                        "owner edge cache is poisoned",
                    )
                })?;
                if !edges.contains(&edge_key) {
                    let registered = parent_evidence
                        .registrations()
                        .any(|(kind, name)| kind == child_kind && name == child_name);
                    if !registered {
                        return Err(ViewError::new(
                            RefusalCode::NotFound,
                            format!(
                                "metadata owner `{}` does not register `{child_kind}.{child_name}`",
                                parent_address.as_str()
                            ),
                        ));
                    }
                    edges.insert(edge_key);
                }
            }
            if pair_count == 1 && parts[0] == NodeKind::WebSocketClient.as_str() {
                if parts.len() != 2 {
                    return Err(ViewError::new(
                        RefusalCode::NotFound,
                        "WebSocketClient has no specified nested export owners",
                    ));
                }
                continue;
            }
            if pair_count > 1 && parts[end - 2] == NodeKind::Command.as_str() {
                // A command has no descriptor file: its full definition is the
                // inline `ChildObjects` registration already proven above.
                if end != parts.len() {
                    return Err(ViewError::new(
                        RefusalCode::NotFound,
                        "metadata command has no nested physical owners",
                    ));
                }
                break;
            }
            let evidence = self.owner_evidence(&current, admitted).map_err(|error| {
                if error.code() == RefusalCode::NotFound {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        format!(
                            "registered metadata owner `{}` has no descriptor",
                            current.as_str()
                        ),
                    )
                } else {
                    error
                }
            })?;
            if evidence.artifact_kind() != parts[end - 2]
                || evidence.artifact_name() != Some(parts[end - 1])
            {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    format!(
                        "metadata descriptor identity does not match `{}`",
                        current.as_str()
                    ),
                ));
            }
            parent = Some((current, evidence));
        }
        self.verified_owners
            .lock()
            .map_err(|_| {
                ViewError::detailed(RefusalDetail::CachePoisoned, "owner cache is poisoned")
            })?
            .insert(key);
        Ok(())
    }

    /// Состояние поддержки членов коллекции вида по `uuid` их уже доказанных
    /// дескрипторов. Без маркера поддержки ничего не добавляется; член, чьё
    /// состояние прочитать нельзя, называется в `limits`, а не выдаётся
    /// «не на поддержке».
    pub(super) fn member_support<'name>(
        &self,
        source_set: &str,
        kind: NodeKind,
        names: impl Iterator<Item = &'name str>,
        admitted: &ViewSourceSnapshot,
    ) -> Result<crate::infrastructure::v13_read_projection::MemberSupport, ViewError> {
        use unica_metadata::support_state::ObjectSupportState;
        let mut support = crate::infrastructure::v13_read_projection::MemberSupport::default();
        if kind == NodeKind::WebSocketClient {
            return Ok(support);
        }
        let marker = match self.read.support_marker(&mut || self.read_checkpoint()) {
            Ok(Some(marker)) => marker,
            Ok(None) => return Ok(support),
            // Нечитаемый маркер называется в `limits`; отмена, срок и
            // отравленный кеш остаются ошибкой вызова.
            Err(error) if error.detail() == Some(RefusalDetail::SourceUnreadable) => {
                support
                    .limits
                    .push(format!("support of the members is unknown: {error}"));
                for name in names {
                    support.states.insert(name.to_string(), json!("unknown"));
                }
                return Ok(support);
            }
            Err(error) => return Err(error),
        };
        for name in names {
            let owner = MetadataAddress::parse_logical(&format!("{}.{name}", kind.as_str()))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
            let evidence = self.owner_evidence(&owner, admitted)?;
            match crate::infrastructure::v13_read_port::object_support_by_uuid(
                &marker,
                evidence.artifact_uuid(),
            ) {
                Ok(ObjectSupportState::NotSupported) => {}
                Ok(state) => {
                    support.states.insert(name.to_string(), json!(state));
                }
                Err(error) => {
                    support.states.insert(name.to_string(), json!("unknown"));
                    support.limits.push(format!(
                        "support of `{}:{}` is unknown: {error}",
                        source_set,
                        owner.as_str()
                    ))
                }
            }
        }
        Ok(support)
    }

    pub(super) fn owner_evidence(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<PlatformXmlSourceSetOwnerEvidence>, ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        let mut cache = self.owner_evidence.lock().map_err(|_| {
            ViewError::detailed(
                RefusalDetail::CachePoisoned,
                "owner evidence cache is poisoned",
            )
        })?;
        if let Some(evidence) = cache.get(&key) {
            return Ok(Arc::clone(evidence));
        }
        let evidence = Arc::new(
            self.read
                .metadata_owner_evidence(target, &mut || self.read_checkpoint())?,
        );
        cache.insert(key, Arc::clone(&evidence));
        Ok(evidence)
    }
}
