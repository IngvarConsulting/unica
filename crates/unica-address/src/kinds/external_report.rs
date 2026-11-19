//! «ВнешнийОтчет», `ExternalReport`.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["ВнешнийОтчет", "ВнешнийОтчёт"],
    metadata: Some(MetadataDeclaration::new(&["ExternalReports", "ВнешниеОтчеты", "ВнешниеОтчёты"])),
    child_order: Some(&["Attribute", "TabularSection", "Form", "Template", "Command"]),
    ..KindDeclaration::new("ExternalReport")
};
