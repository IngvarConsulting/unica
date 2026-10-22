//! Выбор реализаций семейных кодеков продуктовой сборкой. Остальной код
//! обращается к представлению семейства только через его порт.

pub(crate) fn dcs_codec() -> &'static dyn unica_dcs::codec::DcsCodec {
    &unica_dcs_designer::DesignerDcsCodec
}
