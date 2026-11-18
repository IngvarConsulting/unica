pub use unica_application::application;
pub use unica_domain as domain;

// Признак `test-support` собирает тестовые помощники нижнего слоя для
// крейтов над ним. Часть помощников нужна только собственным тестам, и в
// такой сборке они не используются; неиспользуемое ловит сборка пакета без
// признаков и сборка его тестов.
#[doc(hidden)]
#[cfg_attr(
    all(feature = "test-support", not(test)),
    allow(dead_code, unused_imports)
)]
pub mod composition;
#[doc(hidden)]
#[cfg_attr(
    all(feature = "test-support", not(test)),
    allow(dead_code, unused_imports)
)]
pub mod infrastructure;

pub use infrastructure::platform::run_platform_main;

/// Имена операций `unica.apply` из реестра, по которому словарь `can`
/// публикует операции узла. Вход для проверок, которые видят только
/// публичный API.
#[doc(hidden)]
pub fn registered_apply_operation_names() -> Vec<&'static str> {
    domain::apply::operation_registry().names()
}
