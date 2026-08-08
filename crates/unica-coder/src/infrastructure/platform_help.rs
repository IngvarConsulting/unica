// `#[allow(dead_code)]` глушит два разных предупреждения dead_code:
// 1) V8Container/ContainerError/parse/entry и их внутренние помощники —
//    провайдер справки подключит их как вызывающий код в одной из следующих
//    задач плана, предупреждение снимется само собой;
// 2) tests_support::container_without_file_storage — фикстурный хелпер для
//    тестов следующей задачи; в тестах этой задачи не вызывается.
#[allow(dead_code)]
pub mod container;
