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
use crate::infrastructure::platform_help::provider::{index_test_lock, PlatformSyntaxHelpProvider};

#[test]
fn real_installation_answers_navigation_link_question() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — проверка пропущена");
        return;
    };
    // Слот индекса общий на процесс: настоящая установка не должна
    // вытеснять индекс синтетического теста посреди его двух вызовов.
    let _serial = index_test_lock();
    let provider = PlatformSyntaxHelpProvider::new();
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

/// Проверка выше читает только `shcntx`, и это её слепое пятно: разбор
/// контейнера был сломан для 23 из 38 контейнеров установки (первые четыре
/// байта проверялись как сигнатура, хотя это указатель на свободную
/// страницу), а `shcntx_ru.hbk` оказался среди тех пятнадцати, что проходили.
/// Корпус `platform-guides` не работал примерно на 60% своих контейнеров, и
/// ни один тест этого не видел. Здесь проверяется КАЖДЫЙ контейнер обоих
/// корпусов установки.
#[test]
fn every_container_of_a_real_installation_parses() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — проверка пропущена");
        return;
    };
    let corpora =
        crate::infrastructure::platform_help::installation::discover(&PathBuf::from(root), "ru")
            .expect("корпуса установки");
    for (name, corpus) in [
        ("syntax-context", &corpora.syntax_context),
        ("platform-guides", &corpora.platform_guides),
    ] {
        assert!(
            !corpus.containers.is_empty(),
            "корпус {name} обязан нести хотя бы один контейнер"
        );
        let mut pages = 0usize;
        for path in &corpus.containers {
            let bytes = std::fs::read(path).expect("контейнер прочитан");
            let read = crate::infrastructure::platform_help::corpus::read_corpus(&bytes)
                .unwrap_or_else(|error| {
                    panic!(
                        "контейнер {} корпуса {name} не разобрался: {error:?}",
                        path.display()
                    )
                });
            pages += read.len();
        }
        assert!(
            pages > 0,
            "корпус {name} из {} контейнеров не дал ни одной страницы",
            corpus.containers.len()
        );
        eprintln!(
            "{name}: контейнеров {}, страниц {pages}",
            corpus.containers.len()
        );
    }
}

/// Локаль, которой у установки нет, не должна ронять вызов. Установка
/// 8.3.27.2074 несёт `shcntx_ru.hbk` и английский `shcntx_root.hbk`, но не
/// `shcntx_en.hbk`; до правки `language: "en"` давал единственную секцию
/// `Unavailable { VersionMissing }`, `any_usable` оставался ложным и весь
/// `unica.documentation.search` заканчивался отказом.
#[test]
fn real_installation_answers_a_locale_it_does_not_ship() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — проверка пропущена");
        return;
    };
    // Слот индекса общий на процесс: настоящая установка не должна вытеснять
    // индекс синтетического теста посреди его двух вызовов.
    let _serial = index_test_lock();
    let provider = PlatformSyntaxHelpProvider::new();
    let request = DocumentationSearchRequest {
        query: "GetURL".to_string(),
        source_kinds: vec![SourceKind::PlatformHelp],
        limit: 10,
        language: "en".to_string(),
    };
    let context = DocumentationContext {
        platform_version: None,
        installation_root: Some(PathBuf::from(root)),
    };
    let sections = provider.search(&request, &context);
    assert_eq!(
        sections.len(),
        2,
        "ожидались секции обоих корпусов, получено {sections:?}"
    );
    let section = sections
        .iter()
        .find(|section| section.corpus == "syntax-context")
        .expect("секция Синтакс-помощника");
    assert!(
        matches!(section.status, DocumentationSectionStatus::Ok),
        "ожидался Ok, получено {:?}",
        section.status
    );
    assert!(
        !section.language.is_empty(),
        "секция обязана назвать локаль, на которой ответила"
    );
    eprintln!("на запрос en ответила локаль {}", section.language);
    assert!(section
        .hits
        .iter()
        .any(|hit| hit.title.contains("GetURL") || hit.signature.is_some()));
}
