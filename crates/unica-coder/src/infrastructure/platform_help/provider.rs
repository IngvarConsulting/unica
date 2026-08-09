use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::domain::documentation::*;

use super::corpus::{read_corpus, CorpusPage, Signature};
use super::installation::{discover, InstallationCorpora, InstallationError};

/// Длина фрагмента выдачи. Вырезается один раз при индексировании: фрагмент
/// не зависит от запроса, и держать ради него полный текст страницы незачем.
const SNIPPET_CHARS: usize = 400;

/// Единственное объявление корпусов поставщика. И `corpora()`, и `search()`
/// читают эту таблицу, поэтому разойтись молча они не могут, а имена корпусов
/// не повторяются литералом в каждой ветке.
struct CorpusSpec {
    id: &'static str,
    source_kind: SourceKind,
    authority: Authority,
}

const CORPUS_SPECS: [CorpusSpec; 2] = [
    CorpusSpec {
        id: "syntax-context",
        source_kind: SourceKind::PlatformHelp,
        authority: Authority::Vendor,
    },
    CorpusSpec {
        id: "platform-guides",
        source_kind: SourceKind::PlatformHelp,
        authority: Authority::Vendor,
    },
];

/// Страница в индексе: только то, что нужно `rank_pages`. Текст лежит уже
/// приведённым к нижнему регистру, а фрагмент выдачи — уже вырезанным,
/// поэтому исходный текст страницы не удерживается вовсе и не копируется в
/// нижний регистр на каждый запрос (около 43 МБ мусора на вызов до правки).
struct IndexedPage {
    path: String,
    title: String,
    title_lower: String,
    text_lower: String,
    snippet: String,
    signature: Option<Signature>,
}

fn index_page(page: CorpusPage) -> IndexedPage {
    IndexedPage {
        title_lower: page.title.to_lowercase(),
        text_lower: page.text.to_lowercase(),
        snippet: page.text.chars().take(SNIPPET_CHARS).collect(),
        path: page.path,
        title: page.title,
        signature: page.signature,
    }
}

struct IndexedCorpus {
    pages: Vec<IndexedPage>,
    /// Контейнеры, которые не прочитались или не разобрались. Молчаливый
    /// пропуск превращал повреждённый `.hbk` в «ничего не нашлось».
    unreadable: Vec<String>,
}

struct InstallationIndex {
    version: String,
    /// Параллелен `CORPUS_SPECS` по порядку и длине: длину держит компилятор
    /// (см. `build_index`), порядок задан там же одним местом.
    corpora: Vec<IndexedCorpus>,
}

/// Ключ индекса — каталог установки и язык, а не одна лишь версия: под
/// разными корнями встречаются одноимённые каталоги версий, и справка у них
/// своя. Язык входит в ключ, потому что им отбираются сами контейнеры.
#[derive(PartialEq, Eq)]
struct IndexKey {
    root: PathBuf,
    language: String,
}

/// Разобранный корпус живёт один на процесс, а не один на вызов: реестр
/// поставщиков собирается заново в каждой ветке диспетчера, поэтому кеш
/// внутри экземпляра поставщика не переживал ни одного вызова — каждый
/// `unica.documentation.search` заново читал и разбирал всю установку.
///
/// Слот ровно один и перестраивается при смене ключа. Держать по индексу на
/// каждую версию, за которой сходил вызывающий, значит закрепить память по
/// чужому вводу; один слот ограничивает её сверху и повторяет прежнюю
/// семантику «сменилась версия — перестроились». На диск по-прежнему не
/// пишется ничего (п.8 ADR-0029).
struct CachedIndex {
    key: IndexKey,
    index: Arc<InstallationIndex>,
}

static INSTALLATION_INDEX: OnceLock<Mutex<Option<CachedIndex>>> = OnceLock::new();

fn index_slot() -> &'static Mutex<Option<CachedIndex>> {
    INSTALLATION_INDEX.get_or_init(|| Mutex::new(None))
}

