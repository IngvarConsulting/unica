use super::{
    descriptor_uuid, metadata_support_status, MetadataLocalRead, ProviderReadAuthority, ViewError,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use unica_address::source_target::MetadataAddress;
use unica_metadata::{MetaPredefinedItemsData, MetadataKind};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::SourceSetKind;
use unica_source_designer::owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};

impl ProviderReadAuthority {
    /// Локальное чтение объекта: разбор дескриптора и заимствование берутся из
    /// одних и тех же байтов.
    ///
    /// Дескриптор открывается один раз. Второе чтение того же файла стоило бы
    /// лишнего доступа и, что хуже, могло бы застать файл изменившимся между
    /// чтениями — тогда свойства объекта и его заимствование описывали бы
    /// разные состояния источника.
    pub(crate) fn metadata_local(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<MetadataLocalRead, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let support_data =
            self.object_support_with_descriptor(target, Some(&descriptor), checkpoint)?;
        let support_state = support_data.state;
        let support = metadata_support_status(support_data);
        // Заимствование отвечает только в наборе вида `extension`: в
        // конфигурации его не бывает, и разбирать дескриптор ради `own` у
        // каждого объекта незачем.
        let borrowing = if self.source_set_kind == SourceSetKind::Extension {
            Some(
                crate::infrastructure::family_codecs::metadata_read_codec()
                    .borrowing(&descriptor)
                    .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?,
            )
        } else {
            None
        };
        let mut info = crate::infrastructure::family_codecs::metadata_read_codec()
            .local_info(&descriptor, target, support, self.source_set_kind)
            .map_err(|failure| {
                let message = serde_json::to_string(&failure.diagnostics)
                    .unwrap_or_else(|_| "metadata reader failed".to_string());
                ViewError::detailed(RefusalDetail::SourceUnreadable, message)
            })?;
        if info.kind == MetadataKind::ExchangePlan {
            let relative = Path::new("ExchangePlans")
                .join(&info.name)
                .join("Ext/Content.xml");
            let observation = match self
                .read_optional_relative_with_checkpoint(&relative, checkpoint)
            {
                Ok(Some(bytes)) => std::str::from_utf8(&bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|text| {
                        crate::infrastructure::family_codecs::metadata_read_codec()
                            .exchange_content(text)
                    })
                    .map(Some),
                Ok(None) => Ok(None),
                Err(error) if matches!(error.detail(), Some(RefusalDetail::SourceUnreadable)) => {
                    Err(error.to_string())
                }
                Err(error) => return Err(error),
            };
            let errors = match observation {
                Ok(Some(observation)) => {
                    info.relations.content = Some(Some(observation.items));
                    observation.errors
                }
                // Заимствованный план без своего `Ext/Content.xml` несёт состав
                // плана родителя: здесь он неизвестен, а не пуст (#1924).
                Ok(None) if borrowing.as_ref().is_some_and(|b| b.extends.is_some()) => {
                    info.relations.content = Some(None);
                    Vec::new()
                }
                // The platform omits Content.xml for an empty composition.
                Ok(None) => {
                    info.relations.content = Some(Some(Vec::new()));
                    Vec::new()
                }
                Err(error) => vec![error],
            };
            for error in errors {
                info.diagnostics.push(
                    unica_metadata::MetaDiagnostic::error(
                        unica_metadata::MetaDiagnosticCode::ValidationFailed,
                        error,
                    )
                    .with_metadata_path(target.clone())
                    .with_field("relations.content"),
                );
            }
        }
        Ok(MetadataLocalRead {
            info,
            borrowing,
            support_state,
            uuid: descriptor_uuid(&descriptor),
        })
    }

