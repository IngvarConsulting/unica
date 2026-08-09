use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::domain::documentation::*;

use super::container::ContainerError;
use super::corpus::{read_corpus, CorpusError, CorpusPage, Signature};
use super::installation::{discover, CorpusContainers, InstallationCorpora, InstallationError};

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
    /// Локаль, в которой корпус реально прочитан: она уходит в секцию ответа.
    language: String,
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

/// Ключ индекса — каталог установки и сами ВЫБРАННЫЕ контейнеры обоих
/// корпусов, а не запрошенная локаль. Прежний ключ по запрошенной локали
/// делал из `en`, `root` и локали с опечаткой три разных ключа над одними и
/// теми же файлами: единственный слот перестраивался на каждом чередовании
/// таких запросов — секунды индексирования реальной установки на вызов, —
/// хотя на диске не менялось ничего. Разрешённая локаль в ключ тоже не
/// годится: у пустого корпуса она — эхо запроса. Списки выбранных
/// контейнеров детерминированы (`discover` сортирует их) и совпадают ровно
/// тогда, когда оба вызова читали бы одни и те же файлы; абсолютные пути
/// заодно различают одноимённые каталоги версий под разными корнями.
/// Отпечаток выбранного контейнера: путь, длина и время изменения. Второй и
/// третий элементы — то, что меняет переустановка платформы в тот же каталог:
/// имена файлов при ней те же, и ключ из одних путей отдавал бы устаревшую
/// справку до перезапуска процесса. Отпечаток собирается одним `stat` без
/// чтения содержимого, поэтому неизменная установка по-прежнему отвечает из
/// индекса, не перечитывая ни байта корпусов. Файл, пропавший между
/// `discover` и `stat`, несёт `None`: ключ остаётся детерминированным, а
/// чтение назовёт пропажу своей диагностикой.
type ContainerFingerprint = (PathBuf, Option<(u64, std::time::SystemTime)>);

#[derive(PartialEq, Eq)]
struct IndexKey {
    root: PathBuf,
    containers: Vec<ContainerFingerprint>,
}

impl IndexKey {
    fn for_corpora(root: &Path, corpora: &InstallationCorpora) -> IndexKey {
        let fingerprint = |path: &PathBuf| -> ContainerFingerprint {
            let stamp = std::fs::metadata(path)
                .ok()
                .and_then(|meta| meta.modified().ok().map(|mtime| (meta.len(), mtime)));
            (path.clone(), stamp)
        };
        IndexKey {
            root: root.to_path_buf(),
            containers: corpora
                .syntax_context
                .containers
                .iter()
                .chain(corpora.platform_guides.containers.iter())
                .map(fingerprint)
                .collect(),
        }
    }
}