/// Слот один на процесс, поэтому тест, которому важно состояние слота МЕЖДУ
/// вызовами, обязан идти в одиночку: соседний тест с другой установкой
/// вытеснит индекс. Тот же приём, что и `test_support::process_cwd_lock`.
#[cfg(test)]
pub(crate) fn index_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn index_corpus(containers: &[PathBuf]) -> IndexedCorpus {
    let mut pages = Vec::new();
    let mut unreadable = Vec::new();
    for path in containers {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("<без имени>")
            .to_string();
        match std::fs::read(path) {
            Err(error) => unreadable.push(format!("{name}: {error}")),
            Ok(bytes) => match read_corpus(&bytes) {
                Err(error) => unreadable.push(format!("{name}: {error:?}")),
                Ok(read) => pages.extend(read.into_iter().map(index_page)),
            },
        }
    }
    IndexedCorpus { pages, unreadable }
}

fn build_index(corpora: &InstallationCorpora) -> InstallationIndex {
    // Соответствие с `CORPUS_SPECS` позиционное: компилятор держит длину
    // массива, а порядок объявлен здесь. Добавленный корпус не соберётся,
    // пока его контейнеры не названы.
    let sources: [&[PathBuf]; CORPUS_SPECS.len()] =
        [&corpora.syntax_context, &corpora.platform_guides];
    InstallationIndex {
        version: corpora.version.clone(),
        corpora: sources.into_iter().map(index_corpus).collect(),
    }
}

/// Индекс установки под ключом. Перестройка идёт под замком: одновременные
/// поиски (`MCP_MAX_TOOL_WORKERS` = 32) должны ждать одну перестройку, а не
/// строить по своей.
///
/// Восстановление после отравления — как в `workspace_services`: паника в
/// разборе одного контейнера не должна навсегда ронять каждый следующий
/// вызов. Слот перезаписывается целиком, поэтому отравленный страж видит
/// либо прежнее целое состояние, либо чистую перестройку.
fn indexed(key: IndexKey, corpora: &InstallationCorpora) -> Arc<InstallationIndex> {
    let mut slot = index_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = slot.as_ref() {
        if cached.key == key {
            return Arc::clone(&cached.index);
        }
    }
    let index = Arc::new(build_index(corpora));
    *slot = Some(CachedIndex {
        key,
        index: Arc::clone(&index),
    });
    index
}

/// Сигнатура на языке запроса. Хранятся обе локали, поэтому выбор — вопрос
/// предпочтения, а не наличия: запрошенная идёт первой, вторая остаётся
/// запасной, чтобы вопрос про член платформы не остался вовсе без сигнатуры.
fn signature_in(signature: &Signature, language: &str) -> Option<String> {
    let (preferred, fallback) = match language {
        "en" => (&signature.en, &signature.ru),
        _ => (&signature.ru, &signature.en),
    };
    preferred.clone().or_else(|| fallback.clone())
}

