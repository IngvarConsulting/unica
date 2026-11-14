//! Данные, привязки и представление форм.

use super::*;

impl<'a> LogicalViewReadAuthority<'a> {
    /// One parsed `Form.xml` per admitted revision and form target: every
    /// item, attribute, command and event address under the form shares it.
    pub(super) fn form_data(
        &self,
        target: &MetadataAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<FormInfoData>, ViewError> {
        let key = OwnerProofCacheKey {
            revision: RevisionCacheKey {
                source_set_identity: admitted.source_set_identity.clone(),
                revision: admitted.revision.clone(),
            },
            owner: target.as_str().to_string(),
        };
        let mut cache = self.form_data.lock().map_err(|_| {
            ViewError::detailed(RefusalDetail::CachePoisoned, "form cache is poisoned")
        })?;
        if let Some(data) = cache.get(&key) {
            return Ok(Arc::clone(data));
        }
        let data = Arc::new(self.read.form_data(
            target,
            Some(self.platform_profile().line()),
            &mut || self.read_checkpoint(),
        )?);
        cache.insert(key, Arc::clone(&data));
        Ok(data)
    }

    pub(super) fn form_bindings(
        &self,
        module_at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Vec<FormEventBindingInput>, ViewError> {
        let module_at_text = module_at.to_string();
        let form_at = module_at_text
            .rsplit_once(".Module.Form")
            .map(|(form, _)| form)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::ProviderUnavailable,
                    "form module address is invalid",
                )
            })?;
        let form = QualifiedAddress::parse(form_at).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let metadata_path =
            MetadataAddress::parse_logical(&form.logical_path()).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        if self
            .read
            .inherited_resource(&metadata_path, "Form.xml")?
            .is_some()
        {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                format!(
                    "`{module_at}` belongs to a borrowed form whose content the extension does not carry: there is no `Form.xml`, and the platform keeps no form module without it; read the parent named by props.extends of `{}`",
                    extension_object(module_at)
                ),
            ));
        }
        let data = self.form_data(&metadata_path, admitted)?;
        Ok(form_semantic_inputs(form_at, &FormEventEvidence::from_info(&data)).bindings)
    }

    pub(super) fn form_view(
        &self,
        route: &LogicalTreeRoute,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let target = route.reader_metadata_path().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "form route has no typed target",
            )
        })?;
        self.verify_registered_owner(target, admitted)?;
        let data = self.form_data(target, admitted)?;
        let form_at =
            QualifiedAddress::parse(&format!("{}:{}", route.at().source_set(), target.as_str()))
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                })?;
        let module_at =
            QualifiedAddress::parse(&format!("{form_at}.Module.Form")).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        let capability = self.profile.module_capability(&module_at).ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::ProviderAbsent,
                "form module capability is absent from the platform profile",
            )
        })?;
        self.verify_module_owner(&module_at, capability, admitted)?;
        let mut evidence = FormEventEvidence::from_info(&data);
        evidence.context.metadata_owner = route.at().segments().first().map(AddressSegment::kind);
        let inputs = form_semantic_inputs(&form_at.to_string(), &evidence);
        let module = self.module_projection(&module_at, capability, admitted)?;
        let events = project_property_events_from_projection(&evidence, &inputs, &module)
            .map_err(|error| ViewError::new(error.code(), error.to_string()))?;
        let payload = serde_json::to_value(data.as_ref()).map_err(|error| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                format!("form payload serialization failed: {error}"),
            )
        })?;
        crate::infrastructure::v13_read_projection::project_typed_payload_with_form_events(
            route, payload, &events,
        )
    }
}
