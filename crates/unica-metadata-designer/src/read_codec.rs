//! Designer-кодек за портом чтения метаданных: разбор дескрипторов выгрузки
//! Designer для [`MetadataReadCodec`].
use serde_json::{Map, Value};
use std::path::Path;
use unica_address::source_target::MetadataAddress;
use unica_metadata::read_codec::{
    ExchangeContentFacts, FunctionalOptionFacts, MetadataReadCodec, NumeratorFacts, ReadFinding,
    ReadStopped,
};
use unica_metadata::style_color::StyleItemPropertiesObservation;
use unica_metadata::wsdl::Definition;

pub struct DesignerMetadataReadCodec;

impl MetadataReadCodec for DesignerMetadataReadCodec {
    fn job_schedule(&self, text: &str) -> Result<Value, String> {
        crate::job_schedule::parse_job_schedule(text)
    }

    fn ws_reference_location(&self, descriptor: &[u8]) -> Result<Option<String>, String> {
        crate::ws_reference::read_location_url(descriptor)
    }

    fn descriptor_comment(&self, descriptor: &[u8], kind: &str) -> Result<Option<String>, String> {
        crate::descriptor_comment::read_comment(descriptor, kind)
    }

    fn language_code(&self, descriptor: &[u8]) -> Result<String, String> {
        crate::language_code::read_code(descriptor)
    }

    fn functional_option(
        &self,
        descriptor: &str,
        name: &str,
    ) -> Result<FunctionalOptionFacts, String> {
        crate::functional_option::read(descriptor, name).map(|(location, privileged_get_mode)| {
            FunctionalOptionFacts {
                location,
                privileged_get_mode,
            }
        })
    }

    fn numerator(&self, descriptor: &[u8]) -> Result<NumeratorFacts, String> {
        crate::numerator_read::read(descriptor).map(|facts| NumeratorFacts {
            properties: facts.properties,
            unreadable: facts
                .unreadable
                .into_iter()
                .map(|error| ReadFinding {
                    field: error.field,
                    message: error.message,
                })
                .collect(),
        })
    }

    fn style_item(&self, descriptor: &[u8], payload: &mut Value) -> Result<(), String> {
        crate::style_item_read::enrich_style_item(descriptor, payload)
    }

    fn style_properties(
        &self,
        descriptor: &str,
        name: &str,
        relative: &Path,
    ) -> Result<StyleItemPropertiesObservation, String> {
        crate::style_color::observe_properties(descriptor, name, relative)
            .map_err(|issue| issue.to_string())
    }

    fn form_metadata(
        &self,
        descriptor: &[u8],
        target: &MetadataAddress,
    ) -> Result<(Map<String, Value>, Vec<String>), String> {
        crate::form_metadata::parse_form_metadata_properties(descriptor, target)
    }

    fn exchange_content(&self, text: &str) -> Result<ExchangeContentFacts, String> {
        crate::exchange_content::read_content(text).map(|observation| ExchangeContentFacts {
            items: observation.items,
            errors: observation.errors,
        })
    }

    fn wsdl(
        &self,
        bytes: &[u8],
        checkpoint: &mut dyn FnMut() -> Result<(), ReadStopped>,
    ) -> Result<Definition, ReadStopped> {
        crate::wsdl_read::read(bytes, checkpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

    #[test]
    fn functional_option_facts_keep_location_and_privileged_mode() {
        let descriptor = format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><FunctionalOption uuid=\"kept\"><Properties><Name>Option</Name><Location>Constant.Before</Location><PrivilegedGetMode>true</PrivilegedGetMode><Content>Catalog.Kept</Content></Properties></FunctionalOption></MetaDataObject>");
        assert_eq!(
            DesignerMetadataReadCodec.functional_option(&descriptor, "Option"),
            Ok(FunctionalOptionFacts {
                location: "Constant.Before".to_string(),
                privileged_get_mode: true,
            })
        );
    }

    #[test]
    fn numerator_facts_carry_each_unreadable_field_without_stopping() {
        let descriptor = format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><DocumentNumerator><Properties><Name>Probe</Name><NumberType>Unknown</NumberType><CheckUnique>true</CheckUnique></Properties></DocumentNumerator></MetaDataObject>");
        let facts = DesignerMetadataReadCodec
            .numerator(descriptor.as_bytes())
            .unwrap();
        assert_eq!(facts.properties.len(), 1);
        assert_eq!(facts.properties[0].name, "CheckUnique");
        assert_eq!(facts.unreadable.len(), 1);
        assert_eq!(facts.unreadable[0].field, "properties.NumberType");
        assert!(!facts.unreadable[0].message.is_empty());
    }

    #[test]
    fn wsdl_read_stops_when_the_caller_stops_it() {
        let mut calls = 0;
        let read = DesignerMetadataReadCodec.wsdl(b"<definitions/>", &mut || {
            calls += 1;
            Err(ReadStopped)
        });
        assert!(matches!(read, Err(ReadStopped)));
        assert_eq!(calls, 1);
    }

    #[test]
    fn wsdl_read_describes_an_unreadable_document_instead_of_failing() {
        let read = DesignerMetadataReadCodec.wsdl(b"not xml", &mut || Ok(()));
        assert!(!read
            .expect("an unreadable document is a fact")
            .limits
            .is_empty());
    }
}
