//! «ВнешняяОбработка», `ExternalDataProcessor`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ВнешняяОбработка"],
    metadata: Some(MetadataDeclaration::new(&["ExternalDataProcessors", "ВнешниеОбработки"])),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("ExternalDataProcessor")
};
