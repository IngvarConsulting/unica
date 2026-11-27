//! Bind HTTP operations to methods of their own service module.
use super::*;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn http_service_view(
        &self,
        route: &LogicalTreeRoute,
        payload: Value,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let at = route.at();
        let relevant = matches!(at.segments(), [owner, template, operation]
            if owner.kind() == NodeKind::HttpService && owner.name().is_some()
                && template.kind() == NodeKind::UrlTemplate && operation.kind() == NodeKind::Method);
        let declarations = if relevant {
            let templates = payload
                .pointer("/details/details/urlTemplates")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut template_names = BTreeSet::new();
            let mut ambiguous_templates = BTreeSet::new();
            for template in templates {
                if let Some(name) = template.get("name").and_then(Value::as_str) {
                    let folded = name.to_lowercase();
                    if !template_names.insert(folded.clone()) {
                        ambiguous_templates.insert(folded);
                    }
                }
            }
            templates
                .iter()
                .flat_map(|template| {
                    let template_name = template
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let ambiguous = ambiguous_templates.contains(&template_name.to_lowercase());
                    template
                        .get("methods")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(move |method| {
                            Some((
                                format!("{}.{}", template_name, method.get("name")?.as_str()?),
                                (!ambiguous).then_some(method.get("handler")?.as_str()?.to_owned()),
                            ))
                        })
                })
                .fold(
                    BTreeMap::<String, Option<String>>::new(),
                    |mut declarations, (name, procedure)| {
                        declarations
                            .entry(name.to_lowercase())
                            .and_modify(|value| *value = None)
                            .or_insert(procedure);
                        declarations
                    },
                )
        } else {
            BTreeMap::new()
        };
        let projected =
            crate::infrastructure::v13_read_projection::project_typed_payload_for_source(
                route,
                payload,
                self.read.source_set_kind(),
            )?;
        if !relevant {
            return Ok(projected);
        }
        let resolve = |operation: &str| -> Result<(Option<String>, Vec<String>), ViewError> {
            let Some(binding) = declarations.get(&operation.to_lowercase()) else {
                return Ok((None, Vec::new()));
            };
            let Some(name) = binding else {
                return Ok((None, vec!["handler declaration is ambiguous".into()]));
            };
            if name.is_empty() {
                return Ok((None, Vec::new()));
            }
            let module = QualifiedAddress::parse(&format!(
                "{}:HTTPService.{}.Module.HTTPService",
                at.source_set(),
                at.segments()[0].name().unwrap_or_default()
            ))
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            let prove = || -> Result<Option<String>, ViewError> {
                let route = route_logical_address(&module).map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
                let capability = route.module().ok_or_else(|| {
                    ViewError::detailed(
                        RefusalDetail::SourceUnreadable,
                        "HTTP service has no module capability",
                    )
                })?;
                self.verify_module_owner(&module, capability, admitted)?;
                let projection = self.module_projection(&module, capability, admitted)?;
                let folded = name.to_lowercase();
                let mut found = projection
                    .methods()
                    .iter()
                    .filter(|method| method.name.to_lowercase() == folded);
                let first = found.next();
                Ok(first
                    .filter(|_| found.next().is_none())
                    .map(|method| method.at.clone()))
            };
            match prove() {
                Ok(Some(target))
                    if crate::infrastructure::v13_read_projection::safe_prop(
                        "implementationAt",
                        &json!(target),
                    ) =>
                {
                    Ok((Some(target), Vec::new()))
                }
                Ok(_) => Ok((
                    None,
                    vec!["HTTP procedure has no unique readable method".into()],
                )),
                Err(error)
                    if error.code() == RefusalCode::NotFound
                        || error.detail() == Some(RefusalDetail::SourceUnreadable)
                        || (error.code().outcome_depends_on_detail()
                            && error.detail().is_none()) =>
                {
                    Ok((
                        None,
                        vec![format!("HTTP procedure is unavailable: {error}")],
                    ))
                }
                Err(error) => Err(error),
            }
        };
        match projected {
            NodeViewData::Collection(collection) => Ok(NodeViewData::Collection(
                collection.try_map_items(|mut row| {
                    let address = row
                        .get("at")
                        .and_then(Value::as_str)
                        .and_then(|at| QualifiedAddress::parse(at).ok())
                        .ok_or_else(|| {
                            ViewError::detailed(
                                RefusalDetail::SourceUnreadable,
                                "HTTP operation has no address",
                            )
                        })?;
                    let (target, limits) = resolve(&format!(
                        "{}.{}",
                        address.segments()[1].name().unwrap_or_default(),
                        address.segments()[2].name().unwrap_or_default()
                    ))?;
                    if let Some(target) = target {
                        row["props"]["implementationAt"] = json!(target);
                    }
                    if !limits.is_empty() {
                        row.as_object_mut()
                            .expect("operation row")
                            .entry("limits")
                            .or_insert_with(|| json!([]))
                            .as_array_mut()
                            .expect("operation limits")
                            .extend(limits.into_iter().map(Value::String));
                    }
                    Ok::<_, ViewError>(row)
                })?,
            )),
            projected => {
                let (target, limits) = resolve(&format!(
                    "{}.{}",
                    at.segments()[1].name().unwrap_or_default(),
                    at.segments()[2].name().unwrap_or_default()
                ))?;
                let projected = projected.with_additional_limits(limits);
                Ok(match target {
                    Some(target) => projected.with_prop("implementationAt", json!(target)),
                    None => projected,
                })
            }
        }
    }
}
