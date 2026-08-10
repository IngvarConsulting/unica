//! Ретривал-гейт ADR-0035 п.6: golden-запросы задачи #415 против реальной
//! установки платформы. Пропускается, когда `UNICA_PLATFORM_HELP_DIR` не
//! задан — как и остальные проверки против материалов вендора в этом
//! каталоге; в CI не требуется.
//!
//! Гейт закрепляет две вещи разом: recall@5 — каждый запрос замера находит
//! свою страницу в топ-5 своей секции — и бюджет тёплой задержки: после
//! первого (холодного) вызова каждый запрос обязан укладываться в секунду.
//! До ADR-0035 шесть из восьми запросов замера отвечали пустой секцией.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::domain::documentation::{
    DocumentationContext, DocumentationProvider, DocumentationSearchRequest, SourceKind,
};
use crate::infrastructure::platform_help::provider::{index_test_lock, PlatformSyntaxHelpProvider};

struct GoldenCase {
    query: &'static str,
    /// Фрагмент заголовка ожидаемой страницы: устойчивее пути — заголовок
    /// несёт оба имени API и не зависит от нумерации файлов корпуса.
    expected_title_fragment: &'static str,
}

/// Замер #415 дословно: точные имена, естественные формулировки, опечатка,
/// английская пара имён.
const GOLDEN: [GoldenCase; 7] = [
    GoldenCase {
        query: "СтрНайти",
        expected_title_fragment: "СтрНайти (",
    },
    GoldenCase {
        query: "ТаблицаЗначений.Свернуть",
        expected_title_fragment: "ТаблицаЗначений.Свернуть",
    },
    GoldenCase {
        query: "свернуть таблицу значений",
        expected_title_fragment: "ТаблицаЗначений.Свернуть",
    },
    GoldenCase {
        query: "как удалить элемент массива",
        expected_title_fragment: "Массив.Удалить",
    },
    GoldenCase {
        query: "регистр сведений срез последних",
        expected_title_fragment: "СрезПоследних",
    },
    GoldenCase {
        query: "ValueTable GroupBy",
        expected_title_fragment: "ТаблицаЗначений.Свернуть",
    },
    GoldenCase {
        query: "СтрНайтти",
        expected_title_fragment: "СтрНайти (",
    },
];

const WARM_QUERY_BUDGET: Duration = Duration::from_secs(1);

fn request(query: &str) -> DocumentationSearchRequest {
    DocumentationSearchRequest {
        query: query.to_string(),
        source_kinds: vec![SourceKind::PlatformHelp],
        limit: 5,
        language: "ru".to_string(),
    }
}

#[test]
fn retrieval_gate_finds_every_golden_query_within_budget() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — ретривал-гейт пропущен");
        return;
    };
    // Слот индекса общий на процесс: гейт не должен делить его перестройку
    // с синтетическими тестами.
    let _serial = index_test_lock();
    let provider = PlatformSyntaxHelpProvider::new();
    let context = DocumentationContext {
        platform_version: None,
        installation_root: Some(PathBuf::from(root)),
    };
    let cold_started = Instant::now();
    provider.search(&request(GOLDEN[0].query), &context);
    eprintln!(
        "холодное построение индекса и первый запрос: {:?}",
        cold_started.elapsed()
    );

    let mut failures = Vec::new();
    for case in &GOLDEN {
        let started = Instant::now();
        let sections = provider.search(&request(case.query), &context);
        let elapsed = started.elapsed();
        let section = sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция Синтакс-помощника");
        let found = section
            .hits
            .iter()
            .any(|hit| hit.title.contains(case.expected_title_fragment));
        if !found {
            failures.push(format!(
                "{:?}: {:?} нет в топ-5; выдача: {:?}",
                case.query,
                case.expected_title_fragment,
                section
                    .hits
                    .iter()
                    .map(|hit| hit.title.as_str())
                    .collect::<Vec<_>>()
            ));
        }
        if elapsed > WARM_QUERY_BUDGET {
            failures.push(format!(
                "{:?}: тёплый запрос занял {elapsed:?} при бюджете {WARM_QUERY_BUDGET:?}",
                case.query
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
