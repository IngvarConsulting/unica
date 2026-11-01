use roxmltree::Node;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{
    DocumentFormReference, DocumentFormRole, MetaDiagnostic, MetaDiagnosticCode,
    MetaInfoPropertyValue, MetadataKind,
};

pub(crate) fn read(
    properties: Option<Node<'_, '_>>,
    owner: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<DocumentFormReference> {
    DocumentFormRole::ALL.into_iter().map(|role| {
        let field = format!("properties.{}", role.property_name());
        let observation = properties.map(|properties| crate::info_projection::observe_meta_info_property(
            properties, MetadataKind::Document, role.property_name(),
        )).transpose().map(Option::flatten);
        let target = match observation {
            Ok(None) => None,
            Ok(Some(MetaInfoPropertyValue::String(value))) if value.is_empty() => Some(None),
            Ok(Some(MetaInfoPropertyValue::String(value))) => {
                match MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &value) {
                    Ok(target) if target.as_str().split('.').rev().nth(1) == Some("Form")
                        && target.as_str().split('.').count() >= 4 => Some(Some(target)),
                    Ok(target) if target.as_str().starts_with("CommonForm.") => {
                        diagnostics.push(MetaDiagnostic::warning(
                            MetaDiagnosticCode::ValidationWarning,
                            "CommonForm assignment is outside the document Form relation read profile",
                        ).with_metadata_path(owner.clone()).with_field(format!("relations.documentForms.{}",role.relation_name())));
                        None
                    }
                    _ => {
                        diagnostics.push(MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "document form assignment must name a Form target",
                        ).with_metadata_path(owner.clone()).with_field(format!("relations.documentForms.{}",role.relation_name())));
                        None
                    }
                }
            }
            Ok(Some(_)) => None,
            Err(error) => {
                if !diagnostics.iter().any(|diagnostic| diagnostic.field.as_deref() == Some(&field)) {
                    diagnostics.push(MetaDiagnostic::error(error.code,error.message)
                        .with_metadata_path(owner.clone()).with_field(field));
                }
                None
            }
        };
        DocumentFormReference { role, target }
    }).collect()
}
