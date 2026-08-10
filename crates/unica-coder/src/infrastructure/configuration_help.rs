//! Поставщик `configuration-help`: встроенная справка конфигурации рабочего
//! пространства (ADR-0034, этап 5 #242 в конфигурационной части).
//!
//! Источник — то, что уже лежит в source-set'ах проекта: страницы
//! `<Вид>/<Имя>/Ext/Help/<локаль>.html` выгрузки Designer (ту же раскладку
//! пишет `unica.help.add`). На реальной «Управление торговлей» таких
//! объектов 2 115 — корпус, который закрывает измеренный гэп: доменные
//! вопросы конфигурации не описывает ни справка платформы, ни площадка
//! вендора (прогон корпуса 100 УТ-запросов — 0/10 настоящих ответов).
//!
//! Поставщик локален: без сети, без политики, всегда свежий — источник
//! правится тем же рабочим пространством, поэтому страницы читаются на
//! вызов, а не кешируются. Применимая версия попадания — версия самой
//! конфигурации из `Configuration.xml`.

use std::path::{Path, PathBuf};

use crate::domain::documentation::*;
use crate::infrastructure::documentation_retrieval::{RetrievalFields, RetrievalIndex};

/// Длина фрагмента выдачи — как у остальных локальных корпусов.
const SNIPPET_CHARS: usize = 400;

pub struct ConfigurationHelpProvider {
    /// Именованные source-set'ы рабочего пространства: имя и корень.
    pub source_sets: Vec<(String, PathBuf)>,
}

/// Страница справки одного объекта конфигурации.
struct HelpPage {
    source_set: String,
    /// Путь внутри набора исходников: `Catalogs/Номенклатура/Ext/Help/ru.html`.
    relative: String,
    locale: String,
    title: String,
    text: String,
}

