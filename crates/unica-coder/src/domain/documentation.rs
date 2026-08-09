use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocumentationProviderId(String);

impl DocumentationProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for DocumentationProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    PlatformHelp,
    DevelopmentStandard,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::PlatformHelp => "platform-help",
            SourceKind::DevelopmentStandard => "development-standard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    Vendor,
    Community,
}

impl Authority {
    pub fn as_str(self) -> &'static str {
        match self {
            Authority::Vendor => "vendor",
            Authority::Community => "community",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    NotConfigured,
    VersionMissing,
    PolicyDenied,
    Timeout,
}

impl UnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnavailableReason::NotConfigured => "not-configured",
            UnavailableReason::VersionMissing => "version-missing",
            UnavailableReason::PolicyDenied => "policy-denied",
            UnavailableReason::Timeout => "timeout",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentationSectionStatus {
    Ok,
    Empty,
    Unavailable {
        reason: UnavailableReason,
        detail: String,
    },
    Failed {
        diagnostic: String,
    },
}

#[derive(Debug, Clone)]
pub struct DocumentationCorpus {
    pub id: String,
    pub source_kind: SourceKind,
    pub authority: Authority,
}

#[derive(Debug, Clone)]
pub struct DocumentationHit {
    pub rank: u32,
    pub provider_score: f32,
    pub document_id: String,
    pub title: String,
    pub signature: Option<String>,
    pub snippet: String,
    pub applicable_version: String,
}

#[derive(Debug, Clone)]
pub struct DocumentationSection {
    pub provider: DocumentationProviderId,
    pub corpus: String,
    pub source_kind: SourceKind,
    pub authority: Authority,
    /// Локаль, на которой секция ответила НА САМОМ ДЕЛЕ, а не запрошенная.
    /// Запрошенной локали у поставщика может не оказаться: справка платформы
    /// поставляется не во всех локалях сразу, и секция, ответившая соседней,
    /// обязана это назвать — иначе английский ответ на запрос `en` неотличим
    /// от русского.
    pub language: String,
    pub status: DocumentationSectionStatus,
    /// Неполнота, не отменяющая успеха: например, контейнер корпуса, который
    /// не прочитался, при том что остальные дали настоящие попадания. Статус
    /// такой секции остаётся `Ok` — найденное найдено, — но молчание о
    /// пропущенном выдавало бы частичный корпус за целый.
    pub warnings: Vec<String>,
    pub hits: Vec<DocumentationHit>,
}

impl DocumentationSection {
    pub fn empty(
        provider: DocumentationProviderId,
        corpus: &str,
        source_kind: SourceKind,
        authority: Authority,
        language: &str,
    ) -> Self {
        Self {
            provider,
            corpus: corpus.to_string(),
            source_kind,
            authority,
            language: language.to_string(),
            status: DocumentationSectionStatus::Empty,
            warnings: Vec::new(),
            hits: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentationSearchRequest {
    pub query: String,
    pub source_kinds: Vec<SourceKind>,
    pub limit: usize,
    pub language: String,
}

#[derive(Debug, Clone)]
pub struct DocumentationContext {
    pub platform_version: Option<String>,
    pub installation_root: Option<std::path::PathBuf>,
}

pub trait DocumentationProvider: Send + Sync {
    fn id(&self) -> DocumentationProviderId;
    fn corpora(&self) -> Vec<DocumentationCorpus>;
    /// Заготовка контракта под сетевых поставщиков ADR-0032, а не мёртвый
    /// код: локальный поставщик отвечает `false`, а политика сетевого выхода
    /// (`unica.toml`) будет спрашивать именно этот флаг, прежде чем пускать
    /// поставщика к сети. Убрать метод «за неиспользуемостью» значило бы
    /// заново менять публичный трейт тем же вторым заходом.
    fn needs_network(&self) -> bool;
    /// По одной секции на каждый объявленный корпус: поля секции обязаны
    /// описывать именно её содержимое, поэтому поставщик с двумя корпусами
    /// возвращает две секции.
    fn search(
        &self,
        request: &DocumentationSearchRequest,
        context: &DocumentationContext,
    ) -> Vec<DocumentationSection>;
}

pub struct DocumentationRegistry {
    providers: Vec<Arc<dyn DocumentationProvider>>,
}

impl DocumentationRegistry {
    pub fn new(providers: Vec<Arc<dyn DocumentationProvider>>) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        for provider in &providers {
            let id = provider.id();
            if !seen.insert(id.clone()) {
                return Err(format!("duplicate documentation provider id: {id}"));
            }
        }
        Ok(Self { providers })
    }

    /// Порядок реестра задаёт порядок секций публичного результата.
    pub fn providers(&self) -> impl Iterator<Item = &Arc<dyn DocumentationProvider>> {
        self.providers.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Fake {
        id: &'static str,
    }

    impl DocumentationProvider for Fake {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            vec![DocumentationCorpus {
                id: "corpus".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
            }]
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn search(
            &self,
            _: &DocumentationSearchRequest,
            _: &DocumentationContext,
        ) -> Vec<DocumentationSection> {
            vec![DocumentationSection::empty(
                self.id(),
                "corpus",
                SourceKind::PlatformHelp,
                Authority::Vendor,
                "ru",
            )]
        }
    }

    #[test]
    fn registry_preserves_declared_order() {
        // Идентификаторы намеренно НЕ в алфавитном порядке ("zulu" объявлен
        // раньше "alpha"): "first"/"second" из брифа уже отсортированы по
        // алфавиту, поэтому не отличают порядок объявления от сортировки по
        // id — реестр, сортирующий провайдеров по идентификатору, прошёл бы
        // тот вариант незамеченным. См. отчёт, мутация A.
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Fake { id: "zulu" }) as Arc<dyn DocumentationProvider>,
            Arc::new(Fake { id: "alpha" }),
        ])
        .expect("реестр собран");
        let ids: Vec<String> = registry.providers().map(|p| p.id().to_string()).collect();
        assert_eq!(ids, vec!["zulu".to_string(), "alpha".to_string()]);
    }

