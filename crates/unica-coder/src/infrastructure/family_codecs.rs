//! Выбор реализаций семейных кодеков продуктовой сборкой. Остальной код
//! обращается к представлению семейства только через его порт.

pub(crate) fn dcs_codec() -> &'static dyn unica_dcs::codec::DcsCodec {
    &unica_dcs_designer::DesignerDcsCodec
}

pub fn metadata_read_codec() -> &'static dyn unica_metadata::read_codec::MetadataReadCodec {
    &unica_metadata_designer::read_codec::DesignerMetadataReadCodec
}