fn strip_markup(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() / 2);
    let mut inside = false;
    for ch in raw.chars() {
        match ch {
            '<' => inside = true,
            '>' => {
                inside = false;
                out.push(' ');
            }
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn page_title(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let open = lower.find("<h1")?;
    let content_start = open + lower[open..].find('>')? + 1;
    let close = lower[content_start..].find("</h1")? + content_start;
    let inner = strip_markup(&raw[content_start..close]);
    (!inner.is_empty()).then_some(inner)
}

/// Версия конфигурации из `Configuration.xml` source-set'а: применимая
/// версия страниц справки — версия самой конфигурации, а не платформы.
fn configuration_version(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Configuration.xml")).ok()?;
    let open = text.find("<Version>")?;
    let close = text[open..].find("</Version>")? + open;
    let value = text[open + "<Version>".len()..close].trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Имя объекта из сегментов пути до `Ext`: `Documents.ЗаказКлиента`,
/// `Documents.ЗаказКлиента.Forms.ФормаДокумента`,
/// `Subsystems.Продажи.Subsystems.Скидки`; страницы самой конфигурации
/// (корневой `Ext/Help`) называются `Конфигурация`.
fn object_of(relative: &str) -> String {
    let segments: Vec<&str> = relative
        .split('/')
        .take_while(|segment| *segment != "Ext")
        .collect();
    if segments.is_empty() {
        "Конфигурация".to_string()
    } else {
        segments.join(".")
    }
}

fn collect_pages(source_sets: &[(String, PathBuf)]) -> (Vec<HelpPage>, Vec<String>) {
    let mut pages = Vec::new();
    let mut warnings = Vec::new();
    for (set_name, root) in source_sets {
        // Справка живёт только в узаконенных выгрузкой местах: `Ext/Help`
        // корня, объекта метаданных, формы и подсистемы (включая вложенные).
        // Обход не спускается в остальное дерево — на реальной выгрузке это
        // десятки тысяч каталогов модулей, макетов и форм без справки.
        read_help_directory(
            set_name,
            root,
            &root.join("Ext/Help"),
            &mut pages,
            &mut warnings,
        );
        for kind in subdirectories(root) {
            if kind.file_name().and_then(|name| name.to_str()) == Some("Ext") {
                continue;
            }
            for object in subdirectories(&kind) {
                collect_object(set_name, root, &object, &mut pages, &mut warnings);
            }
        }
    }
    (pages, warnings)
}

fn subdirectories(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

fn collect_object(
    set_name: &str,
    root: &Path,
    object: &Path,
    pages: &mut Vec<HelpPage>,
    warnings: &mut Vec<String>,
) {
    read_help_directory(set_name, root, &object.join("Ext/Help"), pages, warnings);
    for form in subdirectories(&object.join("Forms")) {
        read_help_directory(set_name, root, &form.join("Ext/Help"), pages, warnings);
    }
    for nested in subdirectories(&object.join("Subsystems")) {
        collect_object(set_name, root, &nested, pages, warnings);
    }
}

fn read_help_directory(
    set_name: &str,
    root: &Path,
    help: &Path,
    pages: &mut Vec<HelpPage>,
    warnings: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(help) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.ends_with(".html") || path.is_dir() {
            continue;
        }
        let relative = match path.strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let locale = name.trim_end_matches(".html").to_string();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                warnings.push(format!("{relative}: {error}"));
                continue;
            }
        };
        let text = strip_markup(&raw);
        let object = object_of(&relative);
        let title = match page_title(&raw) {
            Some(inner) => format!("{object} — {inner}"),
            None => object.clone(),
        };
        pages.push(HelpPage {
            source_set: set_name.to_string(),
            relative,
            locale,
            title,
            text,
        });
    }
}

/// Локаль корпуса — одна на ответ, как у справки установки: запрошенная,
/// если хоть одна страница её несёт, иначе `ru`, иначе первая по алфавиту.
/// Секция называет ответившую локаль, а страницы других локалей не
/// смешиваются с ней в одной выдаче.
fn resolve_locale(pages: &[HelpPage], language: &str) -> Option<String> {
    let available: std::collections::BTreeSet<&str> =
        pages.iter().map(|page| page.locale.as_str()).collect();
    if available.contains(language) {
        return Some(language.to_string());
    }
    if available.contains("ru") {
        return Some("ru".to_string());
    }
    available.iter().next().map(|locale| locale.to_string())
}

impl DocumentationProvider for ConfigurationHelpProvider {
    fn id(&self) -> DocumentationProviderId {
        DocumentationProviderId::new("configuration-help")
    }

    fn corpora(&self) -> Vec<DocumentationCorpus> {
        vec![DocumentationCorpus {
            id: "configuration-help".to_string(),
            source_kind: SourceKind::ConfigurationDocumentation,
            authority: Authority::Vendor,
        }]
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn search(
        &self,
        request: &DocumentationSearchRequest,
        _context: &DocumentationContext,
    ) -> Vec<DocumentationSection> {
        let (pages, warnings) = collect_pages(&self.source_sets);
        let Some(locale) = resolve_locale(&pages, &request.language) else {
            // Справки в конфигурации нет вовсе — честный Empty, а не отказ:
            // источник настроен, в нём просто нет страниц.
            let mut section = DocumentationSection::empty(
                self.id(),
                "configuration-help",
                SourceKind::ConfigurationDocumentation,
                Authority::Vendor,
                &request.language,
            );
            section.warnings = warnings;
            return vec![section];
        };
        let versions: std::collections::BTreeMap<&str, Option<String>> = self
            .source_sets
            .iter()
            .map(|(name, root)| (name.as_str(), configuration_version(root)))
            .collect();
        // Матчинг — лексическим ядром (ADR-0036). Корпус — страницы справки
        // одной конфигурации, единицы и десятки, поэтому индекс строится на
        // вызов: кешировать его значило бы завести ключ по файлам выгрузки
        // ради экономии, которую здесь не с чего собирать.
        let locale_pages: Vec<&HelpPage> = pages
            .iter()
            .filter(|page| page.locale == locale)
            .collect();
        let retrieval = RetrievalIndex::build(locale_pages.iter().map(|page| RetrievalFields {
            title: &page.title,
            signature: "",
            body: &page.text,
        }));
        let hits: Vec<DocumentationHit> = retrieval
            .query(&request.query, request.limit, &[])
            .into_iter()
            .enumerate()
            .map(|(index, scored)| {
                let page = locale_pages[scored.document];
                DocumentationHit {
                    rank: index as u32 + 1,
                    provider_score: scored.score,
                    document_id: format!(
                        "configuration-help:{}:{}",
                        page.source_set, page.relative
                    ),
                    title: page.title.clone(),
                    signature: None,
                    snippet: page.text.chars().take(SNIPPET_CHARS).collect(),
                    // Применимая версия — версия КОНФИГУРАЦИИ: страницы
                    // описывают её объекты, а не платформу.
                    applicable_version: versions
                        .get(page.source_set.as_str())
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| "unversioned".to_string()),
                }
            })
            .collect();
        let status = if hits.is_empty() {
            DocumentationSectionStatus::Empty
        } else {
            DocumentationSectionStatus::Ok
        };
        vec![DocumentationSection {
            provider: self.id(),
            corpus: "configuration-help".to_string(),
            source_kind: SourceKind::ConfigurationDocumentation,
            authority: Authority::Vendor,
            language: locale,
            status,
            warnings,
            hits,
        }]
    }

    fn get(
        &self,
        document_id: &str,
        _language: &str,
        _context: &DocumentationContext,
    ) -> Option<Result<DocumentationDocument, String>> {
        let rest = document_id.strip_prefix("configuration-help:")?;
        let Some((set_name, relative)) = rest.split_once(':') else {
            return Some(Err(format!(
                "локатор {document_id:?} не несёт source-set и пути"
            )));
        };
        // Путь уходит в файловую систему: сегменты `..` вывели бы чтение за
        // корень source-set — отказ до обращения к диску.
        if relative
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Some(Err(format!(
                "локатор {document_id:?} несёт сегменты '..' или пустые"
            )));
        }
        let Some((_, root)) = self.source_sets.iter().find(|(name, _)| name == set_name) else {
            return Some(Err(format!(
                "source-set {set_name:?} не объявлен рабочим пространством"
            )));
        };
        let path = root.join(relative);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                return Some(Err(format!("страница {relative:?} не читается: {error}")));
            }
        };
        let text = strip_markup(&raw);
        let object = object_of(relative);
        let title = match page_title(&raw) {
            Some(inner) => format!("{object} — {inner}"),
            None => object,
        };
        let locale = relative
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(".html"))
            .unwrap_or("ru")
            .to_string();
        Some(Ok(DocumentationDocument {
            provider: self.id(),
            corpus: "configuration-help".to_string(),
            source_kind: SourceKind::ConfigurationDocumentation,
            authority: Authority::Vendor,
            language: locale,
            document_id: document_id.to_string(),
            title,
            signature: None,
            applicable_version: configuration_version(root)
                .unwrap_or_else(|| "unversioned".to_string()),
            text,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> DocumentationContext {
        DocumentationContext {
            platform_version: None,
            installation_root: None,
        }
    }

    fn request(query: &str, language: &str) -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: query.to_string(),
            source_kinds: Vec::new(),
            limit: 10,
            language: language.to_string(),
        }
    }

    /// Выгрузка с двумя объектами: справочник с русской страницей и документ
    /// с русской и английской; версия конфигурации объявлена в корне.
    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("workspace");
        std::fs::write(
            root.path().join("Configuration.xml"),
            "<?xml version=\"1.0\"?><MetaDataObject><Configuration><Properties>\
             <Version>11.5.7.100</Version></Properties></Configuration></MetaDataObject>",
        )
        .expect("configuration");
        let catalog = root.path().join("Catalogs/Номенклатура/Ext/Help");
        std::fs::create_dir_all(&catalog).expect("catalog help");
        std::fs::write(
            catalog.join("ru.html"),
            "<html><body><h1>Карточка номенклатуры</h1>\
             <p>Ведение справочника номенклатуры и единиц измерения.</p></body></html>",
        )
        .expect("catalog page");
        let document = root.path().join("Documents/ЗаказКлиента/Ext/Help");
        std::fs::create_dir_all(&document).expect("document help");
        std::fs::write(
            document.join("ru.html"),
            "<html><body><h1>Заказ клиента</h1><p>Оформление заказа клиента.</p></body></html>",
        )
        .expect("document page ru");
        std::fs::write(
            document.join("en.html"),
            "<html><body><h1>Customer order</h1><p>Registering a customer order.</p></body></html>",
        )
        .expect("document page en");
        root
    }

    fn provider(root: &tempfile::TempDir) -> ConfigurationHelpProvider {
        ConfigurationHelpProvider {
            source_sets: vec![("main".to_string(), root.path().to_path_buf())],
        }
    }

    #[test]
    fn search_finds_a_page_by_object_name() {
        let root = workspace();
        let sections = provider(&root).search(&request("Номенклатура", "ru"), &context());
        assert_eq!(sections.len(), 1, "ровно одна секция корпуса: {sections:?}");
        let section = &sections[0];
        assert_eq!(section.provider.to_string(), "configuration-help");
        assert_eq!(section.corpus, "configuration-help");
        assert_eq!(section.source_kind, SourceKind::ConfigurationDocumentation);
        assert_eq!(section.status, DocumentationSectionStatus::Ok);
        assert_eq!(section.language, "ru");
        let hit = &section.hits[0];
        assert_eq!(
            hit.document_id,
            "configuration-help:main:Catalogs/Номенклатура/Ext/Help/ru.html"
        );
        assert!(
            hit.title.contains("Catalogs.Номенклатура") && hit.title.contains("Карточка"),
            "заголовок несёт объект и h1: {}",
            hit.title
        );
        assert_eq!(hit.applicable_version, "11.5.7.100");
        assert!(hit.snippet.contains("единиц измерения"), "{}", hit.snippet);
    }

    /// Терм в заголовке страницы весит больше терма в тексте (ADR-0036):
    /// «заказ» стоит в заголовке страницы документа и лишь в тексте страницы
    /// справочника — страница документа обязана быть первой.
    #[test]
    fn a_text_match_ranks_below_a_title_match() {
        let root = workspace();
        let catalog = root.path().join("Catalogs/Номенклатура/Ext/Help");
        std::fs::write(
            catalog.join("ru.html"),
            "<html><body><h1>Карточка номенклатуры</h1>\
             <p>Позиции добавляются в заказ клиента из карточки.</p></body></html>",
        )
        .expect("catalog page rewritten");
        let sections = provider(&root).search(&request("заказ клиента", "ru"), &context());
        let hits = &sections[0].hits;
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(
            hits[0].document_id.contains("ЗаказКлиента"),
            "заголовок обязан перевесить текст: {hits:?}"
        );
        assert!(hits[0].provider_score > hits[1].provider_score, "{hits:?}");
    }

    /// Ядро ADR-0036: естественная формулировка с падежами находит страницу,
    /// хотя точной подстроки запроса в ней нет (#415).
    #[test]
    fn natural_phrasing_with_morphology_finds_the_page() {
        let root = workspace();
        let sections = provider(&root).search(
            &request("единицы измерения номенклатуры", "ru"),
            &context(),
        );
        let section = &sections[0];
        assert_eq!(section.status, DocumentationSectionStatus::Ok, "{section:?}");
        assert!(
            section.hits[0].document_id.contains("Номенклатура"),
            "{:?}",
            section.hits
        );
    }

    #[test]
    fn the_requested_locale_wins_and_excludes_other_locales() {
        let root = workspace();
        let sections = provider(&root).search(&request("order", "en"), &context());
        let section = &sections[0];
        assert_eq!(section.language, "en");
        assert_eq!(section.hits.len(), 1, "{:?}", section.hits);
        assert!(section.hits[0].document_id.ends_with("/en.html"));
    }

    #[test]
    fn an_unknown_locale_falls_back_to_ru() {
        let root = workspace();
        let sections = provider(&root).search(&request("Номенклатура", "de"), &context());
        assert_eq!(sections[0].language, "ru");
        assert_eq!(sections[0].hits.len(), 1);
    }

    #[test]
    fn a_workspace_without_help_answers_with_an_empty_section() {
        let root = tempfile::tempdir().expect("workspace");
        let provider = ConfigurationHelpProvider {
            source_sets: vec![("main".to_string(), root.path().to_path_buf())],
        };
        let sections = provider.search(&request("Номенклатура", "ru"), &context());
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].status, DocumentationSectionStatus::Empty);
        assert!(sections[0].hits.is_empty());
    }

    #[test]
    fn a_malformed_page_becomes_a_warning() {
        let root = workspace();
        let broken = root.path().join("Catalogs/Валюты/Ext/Help");
        std::fs::create_dir_all(&broken).expect("broken help");
        std::fs::write(broken.join("ru.html"), [0xFF, 0xFE, 0x00]).expect("broken page");
        let sections = provider(&root).search(&request("Номенклатура", "ru"), &context());
        let section = &sections[0];
        assert_eq!(section.status, DocumentationSectionStatus::Ok);
        assert!(
            section
                .warnings
                .iter()
                .any(|warning| warning.contains("Catalogs/Валюты/Ext/Help/ru.html")),
            "нечитаемая страница названа предупреждением: {:?}",
            section.warnings
        );
    }

    #[test]
    fn a_form_page_is_labeled_with_its_form() {
        let root = workspace();
        let form = root
            .path()
            .join("Documents/ЗаказКлиента/Forms/ФормаДокумента/Ext/Help");
        std::fs::create_dir_all(&form).expect("form help");
        std::fs::write(
            form.join("ru.html"),
            "<html><body><h1>Работа с формой заказа</h1><p>Подбор товаров в заказ.</p></body></html>",
        )
        .expect("form page");
        let sections = provider(&root).search(&request("Работа с формой заказа", "ru"), &context());
        let hit = &sections[0].hits[0];
        assert_eq!(
            hit.document_id,
            "configuration-help:main:Documents/ЗаказКлиента/Forms/ФормаДокумента/Ext/Help/ru.html"
        );
        assert!(
            hit.title
                .starts_with("Documents.ЗаказКлиента.Forms.ФормаДокумента"),
            "страница формы названа формой, а не конфигурацией: {}",
            hit.title
        );
    }

    #[test]
    fn help_is_found_at_every_legal_layout_location() {
        let root = workspace();
        let cases = [
            ("Ext/Help", "Конфигурация"),
            (
                "Subsystems/Продажи/Subsystems/Скидки/Ext/Help",
                "Subsystems.Продажи.Subsystems.Скидки",
            ),
        ];
        for (location, object) in &cases {
            let dir = root.path().join(location);
            std::fs::create_dir_all(&dir).expect("layout dir");
            std::fs::write(
                dir.join("ru.html"),
                format!(
                    "<html><body><h1>Страница {object}</h1><p>Текст {object}.</p></body></html>"
                ),
            )
            .expect("layout page");
        }
        for (_, object) in &cases {
            let sections =
                provider(&root).search(&request(&format!("Страница {object}"), "ru"), &context());
            // Пословный матчинг вправе поднять и соседние страницы с общим
            // словом «Страница» — важна не единственность, а то, что своя
            // страница найдена и стоит первой (ADR-0036).
            assert!(
                !sections[0].hits.is_empty(),
                "страница объекта {object} обязана найтись"
            );
            assert!(
                sections[0].hits[0].title.starts_with(object),
                "своя страница обязана быть первой: {:?}",
                sections[0].hits
            );
        }
    }

    #[test]
    fn a_page_outside_the_help_layout_is_not_a_corpus_page() {
        let root = workspace();
        let decoy = root
            .path()
            .join("Catalogs/Номенклатура/Templates/Макет/Ext/Help");
        std::fs::create_dir_all(&decoy).expect("decoy dir");
        std::fs::write(
            decoy.join("ru.html"),
            "<html><body><h1>Служебные данные макета</h1><p>Не справка объекта.</p></body></html>",
        )
        .expect("decoy page");
        let sections =
            provider(&root).search(&request("Служебные данные макета", "ru"), &context());
        assert!(
            sections[0].hits.is_empty(),
            "справка живёт у корня, объекта, формы и подсистемы — не в макетах: {:?}",
            sections[0].hits
        );
    }

    #[test]
    fn get_returns_the_full_document_by_locator() {
        let root = workspace();
        let document = provider(&root)
            .get(
                "configuration-help:main:Catalogs/Номенклатура/Ext/Help/ru.html",
                "ru",
                &context(),
            )
            .expect("локатор наш")
            .expect("страница читается");
        assert_eq!(document.provider.to_string(), "configuration-help");
        assert_eq!(document.corpus, "configuration-help");
        assert_eq!(document.source_kind, SourceKind::ConfigurationDocumentation);
        assert_eq!(document.language, "ru");
        assert_eq!(document.applicable_version, "11.5.7.100");
        assert!(
            document.text.contains("единиц измерения"),
            "{}",
            document.text
        );
        assert!(
            document.title.contains("Catalogs.Номенклатура"),
            "{}",
            document.title
        );
    }

    #[test]
    fn a_foreign_locator_is_not_mine() {
        let root = workspace();
        assert!(provider(&root)
            .get(
                "platform-syntax-help:shcntx_ru:objects/x.html",
                "ru",
                &context()
            )
            .is_none());
    }

    #[test]
    fn a_locator_with_traversal_segments_is_refused() {
        let root = workspace();
        let refusal = provider(&root)
            .get(
                "configuration-help:main:../secret/ru.html",
                "ru",
                &context(),
            )
            .expect("локатор наш");
        let error = refusal.expect_err("сегменты '..' — отказ");
        assert!(error.contains(".."), "{error}");
    }

    #[test]
    fn an_undeclared_source_set_is_refused() {
        let root = workspace();
        let refusal = provider(&root)
            .get(
                "configuration-help:other:Catalogs/Номенклатура/Ext/Help/ru.html",
                "ru",
                &context(),
            )
            .expect("локатор наш");
        let error = refusal.expect_err("чужой source-set — отказ");
        assert!(error.contains("other"), "{error}");
    }

    #[test]
    fn a_missing_page_is_refused_with_its_name() {
        let root = workspace();
        let refusal = provider(&root)
            .get(
                "configuration-help:main:Catalogs/Пропавший/Ext/Help/ru.html",
                "ru",
                &context(),
            )
            .expect("локатор наш");
        let error = refusal.expect_err("пропавшая страница — отказ");
        assert!(error.contains("Catalogs/Пропавший"), "{error}");
    }
}
