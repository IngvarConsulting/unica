//! Выбор реализаций семейных кодеков продуктовой сборкой. Остальной код
//! обращается к представлению семейства только через его порт.

pub(crate) fn dcs_codec() -> &'static dyn unica_dcs::codec::DcsCodec {
    &unica_dcs_designer::DesignerDcsCodec
}

pub fn metadata_read_codec() -> &'static dyn unica_metadata::read_codec::MetadataReadCodec {
    &unica_metadata_designer::read_codec::DesignerMetadataReadCodec
}

/// Читатель маркера поддержки набора для кодеков других семейств.
pub(crate) const SUPPORT_MARKER_READER: unica_metadata::support_state::SupportMarkerReader =
    unica_metadata_designer::support::parse_support_state_compat_bytes;
