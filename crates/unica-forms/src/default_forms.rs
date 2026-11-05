//! Typed references for an owner's existing default-form slots.
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

#[derive(Debug, Clone)]
pub enum FormSlotTarget {
    Clear,
    Owned(String),
    Common(MetadataAddress),
}

pub fn common_form_slot(owner_kind: &str, property: &str) -> bool {
    matches!(
        (owner_kind, property),
        (
            "Report",
            "DefaultForm" | "DefaultSettingsForm" | "DefaultVariantForm"
        ) | ("Constant", "DefaultForm")
    )
}

impl FormSlotTarget {
    pub fn parse(
        raw: &str,
        owner: &MetadataAddress,
        source_set: &str,
        property: &str,
    ) -> Result<Self, String> {
        if raw.is_empty() {
            return Ok(Self::Clear);
        }
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("the form slot takes a form name or a logical form address".into());
        }
        if !raw.contains(':') && !raw.contains('.') {
            return Ok(Self::Owned(raw.to_string()));
        }
        let at = QualifiedAddress::resolve_input(raw, &[source_set])
            .map_err(|error| error.to_string())?;
        if at.source_set() != source_set {
            return Err("a default form must belong to the same source set as its owner".into());
        }
        if at
            .segments()
            .last()
            .is_some_and(|segment| segment.kind() == NodeKind::CommonForm)
            && at.segments().len() == 1
        {
            let kind = owner.segments().next().unwrap_or_default();
            if !common_form_slot(kind, property) {
                return Err("CommonForm is supported only by Report defaultForm/defaultSettingsForm/defaultVariantForm and Constant defaultForm".into());
            }
            return MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &at.logical_path())
                .map(Self::Common)
                .map_err(|error| error.to_string());
        }
        if at
            .segments()
            .last()
            .is_some_and(|segment| segment.kind() == NodeKind::Form)
            && at.segments().len() == 2
            && at.prefix(1).logical_path() == owner.as_str()
        {
            return at.segments()[1]
                .name()
                .map(|name| Self::Owned(name.to_string()))
                .ok_or_else(|| "a concrete form address is required".into());
        }
        Err("use a form of this owner, or a compatible CommonForm in the same source set for a supported slot".into())
    }
}
