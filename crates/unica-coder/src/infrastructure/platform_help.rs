pub mod container;
pub mod corpus;
pub mod installation;
pub mod provider;
// Проверка против реальной установки платформы. Брифом задумана как
// интеграционный тест в `crates/unica-coder/tests/`, но этот крейт (`lib.rs`
// объявляет `pub(crate) mod infrastructure;`) не экспортирует `infrastructure`
// наружу, поэтому внешний тестовый крейт не видит `PlatformSyntaxHelpProvider`
// (проверено: `unica_coder::infrastructure::...` даёт E0603 "module
// `infrastructure` is private"). Расширять публичную видимость ради теста не
// стали — см. отчёт задачи 5. Вместо этого модуль подключён как обычный
// внутрикрейтовый тест, со своим файлом, как и требует план.
#[cfg(test)]
mod real_installation;