struct CachedIndex {
    key: IndexKey,
    index: Arc<InstallationIndex>,
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
/// пишется ничего (п.11 ADR-0029).
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

/// Причина, по которой контейнер не стал корпусом, — словами, а не отладочным
/// представлением: этот текст уходит в диагностику секции и попадает
/// пользователю, которому идентификатор варианта вроде `BadBlockHeader`
/// ничего не говорит. Варианты обёрнутой ошибки при этом различаются
/// текстом: без этого «не разобрался» не отличает усечённый файл от чужого
/// формата.
fn corpus_failure(error: &CorpusError) -> String {
    match error {
        CorpusError::Container(ContainerError::TruncatedBlock) => {
            "контейнер обрезан: в файле меньше данных, чем объявляют его блоки".to_string()
        }
        CorpusError::Container(ContainerError::BadBlockHeader) => {
            "файл не является контейнером справки: на месте заголовка блока чужая разметка"
                .to_string()
        }
        CorpusError::MissingFileStorage => "в контейнере нет записи FileStorage".to_string(),
        CorpusError::BadArchive => "запись FileStorage не читается как ZIP".to_string(),
    }
}

fn index_corpus(source: &CorpusContainers) -> IndexedCorpus {
    let mut pages = Vec::new();
    let mut unreadable = Vec::new();
    for path in &source.containers {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("<без имени>")
            .to_string();
        match std::fs::read(path) {
            Err(error) => unreadable.push(format!("{name}: {error}")),
            Ok(bytes) => match read_corpus(&bytes) {
                Err(error) => unreadable.push(format!("{name}: {}", corpus_failure(&error))),
                Ok(read) => pages.extend(read.into_iter().map(index_page)),
            },
        }
    }
    IndexedCorpus {
        pages,
        language: source.language.clone(),
        unreadable,
    }
}

fn build_index(corpora: &InstallationCorpora) -> InstallationIndex {
    // Соответствие с `CORPUS_SPECS` позиционное: компилятор держит длину
    // массива, а порядок объявлен здесь. Добавленный корпус не соберётся,
    // пока его контейнеры не названы.
    let sources: [&CorpusContainers; CORPUS_SPECS.len()] =
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

/// Сигнатура на локали КОРПУСА, а не запроса. `.st` хранит обе локали, поэтому
/// выбор — вопрос предпочтения, а не наличия: одна идёт первой, вторая остаётся
/// запасной, чтобы вопрос про член платформы не остался вовсе без сигнатуры.
///
/// Локаль берётся у корпуса, потому что сигнатура печатается рядом с
/// заголовком и фрагментом ЕГО страницы. Измерено на 8.3.27.2074: запрос на
/// любую из 22 локалей, кроме `ru` и `en`, разрешается в английский `root`, и
/// при выборе по локали запроса пользователь получал «Global context.GetURL»
/// рядом с сигнатурой `ПолучитьНавигационнуюСсылку()`. То же самое случалось с
/// `root` — то есть со значением, которое инструмент сам возвращает в
/// `section.language`: подставить его обратно значило получить ответ хуже, чем
/// от неточного `en`.
///
/// Кириллица — только у `ru`: Синтакс-помощник вендор поставляет в двух
/// локалях, `_ru` и английской `_root`, поэтому «не `ru` — значит английский»
/// покрывает и `root`, и гипотетический `shcntx_<локаль>.hbk` любой другой
/// локали, где английская сигнатура рядом с нерусской страницей всё равно
/// уместнее русской.
fn signature_in(signature: &Signature, language: &str) -> Option<String> {
    let (preferred, fallback) = match language {
        "ru" => (&signature.ru, &signature.en),
        _ => (&signature.en, &signature.ru),
    };
    preferred.clone().or_else(|| fallback.clone())
}

fn rank_pages(
    pages: &[IndexedPage],
    query: &str,
    limit: usize,
    version: &str,
    // Идентификатор поставщика приходит от вызывающего (`self.id()`), а не
    // повторяется здесь литералом: переименование поставщика иначе молча
    // разошлось бы с префиксом `document_id`.
    provider: &str,
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
            document_id: format!("{provider}:{corpus}:{}", page.path),
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
    fn diagnostic(
        &self,
        language: &str,
        status: DocumentationSectionStatus,
    ) -> Vec<DocumentationSection> {
        let spec = &CORPUS_SPECS[0];
        vec![DocumentationSection {
            provider: self.id(),
            corpus: spec.id.to_string(),
            source_kind: spec.source_kind,
            authority: spec.authority,
            // Прочитать не удалось ничего, поэтому «ответившей» локали нет:
            // называется запрошенная — та, на которой ответа не получилось.
            language: language.to_string(),
            status,
            warnings: Vec::new(),
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

    /// Страница целиком по локатору `platform-syntax-help:<корпус>:<путь>` —
    /// доказательство по ADR-0029 п.4, а не фрагмент. Индекс полного текста
    /// не держит (он экономит память под поиск), поэтому страница
    /// перечитывается из контейнеров выбранной установки; `get` редок, и цена
    /// перечитывания честнее удвоения резидента индекса.
    fn get(
        &self,
        document_id: &str,
        language: &str,
        context: &DocumentationContext,
    ) -> Option<Result<crate::domain::documentation::DocumentationDocument, String>> {
        let rest = document_id.strip_prefix("platform-syntax-help:")?;
        // Префикс наш — дальнейшие неожиданности отвечаются отказом
        // владельца, а не тихим None: другой владелец у локатора невозможен.
        let Some((corpus_id, path)) = rest.split_once(':') else {
            return Some(Err(format!(
                "локатор {document_id:?} не несёт корпуса и пути"
            )));
        };
        let Some(root) = context.installation_root.as_ref() else {
            let detail = match context.platform_version.as_deref() {
                Some(version) => {
                    format!("установка платформы {version} не разрешена для рабочего пространства")
                }
                None => "установка платформы не разрешена для рабочего пространства".to_string(),
            };
            return Some(Err(detail));
        };
        let corpora = match discover(root, language) {
            Ok(value) => value,
            Err(error) => return Some(Err(format!("установка не разобрана: {error:?}"))),
        };
        let source = match corpus_id {
            "syntax-context" => &corpora.syntax_context,
            "platform-guides" => &corpora.platform_guides,
            other => return Some(Err(format!("неизвестный корпус {other:?} в локаторе"))),
        };
        for container in &source.containers {
            let Ok(bytes) = std::fs::read(container) else {
                continue;
            };
            let Ok(pages) = read_corpus(&bytes) else {
                continue;
            };
            if let Some(page) = pages.into_iter().find(|page| page.path == path) {
                return Some(Ok(crate::domain::documentation::DocumentationDocument {
                    provider: self.id(),
                    corpus: corpus_id.to_string(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                    // Локаль, которой корпус реально прочитан, — как у поиска.
                    language: source.language.clone(),
                    document_id: document_id.to_string(),
                    title: page.title.clone(),
                    signature: page
                        .signature
                        .as_ref()
                        .and_then(|value| signature_in(value, &source.language)),
                    applicable_version: corpora.version.clone(),
                    text: page.text,
                }));
            }
        }
        Some(Err(format!(
            "страницы {path:?} нет в корпусе {corpus_id} установки {}",
            corpora.version
        )))
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
            return self.diagnostic(
                &request.language,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::NotConfigured,
                    detail,
                },
            );
        };
        let corpora = match discover(root, &request.language) {
            Ok(value) => value,
            Err(InstallationError::HelpMissingForVersion { version }) => {
                // Ни в одной локали: локаль запроса тут ни при чём, потому что
                // отсутствующую подменяет установленная. Отказ означает именно
                // клиентскую поставку без Синтакс-помощника вовсе.
                return self.diagnostic(
                    &request.language,
                    DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::VersionMissing,
                        detail: format!(
                            "установка {version} не содержит Синтакс-помощника ни в одной локали; нужна полная поставка"
                        ),
                    },
                );
            }
            Err(InstallationError::Unreadable { detail }) => {
                return self.diagnostic(
                    &request.language,
                    DocumentationSectionStatus::Failed {
                        diagnostic: format!("каталог установки не читается: {detail}"),
                    },
                );
            }
            // NotFound и VersionUndetermined и раньше давали один и тот же
            // `reason`, но обязаны различаться текстом: иначе вызывающий не
            // отличит «каталога нет» от «версию не вывести из пути» ни по
            // чему, кроме кода, а не по ответу.
            Err(InstallationError::VersionUndetermined) => {
                return self.diagnostic(
                    &request.language,
                    DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::NotConfigured,
                        detail: format!(
                            "версия не выводится из корня установки: {}",
                            root.display()
                        ),
                    },
                );
            }
            Err(InstallationError::NotFound) => {
                return self.diagnostic(
                    &request.language,
                    DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::NotConfigured,
                        detail: format!("каталог установки недоступен: {}", root.display()),
                    },
                );
            }
        };
        let index = indexed(IndexKey::for_corpora(root, &corpora), &corpora);
        let provider_id = self.id().to_string();
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
                    &provider_id,
                    spec.id,
                    // Локаль КОРПУСА, а не запроса: сигнатура печатается рядом
                    // с заголовком и фрагментом его страницы (см. `signature_in`).
                    &corpus.language,
                );
                // `Empty` означает «корпус прочитан, ничего не совпало». Если
                // хоть один контейнер не разобрался, это неправда: совпадение
                // могло лежать именно в нём. Найденные попадания при этом
                // настоящие, поэтому непустая выдача остаётся `Ok` — но
                // непрочитавшиеся контейнеры называются предупреждениями:
                // молчание выдавало бы частичный корпус за целый.
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
                let warnings = if hits.is_empty() {
                    // Failed уже несёт весь список диагностикой, Empty без
                    // пропаж предупреждать не о чем.
                    Vec::new()
                } else {
                    corpus.unreadable.clone()
                };
                DocumentationSection {
                    provider: self.id(),
                    corpus: spec.id.to_string(),
                    source_kind: spec.source_kind,
                    authority: spec.authority,
                    // Локаль КОРПУСА, а не запроса: справка платформы есть не
                    // во всех локалях, и подмена обязана быть названа.
                    language: corpus.language.clone(),
                    status,
                    warnings,
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
        request_in(query, "ru")
    }

    fn request_in(query: &str, language: &str) -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: query.to_string(),
            source_kinds: vec![SourceKind::PlatformHelp],
            limit: 20,
            language: language.to_string(),
        }
    }

    fn syntax_section(sections: &[DocumentationSection]) -> &DocumentationSection {
        sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .expect("секция syntax-context")
    }

    /// Подменяет содержимое контейнера мусором, сохраняя его отпечаток —
    /// длину и время изменения. Тесты «ответ приходит из индекса» различают
    /// перестройку именно так: перечитать файл осмысленно уже нельзя, а ключ
    /// индекса не изменился, потому что не изменился отпечаток.
    fn corrupt_preserving_fingerprint(container: &std::path::Path) {
        let length = std::fs::metadata(container)
            .expect("метаданные контейнера")
            .len() as usize;
        let mtime = std::fs::metadata(container)
            .expect("метаданные контейнера")
            .modified()
            .expect("время изменения");
        std::fs::write(container, vec![b'x'; length]).expect("контейнер испорчен");
        std::fs::OpenOptions::new()
            .write(true)
            .open(container)
            .expect("контейнер открыт")
            .set_modified(mtime)
            .expect("время изменения возвращено");
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
            "platform-syntax-help",
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

    /// Локаль корпуса выбирает локаль сигнатуры. `root` проверяется наравне с
    /// `ru` и `en`, потому что именно им отвечает установка на 22 запрошенные
    /// локали из 24 — и им же назван `section.language`, который вызывающий
    /// может подставить обратно. Пока `root` не был английским, ответом на
    /// `de` и на сам `root` была английская страница «Global context.GetURL»
    /// рядом с кириллической сигнатурой `ПолучитьНавигационнуюСсылку()`.
    #[test]
    fn the_corpus_locale_picks_the_signature_locale() {
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
        let signature_for = |locale: &str| {
            rank_pages(
                &pages,
                "GetURL",
                20,
                "8.3.27.2074",
                "platform-syntax-help",
                "syntax-context",
                locale,
            )[0]
            .signature
            .clone()
        };
        assert_eq!(
            signature_for("ru").as_deref(),
            Some("ПолучитьНавигационнуюСсылку(<Объект>)"),
            "кириллица — только у русского корпуса"
        );
        assert_eq!(
            signature_for("en").as_deref(),
            Some("GetURL(<Object>)"),
            "локаль корпуса обязана выбирать локаль сигнатуры"
        );
        for locale in ["root", "de"] {
            assert_eq!(
                signature_for(locale).as_deref(),
                Some("GetURL(<Object>)"),
                "локаль {locale} — не русская, значит сигнатура английская, а не кириллическая рядом с нерусской страницей"
            );
        }
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
    /// отдельный круг правок (п.10 и п.12 ADR-0029).
    #[test]
    fn unparsable_container_is_named_instead_of_looking_empty() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        // Длиннее заголовка блока (16 + 31 байт): иначе отказ был бы
        // «обрезок», а проверяется здесь именно чужая разметка на месте
        // заголовка блока.
        std::fs::write(root.join("shcntx_ru.hbk"), vec![b'x'; 256])
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
            DocumentationSectionStatus::Failed { diagnostic } => {
                assert!(
                    diagnostic.contains("shcntx_ru.hbk"),
                    "диагностика обязана назвать контейнер, получено {diagnostic}"
                );
                // Причина — словами: этот текст уходит пользователю, и
                // отладочный идентификатор варианта ему ничего не говорит.
                assert!(
                    diagnostic.contains("чужая разметка"),
                    "диагностика обязана назвать причину словами, получено {diagnostic}"
                );
                assert!(
                    !diagnostic.contains("BadBlockHeader"),
                    "отладочный идентификатор не должен уходить пользователю, получено {diagnostic}"
                );
            }
            other => panic!("ожидался Failed с именем контейнера, получено {other:?}"),
        }
    }

    /// Непустая выдача при частично неразобравшемся корпусе оставалась `Ok`
    /// БЕЗ СЛЕДА пропажи: список unreadable молча выбрасывался, и частичный
    /// корпус выдавался за целый. Найденные попадания настоящие, поэтому
    /// статус — по-прежнему `Ok`, но непрочитавшийся контейнер обязан быть
    /// назван предупреждением секции: совпадение могло лежать именно в нём.
    #[test]
    fn a_broken_container_next_to_a_readable_one_is_named_as_a_warning() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(
            root.join("shcntx_ru.hbk"),
            hbk_bytes(&[(
                "alpha.html",
                "<html><body><h1>Alpha</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("целый контейнер");
        // Руководства: один целый и один битый контейнер в ОДНОМ корпусе.
        std::fs::write(
            root.join("1cv8_ru.hbk"),
            hbk_bytes(&[(
                "guide.html",
                "<html><body><h1>Alpha в руководстве</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("целый контейнер руководств");
        std::fs::write(root.join("mngbase_ru.hbk"), vec![b'x'; 256])
            .expect("битый контейнер руководств");

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request("Alpha"), &context);
        let guides = sections
            .iter()
            .find(|section| section.corpus == "platform-guides")
            .expect("секция platform-guides");
        assert!(
            matches!(guides.status, DocumentationSectionStatus::Ok),
            "настоящие попадания остаются Ok, получено {:?}",
            guides.status
        );
        assert_eq!(guides.hits.len(), 1, "попадание из целого контейнера");
        assert_eq!(
            guides.warnings.len(),
            1,
            "непрочитавшийся контейнер обязан быть назван предупреждением"
        );
        assert!(
            guides.warnings[0].contains("mngbase_ru.hbk"),
            "предупреждение обязано назвать контейнер, получено {}",
            guides.warnings[0]
        );
        // Целый корпус рядом предупреждений не несёт.
        let syntax = syntax_section(&sections);
        assert!(
            syntax.warnings.is_empty(),
            "целый корпус не должен нести предупреждений, получено {:?}",
            syntax.warnings
        );
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

    /// Установка, у которой Синтакс-помощник лежит в двух локалях. Одноимённые
    /// страницы в разных контейнерах различимы по адресу, поэтому по ответу
    /// видно, какой контейнер прочитали.
    fn bilingual_installation(dir: &std::path::Path) -> std::path::PathBuf {
        let root = dir.join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        for (name, path, title) in [
            ("shcntx_ru.hbk", "ru/alpha.html", "Alpha по-русски"),
            ("shcntx_en.hbk", "en/alpha.html", "Alpha in English"),
        ] {
            std::fs::write(
                root.join(name),
                hbk_bytes(&[(
                    path,
                    &format!("<html><body><h1>{title}</h1><p>текст</p></body></html>"),
                )]),
            )
            .expect("контейнер");
        }
        root
    }

    /// Аргумент `language` отбирает контейнеры, а не только локаль сигнатуры.
    /// Ревью применило мутацию `discover(root, "ru")` вместо
    /// `discover(root, &request.language)` — и все 2021 тест остались
    /// зелёными: единственный тест на язык (`requested_language_picks_the_signature_locale`)
    /// зовёт `rank_pages` напрямую и до `discover` не доходит. Здесь язык
    /// запроса решает, какой файл прочитан.
    #[test]
    fn the_requested_language_selects_the_containers_not_just_the_signature() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = bilingual_installation(dir.path());
        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request_in("Alpha", "en"), &context);
        let syntax = syntax_section(&sections);
        assert_eq!(
            syntax.hits.len(),
            1,
            "прочитан обязан быть ровно один контейнер, получено {:?}",
            syntax.status
        );
        assert_eq!(
            syntax.hits[0].document_id, "platform-syntax-help:syntax-context:en/alpha.html",
            "язык запроса обязан выбирать контейнер, а не только локаль сигнатуры"
        );
        assert_eq!(
            syntax.language, "en",
            "секция обязана называть локаль, на которой ответила"
        );
    }

    /// Установка 8.3.27.2074 несёт `shcntx_ru.hbk` и `shcntx_root.hbk`, но не
    /// `shcntx_en.hbk`. До правки `language: "en"` давал единственную секцию
    /// `Unavailable { VersionMissing }`, `any_usable` оставался ложным, и весь
    /// вызов заканчивался отказом — при том что английский контейнер лежит
    /// рядом. Теперь отвечает он, и секция это называет.
    #[test]
    fn a_language_the_installation_lacks_is_answered_by_another_and_named() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(
            root.join("shcntx_ru.hbk"),
            hbk_bytes(&[(
                "ru/alpha.html",
                "<html><body><h1>Alpha по-русски</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("русский контейнер");
        std::fs::write(
            root.join("shcntx_root.hbk"),
            hbk_bytes(&[(
                "root/alpha.html",
                "<html><body><h1>Alpha in English</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("английский контейнер");

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request_in("Alpha", "en"), &context);
        let syntax = syntax_section(&sections);
        assert!(
            matches!(syntax.status, DocumentationSectionStatus::Ok),
            "отсутствующая локаль обязана подменяться, а не ронять вызов; получено {:?}",
            syntax.status
        );
        assert_eq!(
            syntax.hits[0].document_id, "platform-syntax-help:syntax-context:root/alpha.html",
            "ответить обязан английский контейнер, а не русский"
        );
        assert_eq!(
            syntax.language, "root",
            "секция обязана называть локаль, которая ответила, а не запрошенную"
        );
    }

    /// Предыдущий тест зовёт `rank_pages` напрямую и не видит, ЧТО ей передаёт
    /// `search`. Здесь установка несёт только английский `shcntx_root.hbk`, а
    /// запрос идёт на `ru` — на локали по умолчанию, то есть без всякого
    /// экзотического ввода. Локаль корпуса (`root`) и локаль запроса (`ru`)
    /// расходятся, и подстановка второй вместо первой даёт кириллическую
    /// сигнатуру рядом с английской страницей — ровно то, что ревьюер измерил
    /// на реальной установке.
    #[test]
    fn the_answering_corpus_locale_not_the_request_picks_the_signature() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        std::fs::write(
            root.join("shcntx_root.hbk"),
            hbk_bytes(&[
                (
                    "objects/GetURL.html",
                    "<html><body><h1>Global context.GetURL</h1><p>текст</p></body></html>",
                ),
                (
                    "objects/GetURL.st",
                    "{1,\n{2,\n{\"\",1,0,\"\",\"\"},\n{0,\n{\"ru\",0,0,\"\",\"ПолучитьНавигационнуюСсылку()\"}\n},\n{0,\n{\"en\",0,0,\"\",\"GetURL()\"}\n}\n}\n}",
                ),
            ]),
        )
        .expect("английский контейнер");

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let sections = provider.search(&request_in("GetURL", "ru"), &context);
        let syntax = syntax_section(&sections);
        assert_eq!(syntax.language, "root", "ответила английская локаль");
        assert_eq!(
            syntax.hits[0].title, "Global context.GetURL",
            "страница английская"
        );
        assert_eq!(
            syntax.hits[0].signature.as_deref(),
            Some("GetURL()"),
            "сигнатура обязана быть на локали ОТВЕТИВШЕГО корпуса, а не запроса"
        );
    }

    /// Язык входит в ключ индекса. Мутация «`IndexKey.language` — константа»
    /// прошла все 2021 тест: остальные тесты кеша меняют КАТАЛОГ, а не язык,
    /// поэтому вторую половину ключа не различают. Здесь установка одна и та
    /// же, а язык между вызовами меняется, и ответ обязан меняться вместе с
    /// ним — правило INV-APP-DOCUMENTATION-NO-DISK-STATE держит именно это.
    #[test]
    fn a_second_language_on_the_same_installation_does_not_answer_from_the_first_ones_index() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = bilingual_installation(dir.path());
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };

        let first = PlatformSyntaxHelpProvider::new().search(&request_in("Alpha", "ru"), &context);
        assert_eq!(
            syntax_section(&first).hits[0].document_id,
            "platform-syntax-help:syntax-context:ru/alpha.html",
            "первый вызов строит индекс русских контейнеров"
        );

        let second = PlatformSyntaxHelpProvider::new().search(&request_in("Alpha", "en"), &context);
        let syntax = syntax_section(&second);
        assert_eq!(
            syntax.hits[0].document_id, "platform-syntax-help:syntax-context:en/alpha.html",
            "смена языка при том же каталоге обязана перестраивать индекс"
        );
        assert_eq!(syntax.language, "en");
    }

    /// Ключ индекса — разрешённые локали корпусов, а не запрошенная. Запросы
    /// `en` и `root` на установке без `shcntx_en.hbk` читают один и тот же
    /// английский контейнер, но с ключом по запрошенной локали это два разных
    /// ключа над одними корпусами: единственный слот перестраивался на каждом
    /// чередовании — 15 секунд индексирования реальной установки на вызов, —
    /// хотя на диске не менялось ничего. Наблюдаемо через порчу контейнеров:
    /// второй запрос, разрешающийся в ту же локаль, обязан ответить из
    /// индекса, а перестройка увидела бы испорченный файл.
    #[test]
    fn two_requests_resolving_to_the_same_locale_share_one_index() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        let container = root.join("shcntx_root.hbk");
        std::fs::write(
            &container,
            hbk_bytes(&[(
                "root/alpha.html",
                "<html><body><h1>Alpha in English</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("английский контейнер");

        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let first = PlatformSyntaxHelpProvider::new().search(&request_in("Alpha", "en"), &context);
        let first_syntax = syntax_section(&first);
        assert_eq!(first_syntax.language, "root", "en разрешается в root");
        assert_eq!(first_syntax.hits.len(), 1, "первый вызов строит индекс");

        // Перестройка прочитала бы мусор и ответила Failed; ответ из индекса
        // остаётся Ok. Отпечаток сохраняется, чтобы перепроверка ключа не
        // приняла порчу за переустановку.
        corrupt_preserving_fingerprint(&container);

        let second =
            PlatformSyntaxHelpProvider::new().search(&request_in("Alpha", "root"), &context);
        let second_syntax = syntax_section(&second);
        assert!(
            matches!(second_syntax.status, DocumentationSectionStatus::Ok),
            "запрос root разрешается в ту же локаль и обязан ответить из индекса, получено {:?}",
            second_syntax.status
        );
        assert_eq!(
            second_syntax.hits[0].document_id,
            "platform-syntax-help:syntax-context:root/alpha.html"
        );
        assert_eq!(second_syntax.language, "root");
    }

    /// `get` возвращает страницу ЦЕЛИКОМ — доказательство по ADR-0029 п.4,
    /// а не 400-символьный фрагмент индекса. Индекс полного текста не держит,
    /// поэтому страница перечитывается из контейнеров выбранной установки;
    /// локаль подставляется тем же правилом, что и у поиска, и называется в
    /// документе. Чужой префикс — «не мой локатор», свой префикс с пропавшей
    /// страницей — отказ владельца, называющий страницу.
    #[test]
    fn get_returns_the_full_page_text_not_the_snippet() {
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        // Текст заметно длиннее фрагмента (400): полнота наблюдаема.
        let body = "слово ".repeat(200);
        std::fs::write(
            root.join("shcntx_root.hbk"),
            hbk_bytes(&[(
                "objects/GetURL.html",
                &format!("<html><body><h1>Global context.GetURL</h1><p>{body}</p></body></html>"),
            )]),
        )
        .expect("контейнер");

        let provider = PlatformSyntaxHelpProvider::new();
        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let document = provider
            .get(
                "platform-syntax-help:syntax-context:objects/GetURL.html",
                "en",
                &context,
            )
            .expect("локатор наш")
            .expect("страница найдена");
        assert_eq!(document.title, "Global context.GetURL");
        assert!(
            document.text.chars().count() > 400,
            "текст обязан быть полным, а не фрагментом: {} символов",
            document.text.chars().count()
        );
        assert_eq!(
            document.language, "root",
            "локаль подставляется и называется, как у поиска"
        );
        assert_eq!(document.applicable_version, "8.3.27.2074");
        assert_eq!(document.corpus, "syntax-context");

        assert!(
            provider
                .get("https://kb.1ci.com/x/", "ru", &context)
                .is_none(),
            "чужой локатор — не мой, отвечает None"
        );
        let missing = provider
            .get(
                "platform-syntax-help:syntax-context:objects/Missing.html",
                "en",
                &context,
            )
            .expect("локатор наш");
        let error = missing.expect_err("пропавшая страница — отказ владельца");
        assert!(
            error.contains("objects/Missing.html"),
            "отказ обязан назвать страницу, получено {error}"
        );
    }

    /// Платформа, переустановленная в тот же каталог, — не экзотика, а
    /// штатное обновление сборки на месте. Индекс без перепроверки отдавал
    /// устаревшую справку до перезапуска процесса: имена файлов те же, ключ
    /// совпадает, и подменённый контейнер никто не перечитывал. Отпечаток
    /// выбранных контейнеров (длина и время изменения) входит в ключ, поэтому
    /// подмена файла на диске обязана перестроить индекс.
    #[test]
    fn a_replaced_container_is_reindexed_instead_of_answering_stale_help() {
        // Слот индекса общий на процесс — тесты, которые его пишут, идут по одному.
        let _serial = index_test_lock();
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(&root).expect("каталог версии");
        let container = root.join("shcntx_ru.hbk");
        std::fs::write(
            &container,
            hbk_bytes(&[(
                "old.html",
                "<html><body><h1>СтароеИмяЧлена</h1><p>текст</p></body></html>",
            )]),
        )
        .expect("контейнер прежней сборки");

        let context = DocumentationContext {
            platform_version: Some("8.3.27.2074".to_string()),
            installation_root: Some(root),
        };
        let first = PlatformSyntaxHelpProvider::new().search(&request("СтароеИмяЧлена"), &context);
        assert_eq!(
            syntax_section(&first).hits.len(),
            1,
            "первый вызов строит индекс прежней сборки"
        );

        // Переустановка на месте: то же имя файла, другое содержимое. Текст
        // новой страницы заметно длиннее, чтобы отличие отпечатка не
        // зависело от гранулярности времени изменения файловой системы.
        std::fs::write(
            &container,
            hbk_bytes(&[(
                "new.html",
                "<html><body><h1>НовоеИмяЧлена</h1><p>текст новой сборки, заметно длиннее прежнего, чтобы длина контейнера гарантированно изменилась</p></body></html>",
            )]),
        )
        .expect("контейнер новой сборки");

        let second = PlatformSyntaxHelpProvider::new().search(&request("НовоеИмяЧлена"), &context);
        let syntax = syntax_section(&second);
        assert_eq!(
            syntax.hits.len(),
            1,
            "подменённый контейнер обязан быть перечитан, получено {:?}",
            syntax.status
        );
        assert_eq!(
            syntax.hits[0].document_id, "platform-syntax-help:syntax-context:new.html",
            "ответ обязан прийти из новой сборки, а не из устаревшего индекса"
        );
    }

    /// Индекс переживает вызов: второй `search` по той же установке обязан
    /// отвечать из уже построенного индекса, а не читать установку заново.
    /// Наблюдаемо это так — содержимое контейнера подменяется мусором ТОЙ ЖЕ
    /// длины с возвращённым временем изменения: отпечаток не меняется, заново
    /// прочитать файл осмысленно уже нельзя, и ответ из индекса отличим от
    /// перестройки.
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

        // Содержимое контейнера подменяется мусором той же длины с
        // возвращённым временем изменения: перестройка увидела бы
        // неразобравшийся контейнер и ответила `Failed` без попаданий, а
        // неизменный отпечаток удерживает ответ из индекса.
        corrupt_preserving_fingerprint(&container);

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
