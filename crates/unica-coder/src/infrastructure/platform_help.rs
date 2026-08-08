// `#[allow(dead_code)]` глушит два разных предупреждения dead_code:
// 1) V8Container/ContainerError/parse/entry и их внутренние помощники —
//    провайдер справки подключит их как вызывающий код в одной из следующих
//    задач плана, предупреждение снимется само собой;
// 2) tests_support::container_without_file_storage — фикстурный хелпер для
//    тестов следующей задачи; в тестах этой задачи не вызывается.
#[allow(dead_code)]
pub mod container;
// `#[allow(dead_code)]` здесь по той же причине, что и на `container` выше:
// CorpusPage/Signature/CorpusError/read_corpus/read_corpus_from_archive и их
// внутренние помощники ещё не вызываются production-кодом — провайдер
// справки подключит их в одной из следующих задач плана, предупреждение
// снимется само собой.
#[allow(dead_code)]
pub mod corpus;
// `#[allow(dead_code)]` здесь по той же причине, что и на `container`/`corpus`
// выше: discover/InstallationCorpora/InstallationError ещё не вызываются
// production-кодом — провайдер справки подключит их в одной из следующих
// задач плана, предупреждение снимется само собой.
#[allow(dead_code)]
pub mod installation;
// `#[allow(dead_code)]` здесь по той же причине, что и на `container`/`corpus`/
// `installation` выше: PlatformSyntaxHelpProvider/rank_pages ещё не подключены
// вызывающим кодом — публичная точка `unica.documentation.search` подключит
// их в одной из следующих задач плана, предупреждение снимется само собой.
#[allow(dead_code)]
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