    /// Предопределённые элементы объекта.
    ///
    /// `None` — вид владельца этой коллекции не имеет, и ветви у него не будет.
    /// `Some` с нулём — файла предопределённых нет, и это доказанная пустота, а
    /// не незнание: платформа не пишет `Predefined.xml`, когда элементов нет.
    ///
    /// Разбор берётся у того же читателя, которым пользовался `meta.info`:
    /// второй разборщик того же файла расходился бы с ним молча.
    pub(crate) fn predefined_items(
        &self,
        target: &MetadataAddress,
        kind: MetadataKind,
        code_type: Option<&str>,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<MetaPredefinedItemsData>, ViewError> {
        if !unica_metadata::metadata_kind_collections(kind)
            .contains(&unica_metadata::MetaCollection::PredefinedItems)
        {
            return Ok(None);
        }
        let name = target.as_str().split('.').nth(1).unwrap_or_default();
        let relative = Path::new(
            crate::infrastructure::family_codecs::metadata_read_codec().metadata_directory(kind),
        )
        .join(name)
        .join("Ext")
        .join("Predefined.xml");
        let Some(bytes) = self.read_optional_relative_with_checkpoint(&relative, checkpoint)?
        else {
            return Ok(Some(MetaPredefinedItemsData {
                total: 0,
                returned: 0,
                truncated: false,
                items: Vec::new(),
            }));
        };
        let mut interrupted = None;
        let result = crate::infrastructure::family_codecs::metadata_read_codec().predefined_items(
            &bytes,
            kind,
            code_type,
            &mut || {
                checkpoint().map_err(|error| {
                    let message = error.to_string();
                    interrupted = Some(error);
                    message
                })
            },
        );
        if let Some(error) = interrupted {
            return Err(error);
        }
        result
            .map(Some)
            .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))
    }

    pub(crate) fn identity_metadata_payload(
        &self,
        target: &MetadataAddress,
    ) -> Result<Value, ViewError> {
        let descriptor = self.metadata_descriptor(target)?;
        let relative = self.metadata_descriptor_relative(target)?;
        let evidence = prove_already_read_metadata_owner(&relative, &descriptor)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        let mut payload = identity_metadata_payload_from_evidence(target, &evidence)?;
        if evidence.artifact_kind() == "Language" {
            match crate::infrastructure::family_codecs::metadata_read_codec()
                .language_code(&descriptor)
            {
                Ok(code) => {
                    payload["properties"] = json!([{
                        "key": "LanguageCode",
                        "value": unica_metadata::MetaObservedPropertyValue::Text { value: code },
                    }]);
                }
                Err(message) => {
                    payload["unreadable"] = json!([{
                        "field": "properties.LanguageCode",
                        "message": message,
                    }]);
                }
            }
        }
        if evidence.artifact_kind() == "DocumentNumerator" {
            match crate::infrastructure::family_codecs::metadata_read_codec().numerator(&descriptor)
            {
                Ok(facts) => {
                    let mut properties = payload["properties"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    properties.extend(facts.properties.into_iter().map(|property| {
                        json!({
                            "key": property.name,
                            "value": property.value,
                        })
                    }));
                    payload["properties"] = json!(properties);
                    if !facts.unreadable.is_empty() {
                        payload["unreadable"] = json!(facts
                            .unreadable
                            .into_iter()
                            .map(|error| json!({
                                "field": error.field,
                                "message": error.message,
                            }))
                            .collect::<Vec<_>>());
                    }
                }
                Err(message) => {
                    payload["unreadable"] = json!([{
                        "field": "properties",
                        "message": message,
                    }])
                }
            }
        }
        if evidence.artifact_kind() == "WSReference" {
            match crate::infrastructure::family_codecs::metadata_read_codec()
                .ws_reference_location(&descriptor)
            {
                Ok(Some(value)) => {
                    payload["properties"] = json!([{
                        "key": "LocationURL",
                        "value": unica_metadata::MetaObservedPropertyValue::Text { value },
                    }])
                }
                Ok(None) => {}
                Err(message) => {
                    payload["unreadable"] =
                        json!([{"field":"properties.LocationURL","message":message}])
                }
            }
        }
        if evidence.artifact_kind() == "FunctionalOption" {
            let name = target.as_str().split('.').nth(1).unwrap_or_default();
            match std::str::from_utf8(&descriptor)
                .map_err(|error| error.to_string())
                .and_then(|source| {
                    crate::infrastructure::family_codecs::metadata_read_codec()
                        .functional_option(source, name)
                }) {
                Ok(option) => {
                    if !payload["properties"].is_array() {
                        payload["properties"] = json!([]);
                    }
                    let properties = payload["properties"]
                        .as_array_mut()
                        .expect("identity property array");
                    properties.push(json!({"key":"Location", "value":unica_metadata::MetaObservedPropertyValue::Text { value: option.location }}));
                    properties.push(json!({"key":"PrivilegedGetMode", "value":unica_metadata::MetaObservedPropertyValue::Boolean { value: option.privileged_get_mode }}));
                }
                Err(message) => {
                    payload["unreadable"] = json!([{"field":"properties", "message":message}])
                }
            }
        }
        if evidence.artifact_kind() == "StyleItem" {
            crate::infrastructure::family_codecs::metadata_read_codec()
                .style_item(&descriptor, &mut payload)
                .map_err(|message| ViewError::detailed(RefusalDetail::SourceUnreadable, message))?;
        }
        {
            match crate::infrastructure::family_codecs::metadata_read_codec()
                .descriptor_comment(&descriptor, evidence.artifact_kind())
            {
                Ok(Some(comment)) => {
                    let property = json!({
                        "key": "Comment",
                        "value": unica_metadata::MetaObservedPropertyValue::Text { value: comment },
                    });
                    if let Some(properties) = payload["properties"].as_array_mut() {
                        properties.push(property);
                    } else {
                        payload["properties"] = json!([property]);
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    let finding = json!({
                        "field": "properties.Comment",
                        "message": message,
                    });
                    if let Some(unreadable) = payload["unreadable"].as_array_mut() {
                        unreadable.push(finding);
                    } else {
                        payload["unreadable"] = json!([finding]);
                    }
                }
            }
        }
        Ok(payload)
    }

    pub(crate) fn metadata_owner_evidence(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<PlatformXmlSourceSetOwnerEvidence, ViewError> {
        // The actor retains the proof under the stamp the descriptor had
        // before it was read (INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE).
        // Without a stamp the descriptor is read as before, with its own
        // refusals and checkpoints. An external set finds its descriptor
        // through its inventory under the caller's checkpoint: it keeps that
        // path.
        if self.is_external_source_set() {
            return self.metadata_owner_evidence_uncached(target, checkpoint);
        }
        let Some((relative, stamp)) = self
            .metadata_descriptor_relative(target)
            .ok()
            .and_then(|relative| Some((relative.clone(), self.descriptor_stamp(&relative)?)))
        else {
            return self.metadata_owner_evidence_uncached(target, checkpoint);
        };
        if let Some(evidence) = self.artifacts.retained_owner_evidence(&relative, &stamp) {
            checkpoint()?;
            return Ok((*evidence).clone());
        }
        let evidence = self.metadata_owner_evidence_uncached(target, checkpoint)?;
        self.artifacts
            .retain_owner_evidence(relative, stamp, Arc::new(evidence.clone()));
        Ok(evidence)
    }

    /// The synonym of a common module for its collection row and its node,
    /// with the limits naming a damaged or oversized one. The actor retains
    /// the synonym as read, under the descriptor's stamp before the read; the
    /// size limit of the answer is applied on every use, since a read of the
    /// property itself takes it whole.
    pub(crate) fn common_module_synonym(
        &self,
        at: &unica_address::address::QualifiedAddress,
    ) -> Result<(Option<Value>, Vec<String>), ViewError> {
        let owner = MetadataAddress::parse_logical(&at.logical_path()).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let observed = if self.is_external_source_set() {
            self.common_module_synonym_observed(&owner)?
        } else if let Some((relative, stamp)) = self
            .metadata_descriptor_relative(&owner)
            .ok()
            .and_then(|relative| Some((relative.clone(), self.descriptor_stamp(&relative)?)))
        {
            match self
                .artifacts
                .retained_common_module_synonym(&relative, &stamp)
            {
                Some(observed) => (*observed).clone(),
                None => {
                    let observed = self.common_module_synonym_observed(&owner)?;
                    self.artifacts.retain_common_module_synonym(
                        relative,
                        stamp,
                        Arc::new(observed.clone()),
                    );
                    observed
                }
            }
        } else {
            self.common_module_synonym_observed(&owner)?
        };
        let mut props = serde_json::Map::new();
        let mut limits = Vec::new();
        match observed {
            Ok(Some(text)) => crate::infrastructure::v13_read_projection::place_prop(
                &mut props,
                &mut limits,
                at,
                "synonym",
                &json!(text),
                false,
                "common module Synonym",
            ),
            Ok(None) => {}
            Err(message) => limits.push(format!("properties.Synonym is unreadable: {message}")),
        }
        Ok((props.remove("synonym"), limits))
    }

    /// The synonym as the descriptor states it: `Ok(None)` when it is absent
    /// or empty, `Err` with the reason when the property cannot be read.
    fn common_module_synonym_observed(
        &self,
        owner: &MetadataAddress,
    ) -> Result<crate::infrastructure::v13_large_configuration::CommonModuleSynonym, ViewError>
    {
        let descriptor = self.metadata_descriptor(owner)?;
        let text = std::str::from_utf8(&descriptor).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        let properties = crate::infrastructure::family_codecs::metadata_read_codec()
            .common_module_properties(text)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        Ok(match properties.info("Synonym") {
            Ok(Some(unica_metadata::MetaInfoPropertyValue::String(text))) if !text.is_empty() => {
                Ok(Some(text))
            }
            Ok(None | Some(unica_metadata::MetaInfoPropertyValue::String(_))) => Ok(None),
            Ok(Some(_)) => unreachable!("the closed Synonym profile yields a string"),
            Err(error) => Err(error.message),
        })
    }

    fn metadata_owner_evidence_uncached(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<PlatformXmlSourceSetOwnerEvidence, ViewError> {
        let external_root =
            self.is_external_source_set() && target.as_str().split('.').count() == 2;
        let descriptor = self.metadata_descriptor_with_checkpoint(target, checkpoint)?;
        let relative = self.metadata_descriptor_relative(target)?;
        if external_root {
            prove_already_read_source_set_owner(&relative, &descriptor, self.source_set_kind)
                .map_err(|error| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
                })
        } else {
            prove_already_read_metadata_owner(&relative, &descriptor).map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.message)
            })
        }
    }
}

