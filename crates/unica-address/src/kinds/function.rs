//! «Функция», `Function`: функция внешнего источника данных.
use super::*;

pub(super) const KIND: KindDeclaration = KindDeclaration {
    russian: &["Функция"],
    metadata: Some(MetadataDeclaration::new(&["Functions", "Функции"])),
    nested: Some(NestedPlace {
        owners: &["ExternalDataSource"],
        directory: None,
        modules: &[],
    }),
    ..KindDeclaration::new("Function")
};