fn rank_pages(
    pages: &[IndexedPage],
    query: &str,
    limit: usize,
    version: &str,
    corpus: &str,
    language: &str,
) -> Vec<DocumentationHit> {
    let needle = query.to_lowercase();
    let mut scored: Vec<(f32, &IndexedPage)> = pages
        .iter()
        .filter_map(|page| {
            let score = if page.title_lower.contains(&needle) {
                1.0
            } else if page.text_lower.contains(&needle) {
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
                .and_then(|value| signature_in(value, language)),
            snippet: page.snippet.clone(),
            applicable_version: version.to_string(),
        })
        .collect()
}

#[derive(Default)]
pub struct PlatformSyntaxHelpProvider;

impl PlatformSyntaxHelpProvider {
    pub fn new() -> Self {
        Self
    }

    /// Источник недоступен целиком — одна диагностичная секция (п.5
    /// ADR-0029). Её происхождение берётся у первого объявленного корпуса:
    /// секция описывает поставщика, а не какой-то один его корпус, и четыре
    /// почти одинаковых литерала повторять для этого незачем.
    fn diagnostic(&self, status: DocumentationSectionStatus) -> Vec<DocumentationSection> {
        let spec = &CORPUS_SPECS[0];
        vec![DocumentationSection {
            provider: self.id(),
            corpus: spec.id.to_string(),
            source_kind: spec.source_kind,
            authority: spec.authority,
            status,
            hits: Vec::new(),
        }]
    }
}

impl DocumentationProvider for PlatformSyntaxHelpProvider {
    fn id(&self) -> DocumentationProviderId {
        DocumentationProviderId::new("platform-syntax-help")
    }

    fn corpora(&self) -> Vec<DocumentationCorpus> {
        CORPUS_SPECS
            .iter()
            .map(|spec| DocumentationCorpus {
                id: spec.id.to_string(),
                source_kind: spec.source_kind,
                authority: spec.authority,
            })
            .collect()
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn search(
        &self,
        request: &DocumentationSearchRequest,
        context: &DocumentationContext,
    ) -> Vec<DocumentationSection> {
        let Some(root) = context.installation_root.as_ref() else {
            // Ограничение, по которому установку искали, названо в отказе:
            // «не разрешена» без версии не отличает «платформа не
            // установлена» от «проект закреплён за версией, которой нет».
            let detail = match context.platform_version.as_deref() {
                Some(version) => {
                    format!("установка платформы {version} не разрешена для рабочего пространства")
                }
                None => "установка платформы не разрешена для рабочего пространства".to_string(),
            };
            return self.diagnostic(DocumentationSectionStatus::Unavailable {
                reason: UnavailableReason::NotConfigured,
                detail,
            });
        };
        let corpora = match discover(root, &request.language) {
            Ok(value) => value,
            Err(InstallationError::HelpMissingForVersion { version }) => {
                // Язык назван: у нерусской установки Синтакс-помощник лежит в
                // `shcntx_<язык>.hbk`, и «нужна полная поставка» без языка
                // отправляло бы читателя чинить не то.
                return self.diagnostic(DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::VersionMissing,
                    detail: format!(
                        "установка {version} не содержит Синтакс-помощника на языке {}; нужна полная поставка",
                        request.language
                    ),
                });
            }
            Err(InstallationError::Unreadable { detail }) => {
                return self.diagnostic(DocumentationSectionStatus::Failed {
                    diagnostic: format!("каталог установки не читается: {detail}"),
                });
            }
            // NotFound и VersionUndetermined и раньше давали один и тот же
            // `reason`, но обязаны различаться текстом: иначе вызывающий не
            // отличит «каталога нет» от «версию не вывести из пути» ни по
            // чему, кроме кода, а не по ответу.
            Err(InstallationError::VersionUndetermined) => {
                return self.diagnostic(DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::NotConfigured,
                    detail: format!("версия не выводится из корня установки: {}", root.display()),
                });
            }
            Err(InstallationError::NotFound) => {
                return self.diagnostic(DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::NotConfigured,
                    detail: format!("каталог установки недоступен: {}", root.display()),
                });
            }
        };
        let index = indexed(
            IndexKey {
                root: root.clone(),
                language: request.language.clone(),
            },
            &corpora,
        );
        // По секции на корпус: поле `corpus` обязано описывать именно свои
        // попадания, поэтому корпуса не смешиваются в одну секцию.
        CORPUS_SPECS
            .iter()
            .zip(index.corpora.iter())
            .map(|(spec, corpus)| {
                let hits = rank_pages(
                    &corpus.pages,
                    &request.query,
                    request.limit,
                    &index.version,
                    spec.id,
                    &request.language,
                );
                // `Empty` означает «корпус прочитан, ничего не совпало». Если
                // хоть один контейнер не разобрался, это неправда: совпадение
                // могло лежать именно в нём. Найденные попадания при этом
                // настоящие, поэтому непустая выдача остаётся `Ok`.
                let status = if hits.is_empty() && !corpus.unreadable.is_empty() {
                    DocumentationSectionStatus::Failed {
                        diagnostic: format!(
                            "контейнеры корпуса не разобрались: {}",
                            corpus.unreadable.join("; ")
                        ),
                    }
                } else if hits.is_empty() {
                    DocumentationSectionStatus::Empty
                } else {
                    DocumentationSectionStatus::Ok
                };
                DocumentationSection {
                    provider: self.id(),
                    corpus: spec.id.to_string(),
                    source_kind: spec.source_kind,
                    authority: spec.authority,
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
        let provider = PlatformSyntaxHelpProvider::new();
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

    /// Разрешение установки может не найти версию, за которой закреплён
    /// проект, — и тогда отказ обязан назвать её. Общий текст «установка
    /// платформы не разрешена» не отличает «платформы на машине нет» от
    /// «есть, но не та», а именно это различие и оплачивает п.3 ADR-0029.
    #[test]
    fn refusal_names_the_version_the_installation_was_resolved_for() {
        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27".to_string()),
            installation_root: None,
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        match &sections[0].status {
            DocumentationSectionStatus::Unavailable { detail, .. } => assert!(
                detail.contains("8.3.27"),
                "отказ обязан назвать искомую версию, получено {detail}"
            ),
            other => panic!("ожидался Unavailable, получено {other:?}"),
        }
    }

    #[test]
    fn client_only_installation_reports_version_missing() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.5.1.1451");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(root.join("chartui_ru.hbk"), b"stub").expect("файл");
        let provider = PlatformSyntaxHelpProvider::new();
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
        // тип. Правило «первый побеждает» дало бы тихую потерю. Страницы
        // проходят через `index_page`, а не собираются полем-в-поле: иначе
        // тест мог бы задать `title_lower`, не совпадающий с `title`.
        let pages: Vec<IndexedPage> = [
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
        ]
        .into_iter()
        .map(index_page)
        .collect();
        let hits = rank_pages(
            &pages,
            "ЭлементыФормы",
            20,
            "8.3.27.2074",
            "syntax-context",
            "ru",
        );
        assert_eq!(hits.len(), 2, "оба попадания сохраняются");
        assert_eq!(hits[0].rank, 1);
        assert_eq!(hits[1].rank, 2);
        assert!(hits
            .iter()
            .all(|hit| hit.applicable_version == "8.3.27.2074"));
    }

    /// Аргумент `language` доходил до `DocumentationSearchRequest` и не
    /// читался никем: локаль сигнатуры была зашита предпочтением русской.
    /// Здесь одна и та же страница спрашивается дважды, и от языка запроса
    /// обязана меняться выданная сигнатура.
    #[test]
    fn requested_language_picks_the_signature_locale() {
        let pages: Vec<IndexedPage> = [crate::infrastructure::platform_help::corpus::CorpusPage {
            path: "objects/GetURL.html".to_string(),
            title: "ПолучитьНавигационнуюСсылку (GetURL)".to_string(),
            text: "текст".to_string(),
            signature: Some(crate::infrastructure::platform_help::corpus::Signature {
                ru: Some("ПолучитьНавигационнуюСсылку(<Объект>)".to_string()),
                en: Some("GetURL(<Object>)".to_string()),
            }),
        }]
        .into_iter()
        .map(index_page)
        .collect();
        let ru = rank_pages(&pages, "GetURL", 20, "8.3.27.2074", "syntax-context", "ru");
        let en = rank_pages(&pages, "GetURL", 20, "8.3.27.2074", "syntax-context", "en");
        assert_eq!(
            ru[0].signature.as_deref(),
            Some("ПолучитьНавигационнуюСсылку(<Объект>)")
        );
        assert_eq!(
            en[0].signature.as_deref(),
            Some("GetURL(<Object>)"),
            "запрошенный язык обязан выбирать локаль сигнатуры"
        );
    }

    /// Прошлое ревью отметило, что решение «секция на корпус» до сих пор
    /// ничем не проверено: поставщик объявляет два корпуса (`corpora()`
    /// возвращает два элемента), и `search` обязан вернуть по секции на
    /// каждый, а не смешивать оба в одну. Мутация, схлопывающая секции в
    /// одну или путающая, из какого корпуса попадание, обязана ронять именно
    /// этот тест.
    #[test]
    fn full_installation_returns_two_sections_scoped_to_their_own_corpus() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
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

        let provider = PlatformSyntaxHelpProvider::new();
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

    /// Повреждённый контейнер исчезал молча, и секция сообщала `Empty` —
    /// «ничего не нашлось», — тогда как правда «контейнер не разобрался».
    /// Это ровно то смешение отказов, ради разделения которых прошёл
    /// отдельный круг правок (п.8 и п.10 ADR-0029).
    #[test]
    fn unparsable_container_is_named_instead_of_looking_empty() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(root.join("shcntx_ru.hbk"), b"not a container at all")
            .expect("повреждённый контейнер");

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("Alpha"), &context);
        let syntax_section = sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context");
        match &syntax_section.status {
            DocumentationSectionStatus::Failed { diagnostic } => assert!(
                diagnostic.contains("shcntx_ru.hbk"),
                "диагностика обязана назвать контейнер, получено {diagnostic}"
            ),
            other => panic!("ожидался Failed с именем контейнера, получено {other:?}"),
        }
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
        let provider = PlatformSyntaxHelpProvider::new();
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
        // Каталог не создаём — `discover` должен вернуть `NotFound`.
        let root = dir.path().join("8.3.27.2074");
        // Захватываем текст ДО перемещения `root` в контекст: он должен
        // войти в diagnostic-текст секции дословно.
        let expected_detail = format!("каталог установки недоступен: {}", root.display());
        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(sections.len(), 1);
        match &sections[0].status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::NotConfigured);
                // Точное совпадение, не `contains`: ревью потребовало, чтобы
                // NotFound и VersionUndetermined давали РАЗНЫЙ текст. Общая
                // фраза без пути (как было до правки) сюда не попадёт.
                assert_eq!(
                    *detail, expected_detail,
                    "NotFound обязан называть путь и не совпадать текстом с VersionUndetermined"
                );
            }
            other => panic!("ожидался Unavailable, получено {other:?}"),
        }
    }

    /// У корня файловой системы нет последнего сегмента, значит версию из
    /// пути не вывести: `discover` вернёт `VersionUndetermined`. Тот же
    /// фикстурный приём, что и в
    /// `installation.rs::root_without_a_last_segment_reports_version_undetermined`.
    #[test]
    fn root_without_last_segment_reports_not_configured() {
        let root = std::path::PathBuf::from("/");
        let expected_detail = format!("версия не выводится из корня установки: {}", root.display());
        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: None,
            installation_root: Some(root),
        };
        let sections = provider.search(&request("что-нибудь"), &context);
        assert_eq!(sections.len(), 1);
        match &sections[0].status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::NotConfigured);
                // Точное совпадение, не `contains`: тот же аргумент, что и в
                // `nonexistent_installation_root_reports_not_configured` —
                // текст обязан отличаться от NotFound, а не просто содержать
                // общее слово.
                assert_eq!(
                    *detail, expected_detail,
                    "VersionUndetermined обязан называть путь и не совпадать текстом с NotFound"
                );
            }
            other => panic!("ожидался Unavailable, получено {other:?}"),
        }
    }

    /// Индекс общий на процесс: паника, случившаяся под его замком (например,
    /// в разборе одного повреждённого контейнера), не должна отравлять
    /// мьютекс навсегда — иначе КАЖДЫЙ следующий `search()` для любого
    /// запроса и любой установки начнёт падать до конца жизни процесса. Тот
    /// же приём восстановления, что и в
    /// `workspace_services.rs::analyzer_lane_recovers_from_poison_without_losing_progress`:
    /// поток берёт замок и паникует, не отпуская его; `join()` результат
    /// игнорируется (нам важен сам факт отравления, не то, что вернул поток).
    #[test]
    fn search_recovers_from_poisoned_index_mutex() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        let page = "<html><body><h1>Alpha</h1><p>текст</p></body></html>";
        std::fs::write(
            root.join("shcntx_ru.hbk"),
            hbk_bytes(&[("alpha.html", page)]),
        )
        .expect("контейнер синтакс-помощника");

        let _ = std::thread::spawn(|| {
            // `unwrap_or_else`, а не `unwrap`: если слот отравил уже другой
            // тест, `unwrap` паниковал бы ДО намеренной паники, и тест перестал
            // бы отличать восстановление от совпадения.
            let _guard = index_slot()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            panic!("намеренное отравление индекса корпусов (тест)");
        })
        .join();

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        // Не должно паниковать: отравленный мьютекс обязан восстанавливаться
        // и отвечать, а не ронять вызывающего.
        let sections = provider.search(&request("Alpha"), &context);
        assert_eq!(
            sections.len(),
            2,
            "поставщик отвечает двумя секциями, а не падает после отравления"
        );
        let syntax_section = sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context");
        assert!(matches!(
            syntax_section.status,
            DocumentationSectionStatus::Ok
        ));
        assert_eq!(syntax_section.hits.len(), 1);
    }

    /// Сравнение ключа индекса — единственное, что не позволяет смешать
    /// корпуса разных версий платформы в памяти одного процесса (между
    /// 8.3.27 и 8.5.4 расходятся сотни имён API). Остальные тесты обращаются
    /// к одной установке, поэтому ветку перестройки не различают. Здесь две
    /// РАЗНЫЕ установленные версии опрашиваются подряд через общий индекс.
    #[test]
    fn second_installation_does_not_answer_from_the_first_ones_index() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");

        let first_root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&first_root).expect("каталог версии 1");
        std::fs::write(
            first_root.join("shcntx_ru.hbk"),
            hbk_bytes(&[(
                "first.html",
                "<html><body><h1>ОбщийРеквизитПервойВерсии</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("контейнер версии 1");

        let second_root = dir.path().join("8.5.4.1306");
        std::fs::create_dir_all(&second_root).expect("каталог версии 2");
        std::fs::write(
            second_root.join("shcntx_ru.hbk"),
            hbk_bytes(&[(
                "second.html",
                "<html><body><h1>ОбщийРеквизитВторойВерсии</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("контейнер версии 2");

        let provider = PlatformSyntaxHelpProvider::new();

        let first_context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(first_root),
        };
        let first_sections = provider.search(&request("ОбщийРеквизит"), &first_context);
        let first_syntax = first_sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context (версия 1)");
        assert_eq!(first_syntax.hits.len(), 1);
        assert_eq!(first_syntax.hits[0].applicable_version, "8.3.27.2074");

        let second_context = DocumentationContext {
            platform_version: Some("8.5.4.1306".to_string()),
            installation_root: Some(second_root),
        };
        let second_sections = provider.search(&request("ОбщийРеквизит"), &second_context);
        let second_syntax = second_sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context (версия 2)");
        assert_eq!(
            second_syntax.hits.len(),
            1,
            "второй ответ обязан содержать только страницы второй версии"
        );
        assert_eq!(
            second_syntax.hits[0].document_id, "platform-syntax-help:syntax-context:second.html",
            "попадание не должно быть страницей первой версии"
        );
        assert_eq!(second_syntax.hits[0].applicable_version, "8.5.4.1306");
    }

    /// Индекс переживает вызов: второй `search` по той же установке обязан
    /// отвечать из уже построенного индекса, а не читать установку заново.
    /// Наблюдаемо это так — контейнер удаляется между вызовами: заново
    /// прочитать его уже нельзя, и ответ из индекса отличим от перестройки.
    #[test]
    fn a_second_call_answers_from_the_index_instead_of_rereading_the_installation() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        let container = root.join("shcntx_ru.hbk");
        std::fs::write(
            &container,
            hbk_bytes(&[(
                "alpha.html",
                "<html><body><h1>Alpha</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("контейнер синтакс-помощника");

        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        // Разные экземпляры поставщика — ровно то, что делает диспетчер:
        // реестр собирается заново на каждый вызов, поэтому индекс обязан
        // жить не в экземпляре.
        let first = PlatformSyntaxHelpProvider::new().search(&request("Alpha"), &context);
        assert_eq!(first[0].hits.len(), 1, "первый вызов строит индекс");

        // Содержимое контейнера подменяется на мусор: перестройка увидела бы
        // неразобравшийся контейнер и ответила `Failed` без попаданий.
        std::fs::write(&container, b"not a container at all").expect("контейнер испорчен");

        let second = PlatformSyntaxHelpProvider::new().search(&request("Alpha"), &context);
        let syntax = second
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context");
        assert!(
            matches!(syntax.status, DocumentationSectionStatus::Ok),
            "второй вызов обязан отвечать из индекса, получено {:?}",
            syntax.status
        );
        assert_eq!(syntax.hits.len(), 1, "попадание приходит из индекса");
    }
}