pub(super) fn identity_metadata_payload_from_evidence(
    target: &MetadataAddress,
    evidence: &PlatformXmlSourceSetOwnerEvidence,
) -> Result<Value, ViewError> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let [expected_kind, expected_name] = parts.as_slice() else {
        return Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            "identity metadata target must name one owner",
        ));
    };
    if evidence.artifact_kind() != *expected_kind
        || evidence.artifact_name() != Some(*expected_name)
    {
        return Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            format!(
                "metadata descriptor identity does not match `{}`",
                target.as_str()
            ),
        ));
    }
    let mut forms = Vec::new();
    let mut templates = Vec::new();
    let mut commands = Vec::new();
    for (kind, name) in evidence.registrations() {
        let value = json!({"name": name});
        match kind {
            "Form" => forms.push(value),
            "Template" => templates.push(value),
            "Command" => commands.push(value),
            _ => {}
        }
    }
    Ok(json!({
        "name": expected_name,
        "kind": expected_kind,
        "support": {"state": "not_supported"},
        "properties": {},
        "declarations": {},
        "relations": {},
        "collections": {
            "attributes": [],
            "tabularSections": [],
            "dimensions": [],
            "resources": [],
            "enumValues": [],
            "columns": [],
            "forms": forms,
            "templates": templates,
            "commands": commands
        }
    }))
}
