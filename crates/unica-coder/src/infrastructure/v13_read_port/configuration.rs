use super::{ProviderReadAuthority, ViewError};
use crate::infrastructure::v13_large_configuration::{
    read_configuration_root, RegistrationIndex, StreamedConfigurationRoot,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use unica_address::address::NodeKind;
use unica_metadata::configuration::{CfHomePageData, CfInterfaceData};
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::SourceSetKind;
use unica_source_designer::owner::prove_already_read_source_set_owner;

impl ProviderReadAuthority {
    pub(crate) fn configuration_payload_with_checkpoint(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Arc<Value>, ViewError> {
        checkpoint()?;
        if matches!(
            self.source_set_kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            #[cfg(any(test, feature = "test-support"))]
            self.configuration_payload_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return self.external_inventory_payload(checkpoint).map(Arc::new);
        }
        self.artifacts
            .configuration_payload(&self.root, || self.parse_configuration_payload(checkpoint))
    }

    fn parse_configuration_payload(
        &self,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Value, ViewError> {
        #[cfg(any(test, feature = "test-support"))]
        self.configuration_payload_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Объявленный, но пустой набор — законное состояние, а не сбой. Без
        // этой ветки отсутствие выгрузки протекало наружу текстом `strerror`
        // («No such file or directory»), то есть файловый слой попадал в
        // логический ответ и ничего не говорил о том, что делать дальше.
        let Some(bytes) = self.read_optional_relative_whole(Path::new("Configuration.xml"))? else {
            let mut refusal = ViewError::new(
                RefusalCode::InvalidState,
                "source set is declared but its directory holds no configuration export;                  fill it with a scaffold before reading",
            );
            refusal.set_next(serde_json::json!({
                "tool": "unica.check",
                "args": {},
                "reason": "вердикт по набору и совет, чем его наполнить",
            }));
            return Err(refusal);
        };
        checkpoint()?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "Configuration.xml is not UTF-8",
            )
        })?;
        let data = crate::infrastructure::family_codecs::metadata_read_codec()
            .configuration_info(
                text,
                self.configuration_support(checkpoint)?,
                self.home_page()?,
                self.command_interface()?,
                self.panels()?,
            )
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))?;
        let owner = prove_already_read_source_set_owner(
            Path::new("Configuration.xml"),
            &bytes,
            self.source_set_kind,
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error.message))?;
        let mut payload = serde_json::to_value(data)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "configuration parser returned a non-object payload",
                )
            })?;
        payload.insert(
            "registeredObjects".to_string(),
            Value::Array(
                owner
                    .registrations()
                    .filter(|(kind, _)| NodeKind::parse(kind).is_ok_and(NodeKind::is_metadata_kind))
                    .map(|(kind, name)| json!({"kind": kind, "name": name}))
                    .collect(),
            ),
        );
        Ok(Value::Object(payload))
    }

    /// Lexical parent version without materializing a large Configuration DOM.
    pub(crate) fn configuration_root_version_literal(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Option<String>, ViewError> {
        let relative = Path::new("Configuration.xml");
        if let Some(buffer) = self.open_buffer(relative) {
            return crate::infrastructure::v13_large_configuration::read_configuration_root_version_literal(std::io::Cursor::new(buffer.text.as_bytes()),checkpoint);
        }
        let file = self
            .root
            .open_relative_regular_nofollow(relative)
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
        crate::infrastructure::v13_large_configuration::read_configuration_root_version_literal(
            file, checkpoint,
        )
    }

    pub(crate) fn streamed_configuration_root(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<StreamedConfigurationRoot, ViewError> {
        checkpoint()?;
        let file = match self
            .root
            .open_relative_regular_nofollow(Path::new("Configuration.xml"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut refusal = ViewError::new(
                    RefusalCode::InvalidState,
                    "source set is declared but its directory holds no configuration export; fill it with a scaffold before reading",
                );
                refusal.set_next(serde_json::json!({
                    "tool": "unica.check",
                    "args": {},
                    "reason": "вердикт по набору и совет, чем его наполнить",
                }));
                return Err(refusal);
            }
            Err(error) => {
                return Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    error.to_string(),
                ));
            }
        };
        let support = serde_json::to_value(self.configuration_support(&mut || checkpoint())?)
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
            })?;
        let home_page = serde_json::to_value(self.home_page()?).map_err(|error| {
            ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
        })?;
        let interface = serde_json::to_value(self.command_interface()?).map_err(|error| {
            ViewError::detailed(RefusalDetail::BackendBroken, error.to_string())
        })?;
        let panels = self.panels()?.unwrap_or(Value::Null);
        read_configuration_root(file, support, home_page, interface, panels, checkpoint)
    }

    /// Build root registration evidence from the complete retained XML once.
    /// The returned anonymous disk index can serve named reads in this source
    /// revision without retaining the inventory in process memory.
    pub(crate) fn streamed_configuration_registration_index(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Arc<RegistrationIndex>, ViewError> {
        self.artifacts.registrations(&self.root, checkpoint)
    }

    /// Командный интерфейс конфигурации читается так же, как домашняя
    /// страница: отдельным документом рядом с `Configuration.xml`, и ложится
    /// свойством корня. Отсутствие документа — законное состояние, а не отказ.
    fn command_interface(&self) -> Result<Option<CfInterfaceData>, ViewError> {
        let Some(bytes) =
            self.read_optional_relative_whole(Path::new("Ext/CommandInterface.xml"))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "CommandInterface.xml is not UTF-8",
            )
        })?;
        crate::infrastructure::family_codecs::metadata_read_codec()
            .configuration_command_interface(text)
            .map(Some)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Раскладка панелей корня в форме, которую принимает `panels.set`.
    fn panels(&self) -> Result<Option<Value>, ViewError> {
        let Some(bytes) = self.read_optional_relative_whole(Path::new(
            crate::infrastructure::family_codecs::metadata_read_codec().client_interface_relative(),
        ))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "ClientApplicationInterface.xml is not UTF-8",
            )
        })?;
        crate::infrastructure::family_codecs::metadata_read_codec()
            .client_panels(text)
            .map(Some)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    fn home_page(&self) -> Result<Option<CfHomePageData>, ViewError> {
        let Some(bytes) =
            self.read_optional_relative_whole(Path::new("Ext/HomePageWorkArea.xml"))?
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "HomePageWorkArea.xml is not UTF-8",
            )
        })?;
        crate::infrastructure::family_codecs::metadata_read_codec()
            .configuration_home_page(text)
            .map(Some)
            .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }
}
