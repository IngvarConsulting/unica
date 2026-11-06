//! `VariantsStorage`: its typed property specification and checks.
use super::*;

pub(super) const SPEC: MetadataPropertySpec = patterned_string_property(
    "VariantsStorage",
    MetaPropertyKey::VariantsStorage,
    &[MetadataKind::Report],
    r"^SettingsStorage\.[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*$",
);

/// One same-source-set canonical target of Report.VariantsStorage.
pub fn report_variants_storage_target(
    value: &str,
) -> Result<unica_address::source_target::MetadataAddress, MetaDiagnostic> {
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    let invalid = || {
        MetaDiagnostic::error(
            MetaDiagnosticCode::InvalidArguments,
            "VariantsStorage must be SettingsStorage.<Name>, a canonical same-source-set reference",
        )
    };
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value).map_err(|_| invalid())?;
    let parts: Vec<_> = target.segments().collect();
    if !matches!(parts.as_slice(), ["SettingsStorage", name] if crate::identifier::is_1c_identifier(name))
        || target.as_str() != value
    {
        return Err(invalid());
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::MetadataKind;

    #[test]
    fn report_variants_storage_accepts_only_canonical_settings_storage_targets() {
        for kind in MetadataKind::ALL {
            let converted = MetaPropertyChanges::convert(
                *kind,
                vec![MetaPropertyInput::new(
                    "VariantsStorage",
                    MetaPropertyValue::String("SettingsStorage.StorageA".into()),
                )],
            );
            assert_eq!(
                converted.is_ok(),
                *kind == MetadataKind::Report,
                "{kind:?}: {converted:?}"
            );
        }
        for target in [
            "",
            "Catalog.StorageA",
            "main:SettingsStorage.StorageA",
            "other:SettingsStorage.StorageA",
            "SettingsStorage.StorageA.Form.Main",
            "SettingsStorage.Bad Name",
        ] {
            assert!(
                MetaPropertyChanges::convert(
                    MetadataKind::Report,
                    vec![MetaPropertyInput::new(
                        "VariantsStorage",
                        MetaPropertyValue::String(target.into())
                    )]
                )
                .is_err(),
                "{target}"
            );
        }
    }
}
