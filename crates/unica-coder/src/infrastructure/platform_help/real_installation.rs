//! Проверка против реальной установки платформы. Пропускается, когда
//! `UNICA_PLATFORM_HELP_DIR` не задан: материалы вендора в репозиторий не
//! попадают и в CI не требуются.
//!
//! Бриф задачи 5 разместил этот тест как отдельный интеграционный крейт
//! (`crates/unica-coder/tests/platform_syntax_help.rs`), обращающийся к
//! `unica_coder::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider`.
//! Это не компилируется: `lib.rs` объявляет `pub(crate) mod infrastructure;`,
//! так что внешний тестовый крейт модуль `infrastructure` вообще не видит
//! (ни через что публичное он не реэкспортирован). Расширять видимость ради
//! теста не стали — вместо этого модуль подключён здесь, внутри библиотеки,
//! обычным `#[cfg(test)]`-тестом со своим файлом: тот же изолированный файл,
//! тот же пропуск без `UNICA_PLATFORM_HELP_DIR`, но с доступом к
//! `pub(crate)`-содержимому крейта. См. отчёт задачи 5, раздел про
//! видимость.

use std::path::PathBuf;

use crate::domain::documentation::{
    DocumentationContext, DocumentationProvider, DocumentationSearchRequest,
    DocumentationSectionStatus, SourceKind,
};
use crate::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider;

#[test]
fn real_installation_answers_navigation_link_question() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — проверка пропущена");
        return;
    };
    let provider = PlatformSyntaxHelpProvider::new("ru");
    let request = DocumentationSearchRequest {
        query: "ПолучитьНавигационнуюСсылку".to_string(),
        source_kinds: vec![SourceKind::PlatformHelp],
        limit: 10,
        language: "ru".to_string(),
    };
    let context = DocumentationContext {
        platform_version: None,
        installation_root: Some(PathBuf::from(root)),
    };
    let sections = provider.search(&request, &context);
    let section = sections
        .iter()
        .find(|section| section.corpus == "syntax-context")
        .expect("секция Синтакс-помощника");
    assert!(
        matches!(section.status, DocumentationSectionStatus::Ok),
        "ожидался Ok, получено {:?}",
        section.status
    );
    let hit = section.hits.first().expect("попадание найдено");
    assert!(hit.title.contains("ПолучитьНавигационнуюСсылку"));
    assert!(!hit.applicable_version.is_empty());
}
