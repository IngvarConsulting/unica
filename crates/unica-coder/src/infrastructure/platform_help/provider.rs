use std::sync::{Arc, Mutex};

use crate::domain::documentation::*;

use super::corpus::{read_corpus, CorpusPage};
use super::installation::{discover, InstallationError};

pub fn rank_pages(
    pages: &[CorpusPage],
    query: &str,
    limit: usize,
    version: &str,
    corpus: &str,
) -> Vec<DocumentationHit> {
    let needle = query.to_lowercase();
    let mut scored: Vec<(f32, &CorpusPage)> = pages
        .iter()
        .filter_map(|page| {
            let title = page.title.to_lowercase();
            let score = if title.contains(&needle) {
                1.0
            } else if page.text.to_lowercase().contains(&needle) {
                0.5
            } else {
                return None;
            };
            Some((score, page))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.1.path.cmp(&right.1.path))
    });
    scored
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(index, (score, page))| DocumentationHit {
            rank: index as u32 + 1,
            provider_score: score,
            document_id: format!("platform-syntax-help:{corpus}:{}", page.path),
            title: page.title.clone(),
            signature: page
                .signature
                .as_ref()
                .and_then(|value| value.ru.clone().or_else(|| value.en.clone())),
            snippet: page.text.chars().take(400).collect(),
            applicable_version: version.to_string(),
        })
        .collect()
}

/// Карта корпусов строится лениво один раз на процесс на версию и живёт в
/// памяти. На диск не пишется ничего.
#[derive(Default)]
struct CorpusCache {
    version: Option<String>,
    syntax_context: Vec<CorpusPage>,
    platform_guides: Vec<CorpusPage>,
}

pub struct PlatformSyntaxHelpProvider {
    language: String,
    cache: Arc<Mutex<CorpusCache>>,
}

impl PlatformSyntaxHelpProvider {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.to_string(),
            cache: Arc::new(Mutex::new(CorpusCache::default())),
        }
    }
}

impl DocumentationProvider for PlatformSyntaxHelpProvider {
    fn id(&self) -> DocumentationProviderId {
        DocumentationProviderId::new("platform-syntax-help")
    }

    fn corpora(&self) -> Vec<DocumentationCorpus> {
        vec![
            DocumentationCorpus {
                id: "syntax-context".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
            },
            DocumentationCorpus {
                id: "platform-guides".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
            },
        ]
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn search(
        &self,
        request: &DocumentationSearchRequest,
        context: &DocumentationContext,
    ) -> Vec<DocumentationSection> {
        let id = self.id();
        let Some(root) = context.installation_root.as_ref() else {
            return vec![DocumentationSection {
                provider: id,
                corpus: "syntax-context".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
                status: DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::NotConfigured,
                    detail: "установка платформы не разрешена для рабочего пространства"
                        .to_string(),
                },
                hits: Vec::new(),
            }];
        };
        let corpora = match discover(root, &self.language) {
            Ok(value) => value,
            Err(InstallationError::HelpMissingForVersion { version }) => {
                return vec![DocumentationSection {
                    provider: id,
                    corpus: "syntax-context".to_string(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                    status: DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::VersionMissing,
                        detail: format!(
                            "установка {version} не содержит Синтакс-помощника; нужна полная поставка"
                        ),
                    },
                    hits: Vec::new(),
                }]
            }
            Err(InstallationError::Unreadable { detail }) => {
                return vec![DocumentationSection {
                    provider: id,
                    corpus: "syntax-context".to_string(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                    status: DocumentationSectionStatus::Failed {
                        diagnostic: format!("каталог установки не читается: {detail}"),
                    },
                    hits: Vec::new(),
                }]
            }
            Err(InstallationError::NotFound | InstallationError::VersionUndetermined) => {
                return vec![DocumentationSection {
                    provider: id,
                    corpus: "syntax-context".to_string(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                    status: DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::NotConfigured,
                        detail: "каталог установки недоступен".to_string(),
                    },
                    hits: Vec::new(),
                }]
            }
        };
        let mut cache = self.cache.lock().expect("кеш корпусов");
        if cache.version.as_deref() != Some(corpora.version.as_str()) {
            let mut syntax = Vec::new();
            for path in &corpora.syntax_context {
                if let Ok(bytes) = std::fs::read(path) {
                    if let Ok(pages) = read_corpus(&bytes) {
                        syntax.extend(pages);
                    }
                }
            }
            let mut guides = Vec::new();
            for path in &corpora.platform_guides {
                if let Ok(bytes) = std::fs::read(path) {
                    if let Ok(pages) = read_corpus(&bytes) {
                        guides.extend(pages);
                    }
                }
            }
            cache.version = Some(corpora.version.clone());
            cache.syntax_context = syntax;
            cache.platform_guides = guides;
        }
        // По секции на корпус: поле `corpus` обязано описывать именно свои
        // попадания, поэтому корпуса не смешиваются в одну секцию.
        [
            ("syntax-context", &cache.syntax_context),
            ("platform-guides", &cache.platform_guides),
        ]
        .into_iter()
        .map(|(corpus, pages)| {
            let hits = rank_pages(
                pages,
                &request.query,
                request.limit,
                &corpora.version,
                corpus,
            );
            let status = if hits.is_empty() {
                DocumentationSectionStatus::Empty
            } else {
                DocumentationSectionStatus::Ok
            };
            DocumentationSection {
                provider: id.clone(),
                corpus: corpus.to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
                status,
                hits,
            }
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    // `super::*` уже реэкспортирует `crate::domain::documentation::*` (её
    // подключает верхний уровень модуля в Step 3), поэтому повторный прямой
    // `use crate::domain::documentation::*;` здесь избыточен и даёт
    // `unused_imports`. В брифе Step 1 и Step 3 показаны отдельными
    // фрагментами; собранные вместе, они конфликтуют по этому импорту —
    // убрал дубликат ради чистого вывода `cargo test`/`clippy -D warnings`.
    use super::*;
    use std::io::Write;

    fn request(query: &str) -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: query.to_string(),
            source_kinds: vec![SourceKind::PlatformHelp],
            limit: 20,
            language: "ru".to_string(),
        }
    }

    /// Собирает `.hbk`-контейнер: zip с HTML-страницами внутри записи
    /// `FileStorage`, как ожидает `read_corpus`. Тот же приём, что и у
    /// приватной `corpus::tests::zip_with`, но локально: та функция не видна
    /// за пределами модуля `corpus`.
    fn hbk_bytes(pages: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, html) in pages {
                writer.start_file(*name, options).expect("запись открыта");
                writer.write_all(html.as_bytes()).expect("запись записана");
            }
            writer.finish().expect("архив закрыт");
        }
        let zip_bytes = buffer.into_inner();
        crate::infrastructure::platform_help::container::tests_support::container_with(
            &[("FileStorage", zip_bytes.as_slice())],
            None,
        )
    }

