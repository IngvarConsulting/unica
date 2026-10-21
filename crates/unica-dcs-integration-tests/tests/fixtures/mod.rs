//! Единственное объявление входов, которые читают тесты пакета.

/// Корень общего корпуса приёмки в репозитории.
macro_rules! acceptance_workspace {
    ($relative:literal) => {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acceptance/workspace/",
            $relative
        )
    };
}

/// Схема отчёта `АнализВерсийОбъектов` из корпуса приёмки.
pub const VERSIONS_SCHEMA: &str = include_str!(acceptance_workspace!(
    "src/Reports/АнализВерсийОбъектов/Templates/ОсновнаяСхемаКомпоновкиДанных/Ext/Template.xml"
));
