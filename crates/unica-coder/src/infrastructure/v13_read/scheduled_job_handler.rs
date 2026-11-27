//! Resolve a scheduled job's observed method within its retained source set.
use super::*;

impl LogicalViewReadAuthority<'_> {
    pub(super) fn with_scheduled_job_handler(
        &self,
        projected: NodeViewData,
        at: &QualifiedAddress,
        handler: Option<&Value>,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if !matches!(at.segments(), [owner] if owner.kind() == NodeKind::ScheduledJob && owner.name().is_some())
        {
            return Ok(projected);
        }
        let Some(method) = handler else {
            return Ok(projected);
        };
        let (Some(owner), Some(name)) = (
            method.get("metadataPath").and_then(Value::as_str),
            method.get("method").and_then(Value::as_str),
        ) else {
            return Ok(projected);
        };
        let module =
            QualifiedAddress::parse(&format!("{}:{owner}", at.source_set())).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        let resolve = || -> Result<Option<String>, ViewError> {
            let route = route_logical_address(&module).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            let capability = route.module().ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "scheduled job handler has no module capability",
                )
            })?;
            self.verify_module_owner(&module, capability, admitted)?;
            let projection = self.module_projection(&module, capability, admitted)?;
            let folded = name.to_lowercase();
            let mut matches = projection
                .methods()
                .iter()
                .filter(|method| method.name.to_lowercase() == folded);
            let first = matches.next();
            Ok(first
                .filter(|_| matches.next().is_none())
                .map(|method| method.at.clone()))
        };
        match resolve() {
            Ok(Some(target))
                if crate::infrastructure::v13_read_projection::safe_prop(
                    "implementationAt",
                    &json!(target),
                ) =>
            {
                Ok(projected.with_prop("implementationAt", json!(target)))
            }
            Ok(_) => Ok(projected.with_additional_limits(vec![
                "scheduled job handler has no unique readable method".into(),
            ])),
            Err(error)
                if error.code() == RefusalCode::NotFound
                    || error.detail() == Some(RefusalDetail::SourceUnreadable)
                    || (error.code().outcome_depends_on_detail() && error.detail().is_none()) =>
            {
                Ok(projected.with_additional_limits(vec![format!(
                    "scheduled job handler is unavailable: {error}"
                )]))
            }
            Err(error) => Err(error),
        }
    }
}