    #[test]
    fn duplicate_provider_ids_are_rejected() {
        // `DocumentationRegistry` хранит `Arc<dyn DocumentationProvider>`, а трейт
        // не несёт `Debug` супертрейта, поэтому `Result::expect_err` не
        // компилируется (ему нужен `Debug` у `Ok`-варианта). Тот же приём, что и
        // в `code_intelligence.rs::registry_rejects_duplicate_provider_ids`.
        let error = match DocumentationRegistry::new(vec![
            Arc::new(Fake { id: "same" }) as Arc<dyn DocumentationProvider>,
            Arc::new(Fake { id: "same" }),
        ]) {
            Ok(_) => panic!("дубликат отклонён"),
            Err(error) => error,
        };
        assert!(error.contains("same"));
    }

    // Бриф даёт здесь `hit_requires_provenance_and_version`: он строит
    // `DocumentationHit` литералом и проверяет два поля того же самого
    // литерала. Ни одна функция модуля при этом не вызывается — компилятор
    // и так не даёт собрать `DocumentationHit` без `applicable_version`
    // (это `String`, а не `Option<String>`), а непустоту строки проверяет
    // только рука автора теста. Мутационная проверка (см. отчёт) подтвердила:
    // ни порча сортировки реестра, ни снятие проверки дубликатов, ни порча
    // `DocumentationSection::empty`/`Display for DocumentationProviderId` не
    // задевают этот тест — он не вызывает ни одной из этих функций и держит
    // ту же оценку для любой реализации. Обязательность версии и локатора у
    // *настоящего* попадания — забота поставщика (`rank_pages` в Task 5),
    // где версия копируется из параметра функции, а не пишется тестом
    // напрямую; там же и лежит содержательная проверка. Заменил на тест
    // единственного реального конструктора этого файла — `DocumentationSection::empty`
    // — который действительно разносит происхождение (`provider`/`corpus`/
    // `source_kind`/`authority`) по правильным полям и не путает Empty-статус
    // с непустым списком попаданий.
    #[test]
    fn section_empty_maps_every_provenance_field_and_stays_empty() {
        let provider = DocumentationProviderId::new("acme-docs");
        // source_kind/authority намеренно НЕ те, что использует `Fake` выше
        // (PlatformHelp/Vendor), чтобы совпадение с константой по умолчанию
        // не маскировало перепутанные аргументы.
        let section = DocumentationSection::empty(
            provider.clone(),
            "developer-guide",
            SourceKind::DevelopmentStandard,
            Authority::Community,
            "de",
        );
        assert_eq!(section.provider, provider, "провайдер секции");
        assert_eq!(section.corpus, "developer-guide", "корпус секции");
        assert_eq!(
            section.language, "de",
            "локаль ответа не должна подменяться константой"
        );
        assert_eq!(
            section.source_kind,
            SourceKind::DevelopmentStandard,
            "смысл источника не должен подменяться константой"
        );
        assert_eq!(
            section.authority,
            Authority::Community,
            "авторитетность не должна подменяться константой"
        );
        assert_eq!(
            section.status,
            DocumentationSectionStatus::Empty,
            "пустая секция обязана нести статус Empty"
        );
        assert!(
            section.hits.is_empty(),
            "пустая секция не должна нести попаданий"
        );
    }

    // Не из брифа: самопроверка нашла, что `as_str()` у `SourceKind`/
    // `Authority`/`UnavailableReason` не вызывает ни один из трёх тестов
    // брифа, а Task 6 в плане сериализует ими публичный JSON дословно
    // ("sourceKind": section.source_kind.as_str(), "authority": ...as_str(),
    // "reason": reason.as_str()). Опечатка в строке (например, подчёркивание
    // вместо дефиса) молча ломает формат ответа `unica.documentation.search`
    // и осталась бы незамеченной без этого теста.
    #[test]
    fn source_kind_authority_and_reason_expose_stable_wire_identifiers() {
        assert_eq!(SourceKind::PlatformHelp.as_str(), "platform-help");
        assert_eq!(
            SourceKind::DevelopmentStandard.as_str(),
            "development-standard"
        );
        assert_eq!(Authority::Vendor.as_str(), "vendor");
        assert_eq!(Authority::Community.as_str(), "community");
        assert_eq!(UnavailableReason::NotConfigured.as_str(), "not-configured");
        assert_eq!(
            UnavailableReason::VersionMissing.as_str(),
            "version-missing"
        );
        assert_eq!(UnavailableReason::PolicyDenied.as_str(), "policy-denied");
        assert_eq!(UnavailableReason::Timeout.as_str(), "timeout");
    }
}