    #[test]
    fn missing_installation_is_unavailable_not_failed() {
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: None,
            installation_root: None,
        };
        let sections = provider.search(&request("ПолучитьНавигационнуюСсылку"), &context);
        assert_eq!(
            sections.len(),
            1,
            "без установки — одна диагностичная секция"
        );
        let section = &sections[0];
        assert!(matches!(
            section.status,
            DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::NotConfigured,
                ..
            }
        ));
        assert!(section.hits.is_empty());
    }

    #[test]
    fn client_only_installation_reports_version_missing() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.5.1.1451");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(root.join("chartui_ru.hbk"), b"stub").expect("файл");
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: Some("8.5.1.1451".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(
            sections.len(),
            1,
            "без Синтакс-помощника — одна диагностичная секция"
        );
        let section = &sections[0];
        assert!(matches!(
            section.status,
            DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::VersionMissing,
                ..
            }
        ));
    }

    #[test]
    fn both_name_collisions_are_returned() {
        // «ЭлементыФормы» встречается в корпусе дважды — как коллекция и как
        // тип. Правило «первый побеждает» дало бы тихую потерю.
        let pages = vec![
            crate::infrastructure::platform_help::corpus::CorpusPage {
                path: "objects/a/FormItems.html".to_string(),
                title: "ЭлементыФормы (FormItems)".to_string(),
                text: "Коллекция элементов формы".to_string(),
                signature: None,
            },
            crate::infrastructure::platform_help::corpus::CorpusPage {
                path: "objects/b/Controls.html".to_string(),
                title: "ЭлементыФормы (Controls)".to_string(),
                text: "Тип элементов формы".to_string(),
                signature: None,
            },
        ];
        let hits = rank_pages(&pages, "ЭлементыФормы", 20, "8.3.27.2074", "syntax-context");
        assert_eq!(hits.len(), 2, "оба попадания сохраняются");
        assert_eq!(hits[0].rank, 1);
        assert_eq!(hits[1].rank, 2);
        assert!(hits
            .iter()
            .all(|hit| hit.applicable_version == "8.3.27.2074"));
    }

    /// Прошлое ревью отметило, что решение «секция на корпус» до сих пор
    /// ничем не проверено: поставщик объявляет два корпуса (`corpora()`
    /// возвращает два элемента), и `search` обязан вернуть по секции на
    /// каждый, а не смешивать оба в одну. Мутация, схлопывающая секции в
    /// одну или путающая, из какого корпуса попадание, обязана ронять именно
    /// этот тест.
    #[test]
    fn full_installation_returns_two_sections_scoped_to_their_own_corpus() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");

        let syntax_page =
            "<html><body><h1>Alpha из Синтакс-помощника</h1><p>текст</p></body></html>";
        let guide_page =
            "<html><body><h1>Alpha из руководства платформы</h1><p>текст</p></body></html>";

        std::fs::write(
            root.join("shcntx_ru.hbk"),
            hbk_bytes(&[("alpha/from-syntax.html", syntax_page)]),
        )
        .expect("контейнер синтакс-помощника");
        std::fs::write(
            root.join("1cv8_ru.hbk"),
            hbk_bytes(&[("alpha/from-guides.html", guide_page)]),
        )
        .expect("контейнер руководств платформы");

        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("Alpha"), &context);

        assert_eq!(sections.len(), 2, "по секции на каждый из двух корпусов");
        assert_ne!(
            sections[0].corpus, sections[1].corpus,
            "секции обязаны иметь разные corpus"
        );

        let syntax_section = sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context");
        assert_eq!(
            syntax_section.hits.len(),
            1,
            "попадание только из своего корпуса"
        );
        assert_eq!(
            syntax_section.hits[0].document_id,
            "platform-syntax-help:syntax-context:alpha/from-syntax.html",
            "попадание секции syntax-context обязано прийти из корпуса syntax-context"
        );

        let guides_section = sections
            .iter()
            .find(|section| section.corpus == "platform-guides")
            .expect("секция platform-guides");
        assert_eq!(
            guides_section.hits.len(),
            1,
            "попадание только из своего корпуса"
        );
        assert_eq!(
            guides_section.hits[0].document_id,
            "platform-syntax-help:platform-guides:alpha/from-guides.html",
            "попадание секции platform-guides обязано прийти из корпуса platform-guides"
        );
        // `both_name_collisions_are_returned` вызывает `rank_pages` напрямую и
        // проверяет только то, что она копирует переданную версию — не то,
        // что `search` передаёт именно версию ПРОЧИТАННОЙ установки
        // (`corpora.version`), а не что-то другое. Здесь версия читается из
        // реального каталога через `discover`, поэтому проверка настоящая.
        assert_eq!(syntax_section.hits[0].applicable_version, "8.3.27.2074");
        assert_eq!(guides_section.hits[0].applicable_version, "8.3.27.2074");
    }

    /// `Unreadable` — это «установка сломана» (права, не каталог), а не
    /// «установка не настроена». Секция обязана нести `Failed`, а не
    /// `Unavailable`: смешение этих двух статусов замаскировало бы поломку
    /// под обычное отсутствие настройки.
    #[test]
    fn unreadable_installation_reports_failed_status() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::write(&root, b"not a directory").expect("файл вместо каталога версии");
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(
            sections.len(),
            1,
            "неразрешённый каталог — одна диагностичная секция"
        );
        assert!(
            matches!(
                sections[0].status,
                DocumentationSectionStatus::Failed { .. }
            ),
            "Unreadable обязан давать Failed, получено {:?}",
            sections[0].status
        );
    }

    /// `installation_root` задан, но каталога по этому пути нет: `discover`
    /// вернёт `NotFound`. Это отдельный от `missing_installation_is_unavailable_not_failed`
    /// путь кода — там `installation_root` вообще `None`, и `discover` не
    /// вызывается ни разу.
    #[test]
    fn nonexistent_installation_root_reports_not_configured() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074"); // не создаём
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(sections.len(), 1);
        assert!(matches!(
            sections[0].status,
            DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::NotConfigured,
                ..
            }
        ));
    }

    /// У корня файловой системы нет последнего сегмента, значит версию из
    /// пути не вывести: `discover` вернёт `VersionUndetermined`. Тот же
    /// фикстурный приём, что и в
    /// `installation.rs::root_without_a_last_segment_reports_version_undetermined`.
    #[test]
    fn root_without_last_segment_reports_not_configured() {
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext {
            platform_version: None,
            installation_root: Some(std::path::PathBuf::from("/")),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(sections.len(), 1);
        assert!(matches!(
            sections[0].status,
            DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::NotConfigured,
                ..
            }
        ));
    }
}
