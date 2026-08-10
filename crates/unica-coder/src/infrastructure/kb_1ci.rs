//! Поставщик `kb-1ci`: руководства площадки вендора через её навигационное
//! дерево (ADR-0032 п.2–3, проектная записка — «Второй источник»).
//!
//! Дерево отдаёт `GET /bin/get/OnecInt/Extensions/GuideNavigation/
//! NavigationSource/WebHome?language=en&outputSyntax=plain&data=children&id=…`
//! — JSON вида `[{id, text, children, a_attr:{title, href}}]`; корень — `#`.
//! Узлы и адреса не зашиваются константами: руководства находятся по именам
//! категорий от корня, доступность выясняется при обращении (площадка за
//! время проектирования переезжала трижды, включая исчезновение файлового
//! режима администратора из дерева).
//!
//! Pretty-адрес из `a_attr.href` отдаёт SPA-оболочку без текста; серверный
//! рендер живёт в `/bin/view/OnecInt/KB/<сегменты>/?language=en` — признак
//! настоящей страницы `id="xwikicontent"`, заголовок — первый `<h1>`.
//! Сегменты выводятся из `id` узла: `OnecInt.KB.` и `.WebHome` отрезаются,
//! остальное делится по `.`, где `\.` — точка ВНУТРИ имени сегмента
//! (`1C_Enterprise_8\.3\.27_Developer_Guide`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Площадка вендора; поставщик собирает от неё и дерево, и контент.
pub const KB_BASE: &str = "https://kb.1ci.com";

/// Транспорт площадки. Один метод: `GET` с честным UA и таймаутом; продовая
/// реализация разносит обращения по времени. Отменяемость — забота
/// вызывающего между запросами: транспорт не знает про MCP.
pub trait KbTransport: Send + Sync {
    fn get(&self, url: &str) -> Result<String, String>;
}

/// Продовый транспорт: `ureq`, честный User-Agent, таймаут 30 с и не чаще
/// одного запроса в 500 мс на процесс — обращения разнесены по времени, как
/// того требует ADR-0032 п.3.
pub struct UreqKbTransport;

static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);
const REQUEST_SPACING: Duration = Duration::from_millis(500);
const USER_AGENT: &str =
    "unica-coder documentation provider (+https://github.com/IngvarConsulting/unica)";

impl KbTransport for UreqKbTransport {
    fn get(&self, url: &str) -> Result<String, String> {
        // Слот времени бронируется под замком, а сон идёт СНАРУЖИ: сон под
        // мьютексом заставил бы каждый параллельный запрос ждать весь чужой
        // интервал на замке, а не в своей очереди слотов.
        let wait = {
            let mut last = LAST_REQUEST
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = Instant::now();
            let slot = match *last {
                Some(previous) if previous + REQUEST_SPACING > now => previous + REQUEST_SPACING,
                _ => now,
            };
            *last = Some(slot);
            slot.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build()
            .get(url)
            .set("User-Agent", USER_AGENT)
            .call()
            .map_err(|error| error.to_string())?
            .into_string()
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct KbNode {
    pub id: String,
    pub title: String,
    pub has_children: bool,
    pub href: String,
}

#[derive(Debug, Clone)]
pub struct KbPage {
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct KbGuideVersion {
    pub version: String,
    pub node_id: String,
    /// Режим руководства администратора (текст узла категории режима);
    /// у руководства разработчика режима нет.
    pub mode: Option<String>,
}

#[derive(Debug, Default)]
pub struct GuideCatalog {
    pub developer: Vec<KbGuideVersion>,
    pub administrator: Vec<KbGuideVersion>,
}

/// Дети узла дерева. `id` корня — `#`.
pub fn children(
    transport: &dyn KbTransport,
    base: &str,
    node_id: &str,
) -> Result<Vec<KbNode>, String> {
    let encoded = url_encode(node_id);
    let url = format!(
        "{base}/bin/get/OnecInt/Extensions/GuideNavigation/NavigationSource/WebHome?language=en&outputSyntax=plain&data=children&id={encoded}"
    );
    let body = transport.get(&url)?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| format!("дерево навигации не разбирается: {error}"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| "дерево навигации обязано быть массивом".to_string())?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            Some(KbNode {
                id: entry.get("id")?.as_str()?.to_string(),
                title: entry.get("text")?.as_str()?.to_string(),
                has_children: entry
                    .get("children")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                href: entry
                    .get("a_attr")
                    .and_then(|attr| attr.get("href"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect())
}

fn url_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Сегменты контентного адреса из `id` узла, или `None` для чужого id.
/// `\.` — точка внутри имени сегмента, неэкранированная точка — разделитель.
pub fn content_segments(node_id: &str) -> Option<Vec<String>> {
    let rest = node_id.strip_prefix("OnecInt.KB.")?;
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            '.' => segments.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    segments.push(current);
    if segments.last().map(String::as_str) != Some("WebHome") {
        return None;
    }
    segments.pop();
    if segments.is_empty() {
        return None;
    }
    Some(segments)
}

/// Серверно отрендеренный адрес страницы узла: `/bin/view/OnecInt/KB/…`.
/// Pretty-href для него не годится — тот отдаёт SPA-оболочку без текста.
pub fn content_url(base: &str, node_id: &str) -> Option<String> {
    let segments = content_segments(node_id)?;
    Some(format!(
        "{base}/bin/view/OnecInt/KB/{}/?language=en",
        segments.join("/")
    ))
}

/// Текст без разметки, с пропуском содержимого `<script>` и `<style>`:
/// серверная страница несёт обвязку, и её скрипты не должны попадать во
/// фрагмент выдачи. Тот же посимвольный приём, что и у разбора корпуса
/// установки.
fn strip_markup_skipping_scripts(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut out = String::with_capacity(raw.len() / 4);
    let mut position = 0usize;
    let mut inside_tag = false;
    let bytes = raw.as_bytes();
    while position < bytes.len() {
        if !inside_tag {
            if lower[position..].starts_with("<script") || lower[position..].starts_with("<style") {
                let closer = if lower[position..].starts_with("<script") {
                    "</script"
                } else {
                    "</style"
                };
                match lower[position..].find(closer) {
                    Some(offset) => {
                        position += offset;
                        continue;
                    }
                    None => break,
                }
            }
            if bytes[position] == b'<' {
                inside_tag = true;
                position += 1;
                continue;
            }
            // Продвигаемся по границам символов, а не байтов.
            let ch_len = raw[position..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            out.push_str(&raw[position..position + ch_len]);
            position += ch_len;
        } else {
            if bytes[position] == b'>' {
                inside_tag = false;
                out.push(' ');
            }
            position += 1;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Читает серверно отрендеренную страницу. Оболочка без `xwikicontent` — то
/// самое «перенаправление на корень считается отсутствием страницы», а не
/// пустой текст.
///
/// Живая раскладка (зонд 2026-08-09): `<h1>` заголовка лежит в шапке
/// страницы, ВНЕ контентного div, поэтому заголовок ищется по всей странице.
/// Текст — от маркера контентной области до `</main>`: после него идёт футер
/// с иконками, который в тексте страницы не место. Страница-оглавление несёт
/// пустой контентный div — это пустой текст, и его толкует вызывающий:
/// поиск отвечает пустым фрагментом, получение — отказом «оглавление».
pub fn read_page(html: &str) -> Result<KbPage, String> {
    let lower = html.to_ascii_lowercase();
    let Some(content_start) = lower.find("id=\"xwikicontent\"") else {
        return Err(
            "страница не отдана: серверного рендера нет — оболочка или перенаправление на корень"
                .to_string(),
        );
    };
    let title = lower.find("<h1").and_then(|open| {
        let after_open = open + lower[open..].find('>')? + 1;
        let close = lower[after_open..].find("</h1")? + after_open;
        let inner = strip_markup_skipping_scripts(&html[after_open..close]);
        (!inner.is_empty()).then_some(inner)
    });
    // Начало текста — после закрытия открывающего тега контентного div,
    // чтобы его собственные атрибуты не попали в текст.
    let text_start = lower[content_start..]
        .find('>')
        .map(|offset| content_start + offset + 1)
        .unwrap_or(content_start);
    let text_end = lower[text_start..]
        .find("</main")
        .map(|offset| text_start + offset)
        .unwrap_or(html.len());
    let text = strip_markup_skipping_scripts(&html[text_start..text_end]);
    let title = match title {
        Some(title) => title,
        None => text.chars().take(120).collect(),
    };
    Ok(KbPage { title, text })
}

/// Первое версионное число вида `8.<…>` в тексте узла: не менее двух
/// точечных числовых составляющих. Версии в дереве живут внутри имён
/// («1C:Enterprise 8.3.27 Developer Guide»), отдельного поля у них нет.
fn version_in(title: &str) -> Option<String> {
    for token in title.split_whitespace() {
        let cleaned = token.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.');
        if cleaned.is_empty() {
            continue;
        }
        let parts: Vec<&str> = cleaned.split('.').collect();
        if parts.len() >= 2
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Some(cleaned.to_string());
        }
    }
    None
}

fn find_category<'nodes>(nodes: &'nodes [KbNode], needle: &str) -> Option<&'nodes KbNode> {
    nodes.iter().find(|node| node.title.contains(needle))
}

/// Каталог руководств от корня дерева: категории находятся по именам, версии
/// собираются с ОБОИХ уровней раскладки разработчика (прямые дети категории и
/// дети контейнеров без версии в имени), а режимы администратора берутся из
/// дерева, а не из списка, — файловый режим уже исчезал из него целиком, и
/// зашитый перечень режимов превратил бы переезд площадки в ложный отказ.
pub fn discover_guides(transport: &dyn KbTransport, base: &str) -> Result<GuideCatalog, String> {
    let roots = children(transport, base, "#")?;
    let platform = find_category(&roots, "1C:Enterprise Platform")
        .ok_or_else(|| "в дереве площадки нет раздела 1C:Enterprise Platform".to_string())?;
    let sections = children(transport, base, &platform.id)?;
    let guides = sections
        .iter()
        .find(|node| node.title.eq_ignore_ascii_case("guides"))
        .ok_or_else(|| "в разделе платформы нет категории Guides".to_string())?;
    let categories = children(transport, base, &guides.id)?;

    let mut catalog = GuideCatalog::default();
    if let Some(developer) = find_category(&categories, "Developer Guides") {
        for child in children(transport, base, &developer.id)? {
            if let Some(version) = version_in(&child.title) {
                if child.title.contains("Developer Guide") {
                    catalog.developer.push(KbGuideVersion {
                        version,
                        node_id: child.id.clone(),
                        mode: None,
                    });
                }
            } else if child.title.contains("Developer Guide") && child.has_children {
                // Контейнер текущих версий («1C:Enterprise Developer Guide»):
                // версии лежат уровнем ниже. Контейнеры без версионных детей
                // (например, руководство библиотеки) ничего не добавляют.
                for grandchild in children(transport, base, &child.id)? {
                    if let Some(version) = version_in(&grandchild.title) {
                        if grandchild.title.contains("Developer Guide") {
                            catalog.developer.push(KbGuideVersion {
                                version,
                                node_id: grandchild.id.clone(),
                                mode: None,
                            });
                        }
                    }
                }
            }
        }
    }
    if let Some(administrator) = find_category(&categories, "Administrator Guides") {
        for mode in children(transport, base, &administrator.id)? {
            if !mode.has_children {
                continue;
            }
            for child in children(transport, base, &mode.id)? {
                if let Some(version) = version_in(&child.title) {
                    catalog.administrator.push(KbGuideVersion {
                        version,
                        node_id: child.id.clone(),
                        mode: Some(mode.title.clone()),
                    });
                }
            }
        }
    }
    Ok(catalog)
}

/// Запись поискового индекса руководства: узел двухуровневого оглавления.
/// Поиск идёт по заголовкам узлов — сетевой корпус нельзя читать целиком на
/// каждый запрос, а оглавление и есть то, что дерево отдаёт дёшево.
#[derive(Debug, Clone)]
pub struct KbSearchEntry {
    pub node_id: String,
    pub title: String,
    pub href: String,
    pub version: String,
}

/// Кеш каталога руководств: один на процесс, ключ — база площадки.
/// Перестраивается при смене базы; на диск не пишется ничего.
static KB_CATALOG: Mutex<Option<(String, Instant, std::sync::Arc<GuideCatalog>)>> =
    Mutex::new(None);

/// Кеш оглавлений выбранных руководств: ключ — (база, узел руководства).
/// Ключи приходят из дерева самой площадки, поэтому множество ограничено
/// числом её руководств, а не чужим вводом.
type EntriesKey = (String, String);
type CachedEntries = (Instant, std::sync::Arc<Vec<KbSearchEntry>>);
static KB_ENTRIES: Mutex<std::collections::BTreeMap<EntriesKey, CachedEntries>> =
    Mutex::new(std::collections::BTreeMap::new());

/// Кеши общие на процесс: тесты, которым важно их состояние, идут по одному.
#[cfg(test)]
pub(crate) fn kb_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Страниц, дочитываемых на вызов: «число страниц на вызов ограничено»
/// (ADR-0032 п.3). Остальные совпадения отвечают заголовком и локатором.
const PAGE_FETCH_CAP: usize = 5;

/// Длина фрагмента выдачи — как у поставщика установки.
const SNIPPET_CHARS: usize = 400;

pub struct Kb1ciProvider {
    pub base: String,
    pub network: crate::infrastructure::documentation_policy::NetworkAccess,
    pub transport: std::sync::Arc<dyn KbTransport>,
    /// Токен вызова MCP: проверяется перед каждым сетевым запросом, отмена
    /// не публикует частичных секций поставщика.
    pub cancellation: crate::domain::cancellation::CancellationToken,
    /// Срок жизни кешей дерева площадки: она переезжала четырежды за время
    /// работы, и долгоживущий процесс сервера не должен требовать рестарта,
    /// чтобы увидеть новую раскладку и не отдавать устаревшие локаторы.
    pub cache_ttl: Duration,
    /// Источник лексикона ru↔en для расширения русских запросов к английским
    /// заголовкам площадки (ADR-0036 п.4).
    pub lexicon: std::sync::Arc<dyn KbLexiconSource>,
}

/// Источник двуязычного лексикона. Отделён трейтом: боевой источник отвечает
/// из процессного индекса справки установки, тестам нужен предсказуемый
/// словарь без установки на машине.
pub trait KbLexiconSource: Send + Sync {
    fn lexicon(
        &self,
        context: &crate::domain::documentation::DocumentationContext,
    ) -> Option<std::sync::Arc<crate::infrastructure::documentation_retrieval::BilingualLexicon>>;
}

/// Боевой источник: лексикон двуязычных заголовков Синтакс-помощника той
/// установки, которую диспетчер разрешил для вызова. Без установки — `None`,
/// и русский запрос к английским заголовкам честно остаётся без расширения.
pub struct InstallationLexiconSource;

impl KbLexiconSource for InstallationLexiconSource {
    fn lexicon(
        &self,
        context: &crate::domain::documentation::DocumentationContext,
    ) -> Option<std::sync::Arc<crate::infrastructure::documentation_retrieval::BilingualLexicon>>
    {
        let root = context.installation_root.as_ref()?;
        crate::infrastructure::platform_help::provider::bilingual_lexicon_for(root)
    }
}

/// Продовый срок жизни kb-кешей: часы держат цену обхода около нуля и
/// позволяют пережить переезд площадки без перезапуска процесса.
pub const KB_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

fn numeric_version(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Семейство сравнивается по числовым составляющим: `8.3.27` покрывает
/// `8.3.27` и любую его сборку, но не `8.3.2`.
/// Руководство подходит закреплению, когда одно из имён — числовой префикс
/// другого: запрос семейства выбирает его руководства, а пин сборки
/// (`8.3.27.2074`) усекается до семейства площадки (`8.3.27`). Чужое
/// семейство префиксом не является и не подставляется.
fn matches_family(version: &str, family: &str) -> bool {
    let version = numeric_version(version);
    let family = numeric_version(family);
    version.starts_with(&family) || family.starts_with(&version)
}

impl Kb1ciProvider {
    /// Источник недоступен целиком — одна диагностичная секция, происхождение
    /// от первого объявленного корпуса: тот же приём, что у поставщика
    /// установки (п.5 ADR-0029).
    fn diagnostic(
        &self,
        language: &str,
        status: crate::domain::documentation::DocumentationSectionStatus,
    ) -> Vec<crate::domain::documentation::DocumentationSection> {
        use crate::domain::documentation::*;
        vec![DocumentationSection {
            provider: DocumentationProviderId::new("kb-1ci"),
            corpus: "kb-developer-guide".to_string(),
            source_kind: SourceKind::PlatformHelp,
            authority: Authority::Vendor,
            language: language.to_string(),
            status,
            warnings: Vec::new(),
            hits: Vec::new(),
        }]
    }

    /// Каталог руководств из кеша процесса; смена базы перестраивает его.
    fn catalog(&self) -> Result<std::sync::Arc<GuideCatalog>, String> {
        {
            let slot = KB_CATALOG
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((base, born, catalog)) = slot.as_ref() {
                if *base == self.base && born.elapsed() < self.cache_ttl {
                    return Ok(std::sync::Arc::clone(catalog));
                }
            }
        }
        let catalog = std::sync::Arc::new(discover_guides(self.transport.as_ref(), &self.base)?);
        let mut slot = KB_CATALOG
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some((
            self.base.clone(),
            Instant::now(),
            std::sync::Arc::clone(&catalog),
        ));
        Ok(catalog)
    }

    /// Двухуровневое оглавление руководства из кеша процесса. Отменяемость
    /// проверяется перед каждым сетевым запросом: отмена не публикует
    /// частичных секций.
    fn entries_for(
        &self,
        guide: &KbGuideVersion,
    ) -> Result<std::sync::Arc<Vec<KbSearchEntry>>, String> {
        let key = (self.base.clone(), guide.node_id.clone());
        {
            let cache = KB_ENTRIES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((born, entries)) = cache.get(&key) {
                if born.elapsed() < self.cache_ttl {
                    return Ok(std::sync::Arc::clone(entries));
                }
            }
        }
        let mut entries = Vec::new();
        if self.cancellation.is_cancelled() {
            return Err("вызов отменён до обхода оглавления".to_string());
        }
        let chapters = children(self.transport.as_ref(), &self.base, &guide.node_id)?;
        for chapter in &chapters {
            entries.push(KbSearchEntry {
                node_id: chapter.id.clone(),
                title: chapter.title.clone(),
                href: chapter.href.clone(),
                version: guide.version.clone(),
            });
            if chapter.has_children {
                if self.cancellation.is_cancelled() {
                    return Err("вызов отменён посреди обхода оглавления".to_string());
                }
                for section in children(self.transport.as_ref(), &self.base, &chapter.id)? {
                    entries.push(KbSearchEntry {
                        node_id: section.id.clone(),
                        title: section.title.clone(),
                        href: section.href.clone(),
                        version: guide.version.clone(),
                    });
                }
            }
        }
        let entries = std::sync::Arc::new(entries);
        let mut cache = KB_ENTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(key, (Instant::now(), std::sync::Arc::clone(&entries)));
        Ok(entries)
    }

    /// Руководства корпуса под запрошенное семейство: на семейство — самая
    /// старшая его версия, без семейства — численно старшая вообще; у
    /// руководств администратора выбор идёт по каждому режиму отдельно.
    fn selected_guides(guides: &[KbGuideVersion], family: Option<&str>) -> Vec<KbGuideVersion> {
        let mut by_mode: std::collections::BTreeMap<Option<String>, KbGuideVersion> =
            std::collections::BTreeMap::new();
        for guide in guides {
            if let Some(family) = family {
                if !matches_family(&guide.version, family) {
                    continue;
                }
            }
            let slot = by_mode.entry(guide.mode.clone());
            match slot {
                std::collections::btree_map::Entry::Vacant(vacant) => {
                    vacant.insert(guide.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut occupied) => {
                    if numeric_version(&guide.version) > numeric_version(&occupied.get().version) {
                        occupied.insert(guide.clone());
                    }
                }
            }
        }
        by_mode.into_values().collect()
    }
}

impl crate::domain::documentation::DocumentationProvider for Kb1ciProvider {
    fn id(&self) -> crate::domain::documentation::DocumentationProviderId {
        crate::domain::documentation::DocumentationProviderId::new("kb-1ci")
    }

    fn corpora(&self) -> Vec<crate::domain::documentation::DocumentationCorpus> {
        use crate::domain::documentation::{Authority, DocumentationCorpus, SourceKind};
        vec![
            DocumentationCorpus {
                id: "kb-developer-guide".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
            },
            DocumentationCorpus {
                id: "kb-administrator-guide".to_string(),
                source_kind: SourceKind::PlatformHelp,
                authority: Authority::Vendor,
            },
        ]
    }

    fn needs_network(&self) -> bool {
        true
    }

    /// Страница целиком по pretty-локатору попадания: его путь зеркалит
    /// сегменты серверного рендера, поэтому дерево не обходится — один
    /// запрос `/bin/view`. Граница объявленных корней (ADR-0032 п.2)
    /// действует и здесь: адрес вне категорий руководств не запрашивается,
    /// а называется отказом.
    fn get(
        &self,
        document_id: &str,
        _language: &str,
        _context: &crate::domain::documentation::DocumentationContext,
    ) -> Option<Result<crate::domain::documentation::DocumentationDocument, String>> {
        use crate::domain::documentation::*;
        use crate::infrastructure::documentation_policy::NetworkAccess;

        let path = document_id.strip_prefix(&self.base)?;
        if self.network == NetworkAccess::Deny {
            return Some(Err(
                "сетевой выход kb-1ci запрещён политикой unica.toml".to_string()
            ));
        }
        if self.cancellation.is_cancelled() {
            return Some(Err("вызов отменён до обращения к площадке".to_string()));
        }
        let path = path.split('?').next().unwrap_or(path).trim_matches('/');
        // Путь локатора уходит в адрес запроса дословно: сегменты `..` (и
        // пустые) вывели бы запрос за объявленные корни руководств — граница
        // ADR-0032 п.2, действующая и на получение.
        if path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Some(Err(format!(
                "адрес {document_id:?} несёт сегменты '..' или пустые и не является адресом страницы руководства"
            )));
        }
        let corpus = if path.contains("Developer_Guides") {
            "kb-developer-guide"
        } else if path.contains("Administrator_Guides") {
            "kb-administrator-guide"
        } else {
            return Some(Err(format!(
                "адрес {document_id:?} вне объявленных корней руководств: запрашиваются только страницы категорий Developer Guides и Administrator Guides"
            )));
        };
        // Версия — из имени сегмента руководства; попадания всегда её несут.
        let version = path
            .split('/')
            .find_map(|segment| version_in(&segment.replace('_', " ")));
        let Some(version) = version else {
            return Some(Err(format!(
                "адрес {document_id:?} не несёт версии руководства"
            )));
        };
        let url = format!("{}/bin/view/OnecInt/KB/{}/?language=en", self.base, path);
        let html = match self.transport.get(&url) {
            Ok(html) => html,
            Err(error) => return Some(Err(format!("площадка недоступна: {error}"))),
        };
        let page = match read_page(&html) {
            Ok(page) => page,
            Err(error) => return Some(Err(format!("{url}: {error}"))),
        };
        // Пустой контентный div — страница-оглавление: текста у неё нет, и
        // выдавать пустоту за документ нечестно. Подраздел открывается по
        // своему локатору из выдачи.
        if page.text.trim().is_empty() {
            return Some(Err(format!(
                "{document_id}: страница — оглавление раздела и не несёт текста; откройте её подраздел по его локатору"
            )));
        }
        Some(Ok(DocumentationDocument {
            provider: DocumentationProviderId::new("kb-1ci"),
            corpus: corpus.to_string(),
            source_kind: SourceKind::PlatformHelp,
            authority: Authority::Vendor,
            language: "en".to_string(),
            document_id: document_id.to_string(),
            title: page.title,
            signature: None,
            applicable_version: version,
            text: page.text,
        }))
    }

    fn search(
        &self,
        request: &crate::domain::documentation::DocumentationSearchRequest,
        context: &crate::domain::documentation::DocumentationContext,
    ) -> Vec<crate::domain::documentation::DocumentationSection> {
        use crate::domain::documentation::*;
        use crate::infrastructure::documentation_policy::NetworkAccess;

        if self.network == NetworkAccess::Deny {
            return self.diagnostic(
                &request.language,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::PolicyDenied,
                    detail: "сетевой выход kb-1ci запрещён политикой unica.toml".to_string(),
                },
            );
        }
        if self.cancellation.is_cancelled() {
            return self.diagnostic(
                &request.language,
                DocumentationSectionStatus::Unavailable {
                    reason: UnavailableReason::Timeout,
                    detail: "вызов отменён до обхода площадки".to_string(),
                },
            );
        }
        let catalog = match self.catalog() {
            Ok(catalog) => catalog,
            Err(error) => {
                // Сеть недоступна, узел ответил ошибкой или истекло время —
                // unavailable у сетевой секции; локальные поставщики
                // продолжают отвечать (записка, «Отказы»).
                return self.diagnostic(
                    &request.language,
                    DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::Timeout,
                        detail: format!("площадка недоступна: {error}"),
                    },
                );
            }
        };
        let family = context.platform_version.as_deref();
        let corpora = [
            ("kb-developer-guide", &catalog.developer),
            ("kb-administrator-guide", &catalog.administrator),
        ];
        // Расширения ru→en считаются один раз на вызов: лексикон отвечает из
        // процессного индекса справки установки, без установки расширений нет
        // и русский запрос честно не совпадает с английскими заголовками.
        let expansions = self
            .lexicon
            .lexicon(context)
            .map(|lexicon| lexicon.expansions(&request.query))
            .unwrap_or_default();
        corpora
            .into_iter()
            .map(|(corpus_id, guides)| {
                let selected = Self::selected_guides(guides, family);
                if selected.is_empty() {
                    let mut available: Vec<String> = guides
                        .iter()
                        .map(|guide| guide.version.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    available.sort_by_key(|version| numeric_version(version));
                    let detail = if available.is_empty() {
                        "на площадке нет ни одной версии руководства".to_string()
                    } else {
                        format!(
                            "версии {} на площадке нет; доступны: {}",
                            family.unwrap_or("запрошенной"),
                            available.join(", ")
                        )
                    };
                    return DocumentationSection {
                        provider: DocumentationProviderId::new("kb-1ci"),
                        corpus: corpus_id.to_string(),
                        source_kind: SourceKind::PlatformHelp,
                        authority: Authority::Vendor,
                        language: request.language.clone(),
                        status: DocumentationSectionStatus::Unavailable {
                            reason: UnavailableReason::VersionMissing,
                            detail,
                        },
                        warnings: Vec::new(),
                        hits: Vec::new(),
                    };
                }
                let mut entries: Vec<KbSearchEntry> = Vec::new();
                let mut failure: Option<String> = None;
                for guide in &selected {
                    match self.entries_for(guide) {
                        Ok(guide_entries) => entries.extend(guide_entries.iter().cloned()),
                        Err(error) => {
                            failure = Some(error);
                            break;
                        }
                    }
                }
                // Матчинг — лексическим ядром по заголовкам узлов (ADR-0036
                // п.4): у сетевого корпуса дёшево есть только оглавление, и
                // индекс из одних заголовков строится на вызов. Порядок
                // совпадений — по оценке, а не по порядку обхода дерева.
                let index = crate::infrastructure::documentation_retrieval::RetrievalIndex::build(
                    entries.iter().map(|entry| {
                        crate::infrastructure::documentation_retrieval::RetrievalFields {
                            title: &entry.title,
                            signature: "",
                            body: "",
                        }
                    }),
                );
                let matched: Vec<(KbSearchEntry, f32)> = index
                    .query(&request.query, request.limit, &expansions)
                    .into_iter()
                    .map(|scored| (entries[scored.document].clone(), scored.score))
                    .collect();
                if let Some(error) = failure {
                    return DocumentationSection {
                        provider: DocumentationProviderId::new("kb-1ci"),
                        corpus: corpus_id.to_string(),
                        source_kind: SourceKind::PlatformHelp,
                        authority: Authority::Vendor,
                        language: request.language.clone(),
                        status: DocumentationSectionStatus::Unavailable {
                            reason: UnavailableReason::Timeout,
                            detail: format!("площадка недоступна: {error}"),
                        },
                        warnings: Vec::new(),
                        hits: Vec::new(),
                    };
                }
                let mut warnings = Vec::new();
                // Усечение пина сборки до семейства — не молчаливая
                // подстановка: секция называет и сборку, и семейство.
                if let Some(requested) = family {
                    let mut families: Vec<&str> = selected
                        .iter()
                        .filter(|guide| {
                            guide.version != requested
                                && numeric_version(requested)
                                    .starts_with(&numeric_version(&guide.version))
                        })
                        .map(|guide| guide.version.as_str())
                        .collect();
                    families.sort_unstable();
                    families.dedup();
                    if !families.is_empty() {
                        warnings.push(format!(
                            "сборка {requested} отвечена руководством семейства {}: площадка ведёт руководства по семействам",
                            families.join(", ")
                        ));
                    }
                }
                let mut cancelled_mid_ranking = false;
                let hits: Vec<DocumentationHit> = matched
                    .iter()
                    .enumerate()
                    .map(|(index, (entry, score))| {
                        // Страницы дочитываются только для верхних совпадений:
                        // «число страниц на вызов ограничено» (ADR-0032 п.3).
                        // Прочитанная страница отдаёт и фрагмент, и заголовок
                        // из своего <h1>: доказательство берётся у открытой
                        // страницы, а узел дерева остаётся запасным именем.
                        let mut title = entry.title.clone();
                        if self.cancellation.is_cancelled() {
                            cancelled_mid_ranking = true;
                        }
                        let snippet = if index < PAGE_FETCH_CAP && !cancelled_mid_ranking {
                            match content_url(&self.base, &entry.node_id)
                                .ok_or_else(|| "адрес страницы не строится".to_string())
                                .and_then(|url| {
                                    self.transport.get(&url).and_then(|html| {
                                        read_page(&html).map_err(|error| format!("{url}: {error}"))
                                    })
                                }) {
                                Ok(page) => {
                                    if !page.title.is_empty() {
                                        title = page.title;
                                    }
                                    page.text.chars().take(SNIPPET_CHARS).collect()
                                }
                                Err(error) => {
                                    warnings.push(error);
                                    String::new()
                                }
                            }
                        } else {
                            String::new()
                        };
                        DocumentationHit {
                            rank: index as u32 + 1,
                            provider_score: *score,
                            document_id: format!("{}{}", self.base, entry.href),
                            title,
                            signature: None,
                            snippet,
                            applicable_version: entry.version.clone(),
                        }
                    })
                    .collect();
                // Отмена посреди ранжирования — обрыв, а не ответ: пустые
                // фрагменты при отменённом вызове публиковать как Ok нельзя
                // (ADR-0032 п.10 — частичные результаты не публикуются).
                let status = if cancelled_mid_ranking {
                    DocumentationSectionStatus::Unavailable {
                        reason: UnavailableReason::Timeout,
                        detail: "вызов отменён посреди дочитывания страниц".to_string(),
                    }
                } else if hits.is_empty() {
                    DocumentationSectionStatus::Empty
                } else {
                    DocumentationSectionStatus::Ok
                };
                let hits = if cancelled_mid_ranking {
                    Vec::new()
                } else {
                    hits
                };
                DocumentationSection {
                    provider: DocumentationProviderId::new("kb-1ci"),
                    corpus: corpus_id.to_string(),
                    source_kind: SourceKind::PlatformHelp,
                    authority: Authority::Vendor,
                    // Дерево и страницы запрошены language=en — площадка
                    // англоязычна, и секция называет локаль, которой ответила.
                    language: "en".to_string(),
                    status,
                    warnings,
                    hits,
                }
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;

    /// Фейковый транспорт: канонические ответы по URL, счётчик обращений.
    pub(crate) struct FakeTransport {
        pub responses: BTreeMap<String, Result<String, String>>,
        pub calls: AtomicUsize,
    }

    impl KbTransport for FakeTransport {
        fn get(&self, url: &str) -> Result<String, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.responses
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(format!("незапланированный запрос: {url}")))
        }
    }

    pub(crate) fn tree_url(base: &str, id: &str) -> String {
        format!(
            "{base}/bin/get/OnecInt/Extensions/GuideNavigation/NavigationSource/WebHome?language=en&outputSyntax=plain&data=children&id={}",
            super::url_encode(id)
        )
    }

    pub(crate) fn node(id: &str, text: &str, children: bool) -> String {
        node_href(id, text, children, "/x/?language=en")
    }

    pub(crate) fn node_href(id: &str, text: &str, children: bool, href: &str) -> String {
        // На проводе обратный слэш живёт как `\\.`: живой ответ площадки —
        // валидный JSON, и декодированный id несёт один слэш. Фикстура
        // кодирует так же, иначе она была бы невалидным JSON, которого
        // площадка не отдаёт.
        let id = id.replace('\\', "\\\\");
        format!(
            r#"{{"id":"{id}","text":"{text}","children":{children},"a_attr":{{"title":"{text}","href":"{href}"}}}}"#
        )
    }

    pub(crate) const DEV27: &str = r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.WebHome";
    pub(crate) const DEV27_APPENDIX: &str = r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.Appendix_1\._URL_formats.WebHome";
    pub(crate) const DEV27_E1CIB: &str = r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.Appendix_1\._URL_formats.The_e1cib_URL_scheme.WebHome";
    pub(crate) const ADMIN27: &str = r"OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.1C_Enterprise_8\.3\.27_Administrator_Guide\._Client_Server_Mode.WebHome";

    /// Каноническая площадка целиком: дерево из зондов 2026-08-09 плюс главы
    /// руководства 8.3.27 и серверные страницы. Одна фикстура на все тесты
    /// поставщика, чтобы ответы не расходились между ними.
    pub(crate) fn standard_site(base: &str) -> BTreeMap<String, Result<String, String>> {
        let mut responses: BTreeMap<String, Result<String, String>> = BTreeMap::new();
        responses.insert(
            tree_url(base, "#"),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.WebHome",
                    "1C:Enterprise Platform",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(base, "OnecInt.KB.1C_Enterprise_Platform.WebHome"),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.WebHome",
                    "Guides",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(base, "OnecInt.KB.1C_Enterprise_Platform.Guides.WebHome"),
            Ok(format!(
                "[{},{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.WebHome",
                    "Developer Guides",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.WebHome",
                    "Administrator Guides",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.WebHome",
            ),
            Ok(format!(
                "[{},{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_8\.3\.25_Developer_Guide.WebHome",
                    "1C:Enterprise 8.3.25 Developer Guide",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Development_Standards.WebHome",
                    "1C:Enterprise Development Standards",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.WebHome",
                    "1C:Enterprise Developer Guide",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.WebHome",
            ),
            Ok(format!(
                "[{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.26_Developer_Guide.WebHome",
                    "1C:Enterprise 8.3.26 Developer Guide",
                    true
                ),
                node(DEV27, "1C:Enterprise 8.3.27 Developer Guide", true)
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.WebHome",
            ),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.WebHome",
                    "Administrator Guide.Client/Server Mode",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.WebHome",
            ),
            Ok(format!(
                "[{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.1C_Enterprise_8\.5\.1_Administrator_Guide\._Client_Server_Mode.WebHome",
                    "1C:Enterprise 8.5.1 Administrator Guide.Client/Server Mode",
                    true
                ),
                node(ADMIN27, "1C:Enterprise 8.3.27 Administrator Guide. Client/Server Mode", true)
            )),
        );
        // Главы руководства разработчика 8.3.27: приложение с детьми и глава
        // без них.
        responses.insert(
            tree_url(base, DEV27),
            Ok(format!(
                "[{},{}]",
                node_href(
                    DEV27_APPENDIX,
                    "Appendix 1. URL formats",
                    true,
                    "/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Appendix_1._URL_formats/?language=en"
                ),
                node_href(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.Chapter_2\._Managing_configurations.WebHome",
                    "Chapter 2. Managing configurations",
                    false,
                    "/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Chapter_2._Managing_configurations/?language=en"
                )
            )),
        );
        responses.insert(
            tree_url(base, DEV27_APPENDIX),
            Ok(format!(
                "[{}]",
                node_href(
                    DEV27_E1CIB,
                    "1. The e1cib URL scheme",
                    false,
                    "/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Appendix_1._URL_formats/The_e1cib_URL_scheme/?language=en"
                )
            )),
        );
        // Главы руководства администратора 8.3.27 (клиент-серверный режим).
        responses.insert(
            tree_url(base, ADMIN27),
            Ok(format!(
                "[{}]",
                node_href(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.1C_Enterprise_8\.3\.27_Administrator_Guide\._Client_Server_Mode.Chapter_1\._Cluster.WebHome",
                    "Chapter 1. Client/server cluster",
                    false,
                    "/1C_Enterprise_Platform/Guides/Administrator_Guides/Administrator_Guide_Client_Server_Mode/1C_Enterprise_8.3.27_Administrator_Guide._Client_Server_Mode/Chapter_1._Cluster/?language=en"
                )
            )),
        );
        // Серверные страницы совпадающих узлов.
        responses.insert(
            super::content_url(base, DEV27_APPENDIX).expect("адрес приложения"),
            Ok(r#"<div id="xwikicontent"><h1>Appendix 1. URL formats</h1><p>The e1cib scheme addresses forms and lists.</p></div>"#.to_string()),
        );
        responses.insert(
            super::content_url(base, DEV27_E1CIB).expect("адрес страницы e1cib"),
            Ok(r#"<div id="xwikicontent"><h1>1. The e1cib URL scheme</h1><p>Parameters vrn, stngs and cmdprm select the view.</p></div>"#.to_string()),
        );
        responses
    }
}

#[cfg(test)]
mod provider_tests {
    use super::tests_support::*;
    use super::*;
    use crate::domain::documentation::{
        DocumentationContext, DocumentationProvider, DocumentationSearchRequest,
        DocumentationSectionStatus, SourceKind, UnavailableReason,
    };
    use crate::infrastructure::documentation_policy::NetworkAccess;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn provider_over(
        base: &str,
        responses: std::collections::BTreeMap<String, Result<String, String>>,
        network: NetworkAccess,
    ) -> (Kb1ciProvider, Arc<FakeTransport>) {
        let transport = Arc::new(FakeTransport {
            responses,
            calls: AtomicUsize::new(0),
        });
        (
            Kb1ciProvider {
                base: base.to_string(),
                network,
                transport: Arc::clone(&transport) as Arc<dyn KbTransport>,
                cancellation: crate::domain::cancellation::CancellationToken::default(),
                cache_ttl: Duration::from_secs(3600),
                lexicon: Arc::new(InstallationLexiconSource),
            },
            transport,
        )
    }

    fn request(
        query: &str,
        version: Option<&str>,
    ) -> (DocumentationSearchRequest, DocumentationContext) {
        (
            DocumentationSearchRequest {
                query: query.to_string(),
                source_kinds: vec![SourceKind::PlatformHelp],
                limit: 20,
                language: "en".to_string(),
            },
            DocumentationContext {
                platform_version: version.map(str::to_string),
                installation_root: None,
            },
        )
    }

    fn corpus_section<'sections>(
        sections: &'sections [crate::domain::documentation::DocumentationSection],
        corpus: &str,
    ) -> &'sections crate::domain::documentation::DocumentationSection {
        sections
            .iter()
            .find(|section| section.corpus == corpus)
            .unwrap_or_else(|| panic!("секция {corpus} не найдена: {sections:?}"))
    }

    /// Поиск идёт по заголовкам двухуровневого оглавления выбранного
    /// руководства, а страницы дочитываются только для верхних совпадений:
    /// сетевой корпус нельзя читать целиком на каждый запрос. `document_id` —
    /// абсолютный pretty-адрес узла: устойчивый локатор, который открывает
    /// человек; фрагмент — текст серверно отрендеренной страницы.
    #[test]
    fn kb_provider_matches_titles_and_reads_only_the_top_pages() {
        let _serial = kb_test_lock();
        let base = "https://kb.match";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("URL", Some("8.3.27"));

        let sections = provider.search(&request, &context);
        assert_eq!(sections.len(), 2, "по секции на корпус: {sections:?}");
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "получено {:?}",
            developer.status
        );
        assert_eq!(
            developer.hits.len(),
            2,
            "приложение и его дочерняя страница: {:?}",
            developer.hits
        );
        assert_eq!(
            developer.hits[0].document_id,
            format!("{base}/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Appendix_1._URL_formats/?language=en"),
            "локатор — абсолютный pretty-адрес узла"
        );
        assert!(
            developer.hits[0]
                .snippet
                .contains("The e1cib scheme addresses forms"),
            "фрагмент — текст серверной страницы, получено {:?}",
            developer.hits[0].snippet
        );
        assert_eq!(developer.hits[0].applicable_version, "8.3.27");
        assert_eq!(developer.language, "en");

        let administrator = corpus_section(&sections, "kb-administrator-guide");
        assert!(
            matches!(administrator.status, DocumentationSectionStatus::Empty),
            "запрос URL не совпадает с главами администратора: {:?}",
            administrator.status
        );

        // Дочитаны ровно две страницы совпадений; всё остальное — дерево.
        let tree_calls = 7 /* корень…версии */ + 3 /* оглавления dev27, приложение, admin27 */;
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            tree_calls + 2,
            "страницы читаются только для совпадений"
        );
    }

    /// Ядро ADR-0036 в kb: слова запроса совпадают с заголовком узла
    /// пословно, а не целой подстрокой — переставленный порядок слов больше
    /// не прячет главу (#415).
    #[test]
    fn kb_an_english_query_matches_title_tokens_not_the_whole_substring() {
        let _serial = kb_test_lock();
        let base = "https://kb.tokens";
        let (provider, _transport) =
            provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("formats URL", Some("8.3.27"));
        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "переставленные слова обязаны найти главу: {:?}",
            developer.status
        );
        assert!(
            developer.hits[0].document_id.contains("URL_formats"),
            "{:?}",
            developer.hits
        );
    }

    /// Русский запрос к английским заголовкам площадки работает через
    /// лексикон установки (ADR-0036 п.4); без установки — честный Empty,
    /// а не подложное совпадение.
    #[test]
    fn kb_a_russian_query_expands_through_the_installation_lexicon() {
        struct FixedLexicon;
        impl KbLexiconSource for FixedLexicon {
            fn lexicon(
                &self,
                _context: &crate::domain::documentation::DocumentationContext,
            ) -> Option<
                std::sync::Arc<
                    crate::infrastructure::documentation_retrieval::BilingualLexicon,
                >,
            > {
                Some(std::sync::Arc::new(
                    crate::infrastructure::documentation_retrieval::BilingualLexicon::from_titles(
                        ["НавигационнаяСсылка.Форматы (URL.Formats)"],
                    ),
                ))
            }
        }
        let _serial = kb_test_lock();
        let base = "https://kb.lexicon";
        let (mut provider, _transport) =
            provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("форматы ссылок", Some("8.3.27"));

        let without = provider.search(&request, &context);
        assert!(
            matches!(
                corpus_section(&without, "kb-developer-guide").status,
                DocumentationSectionStatus::Empty
            ),
            "без лексикона русский запрос честно пуст: {:?}",
            corpus_section(&without, "kb-developer-guide").status
        );

        provider.lexicon = Arc::new(FixedLexicon);
        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "лексикон обязан довести русский запрос до английской главы: {:?}",
            developer.status
        );
        assert!(
            developer.hits[0].document_id.contains("URL_formats"),
            "{:?}",
            developer.hits
        );
    }

    /// Пин сборки отвечается руководством её семейства: площадка ведёт
    /// руководства по семействам, и «8.3.27.2074» — усечение до «8.3.27»,
    /// а не подстановка соседней версии. Усечение называется предупреждением
    /// секции, а попадание несёт версию площадки.
    #[test]
    fn kb_a_build_pin_is_answered_by_its_family_guide_with_a_disclosure() {
        let _serial = kb_test_lock();
        let base = "https://kb.build-pin";
        let (provider, _transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("URL", Some("8.3.27.2074"));

        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "семейство 8.3.27 обязано ответить на сборку 8.3.27.2074: {:?}",
            developer.status
        );
        assert_eq!(developer.hits[0].applicable_version, "8.3.27");
        assert!(
            developer
                .warnings
                .iter()
                .any(|warning| warning.contains("8.3.27.2074") && warning.contains("8.3.27")),
            "усечение сборки до семейства называется предупреждением: {:?}",
            developer.warnings
        );
    }

    /// Чужое семейство не подставляется и для пина сборки: усечение — не
    /// разрешение брать соседнее руководство.
    #[test]
    fn kb_a_build_pin_of_an_absent_family_is_still_refused() {
        let _serial = kb_test_lock();
        let base = "https://kb.build-foreign";
        let (provider, _transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("URL", Some("8.4.1.100"));

        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        match &developer.status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::VersionMissing);
                assert!(detail.contains("8.4.1.100"), "{detail}");
            }
            other => panic!("ожидался Unavailable{{VersionMissing}}, получено {other:?}"),
        }
    }

    /// Версии, которых нет, называются поимённо: «подстановка соседней версии
    /// запрещена и здесь» (проектная записка, «Отказы»), а отказ без перечня
    /// доступного заставил бы пользователя гадать.
    #[test]
    fn kb_provider_names_available_versions_when_the_requested_one_is_absent() {
        let _serial = kb_test_lock();
        let base = "https://kb.versions";
        let (provider, _transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request, context) = request("URL", Some("8.9"));

        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        match &developer.status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::VersionMissing);
                assert!(
                    detail.contains("8.3.27") && detail.contains("8.3.25"),
                    "отказ обязан перечислить доступные версии, получено {detail}"
                );
            }
            other => panic!("ожидался Unavailable{{VersionMissing}}, получено {other:?}"),
        }
    }

    #[test]
    fn kb_denied_by_policy_answers_policy_denied_without_network() {
        let _serial = kb_test_lock();
        let base = "https://kb.denied";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Deny);
        let (request, context) = request("URL", Some("8.3.27"));

        let sections = provider.search(&request, &context);
        assert_eq!(sections.len(), 1, "одна диагностичная секция: {sections:?}");
        match &sections[0].status {
            DocumentationSectionStatus::Unavailable { reason, detail } => {
                assert_eq!(*reason, UnavailableReason::PolicyDenied);
                assert!(
                    detail.contains("unica.toml"),
                    "отказ обязан назвать политику, получено {detail}"
                );
            }
            other => panic!("ожидался Unavailable{{PolicyDenied}}, получено {other:?}"),
        }
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            0,
            "запрещённый поставщик не должен трогать сеть"
        );
    }

    /// Оболочка вместо серверного рендера — «страницы нет», но совпадение по
    /// заголовку настоящее: попадание остаётся с пустым фрагментом, а пропажа
    /// называется предупреждением секции — тем же полем, что и частично
    /// неразобравшийся корпус установки.
    #[test]
    fn a_shell_page_becomes_a_warning_not_a_failure() {
        let _serial = kb_test_lock();
        let base = "https://kb.shell";
        let mut responses = standard_site(base);
        responses.insert(
            super::content_url(base, DEV27_APPENDIX).expect("адрес приложения"),
            Ok("<html><body><main>SPA shell</main></body></html>".to_string()),
        );
        let (provider, _transport) = provider_over(base, responses, NetworkAccess::Allow);
        let (request, context) = request("URL formats", Some("8.3.27"));

        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "настоящее совпадение остаётся, получено {:?}",
            developer.status
        );
        // Пословный матчинг вправе поднять и дочернюю страницу приложения —
        // проверяется попадание самой оболочки, а не единственность выдачи.
        let shell_hit = developer
            .hits
            .iter()
            .find(|hit| hit.document_id.contains("Appendix_1._URL_formats/?"))
            .expect("попадание оболочки остаётся в выдаче");
        assert!(
            shell_hit.snippet.is_empty(),
            "фрагмента нет — страница не отдана"
        );
        assert_eq!(
            developer.warnings.len(),
            1,
            "пропажа страницы обязана быть названа предупреждением"
        );
        assert!(
            developer.warnings[0].contains("Appendix_1._URL_formats")
                && developer.warnings[0].contains("страница не отдана"),
            "предупреждение обязано назвать отказавший адрес и причину, получено {}",
            developer.warnings[0]
        );
    }

    /// Кеши дерева стареют: площадка переезжала четырежды, и долгоживущий
    /// сервер обязан увидеть новую раскладку без рестарта. Просроченный кеш
    /// перечитывается — наблюдаемо новыми обращениями к дереву при нулевом
    /// сроке жизни.
    #[test]
    fn an_expired_tree_cache_is_refetched() {
        let _serial = kb_test_lock();
        let base = "https://kb.ttl";
        let (mut provider, transport) =
            provider_over(base, standard_site(base), NetworkAccess::Allow);
        provider.cache_ttl = Duration::ZERO;
        let (first, context) = request("нет-такого-заголовка", Some("8.3.27"));
        provider.search(&first, &context);
        let after_first = transport.calls.load(Ordering::SeqCst);

        let (second, context) = request("нет-такого-заголовка", Some("8.3.27"));
        provider.search(&second, &context);
        assert!(
            transport.calls.load(Ordering::SeqCst) > after_first,
            "просроченный кеш обязан перечитываться, обращений не прибавилось"
        );
    }

    /// Дерево выбранного руководства живёт в памяти процесса: второй запрос
    /// не перечитывает площадку. Запрос без совпадений не дочитывает страниц,
    /// поэтому ноль новых обращений — прямое наблюдение кеша.
    #[test]
    fn a_second_search_answers_the_tree_from_memory() {
        let _serial = kb_test_lock();
        let base = "https://kb.cache";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let (request_first, context) = request("URL", Some("8.3.27"));
        provider.search(&request_first, &context);
        let after_first = transport.calls.load(Ordering::SeqCst);

        let (request_second, context) = request("нет-такого-заголовка", Some("8.3.27"));
        let sections = provider.search(&request_second, &context);
        assert!(matches!(
            corpus_section(&sections, "kb-developer-guide").status,
            DocumentationSectionStatus::Empty
        ));
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            after_first,
            "второй запрос обязан отвечать из кеша дерева, без новых обращений"
        );
    }

    /// `get` открывает страницу по её pretty-локатору из попадания: путь
    /// локатора зеркалит сегменты серверного рендера, поэтому дерево не
    /// обходится вовсе — один запрос `/bin/view`. Граница объявленных корней
    /// действует и здесь: адрес вне категорий руководств не запрашивается.
    #[test]
    fn kb_get_opens_the_page_by_its_pretty_locator_without_walking_the_tree() {
        let _serial = kb_test_lock();
        let base = "https://kb.get";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let locator = format!(
            "{base}/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Appendix_1._URL_formats/?language=en"
        );
        let document = provider
            .get(&locator, "en", &request("x", Some("8.3.27")).1)
            .expect("локатор наш")
            .expect("страница открыта");
        assert_eq!(document.title, "Appendix 1. URL formats");
        assert!(
            document.text.contains("The e1cib scheme addresses forms"),
            "текст — серверная страница, получено {}",
            document.text
        );
        assert_eq!(document.corpus, "kb-developer-guide");
        assert_eq!(document.applicable_version, "8.3.27");
        assert_eq!(document.language, "en");
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            1,
            "get не обходит дерево — один запрос страницы"
        );

        assert!(
            provider
                .get("https://v8std.ru/std/702/", "en", &request("x", None).1)
                .is_none(),
            "чужая база — не мой локатор"
        );
        let outside = provider
            .get(
                &format!("{base}/Support/Billing/?language=en"),
                "en",
                &request("x", None).1,
            )
            .expect("база наша");
        let error = outside.expect_err("адрес вне корней руководств — отказ");
        assert!(
            error.contains("корн"),
            "отказ обязан назвать границу корней, получено {error}"
        );
    }

    /// Локатор приходит от вызывающего, и его путь уходит в адрес запроса:
    /// сегменты `..` выводили бы запрос за объявленные корни руководств —
    /// ровно та граница, которую ADR-0032 п.2 ставит вместо robots.txt.
    /// Путь намеренно несёт и категорию, и версию: проверки корпуса и версии
    /// такой обход проходит, и ловить его обязана именно проверка сегментов.
    #[test]
    fn kb_get_refuses_a_locator_with_parent_segments() {
        let _serial = kb_test_lock();
        let base = "https://kb.traversal";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Allow);
        let sneaky = format!(
            "{base}/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.27_Developer_Guide/../../../../../XWiki/Admin/?language=en"
        );
        let error = provider
            .get(&sneaky, "en", &request("x", None).1)
            .expect("база наша")
            .expect_err("сегменты .. обязаны отклоняться");
        assert!(
            error.contains(".."),
            "отказ обязан назвать сегмент, получено {error}"
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0, "сеть не тронута");
    }

    /// Транспорт, гасящий токен на первом же запросе страницы: так отмена
    /// приходит ПОСРЕДИ ранжирования, между дочитываниями, — за ранним
    /// гейтом `search`, который пре-отменённый токен ловит и без этого.
    struct CancellingTransport {
        inner: FakeTransport,
        token: crate::domain::cancellation::CancellationToken,
    }

    impl KbTransport for CancellingTransport {
        fn get(&self, url: &str) -> Result<String, String> {
            if url.contains("/bin/view/") {
                self.token.cancel();
            }
            self.inner.get(url)
        }
    }

    /// Отмена посреди ранжирования не публикует секцию как успех: пустые
    /// фрагменты при отменённом вызове — не ответ, а обрыв, и ADR-0032 п.10
    /// требует диагностичной секции без частичных результатов обхода.
    #[test]
    fn cancellation_during_ranking_yields_a_diagnostic_section_not_ok_with_gaps() {
        let _serial = kb_test_lock();
        let base = "https://kb.midcancel";
        let token = crate::domain::cancellation::CancellationToken::default();
        let transport = Arc::new(CancellingTransport {
            inner: FakeTransport {
                responses: standard_site(base),
                calls: AtomicUsize::new(0),
            },
            token: token.clone(),
        });
        let provider = Kb1ciProvider {
            base: base.to_string(),
            network: NetworkAccess::Allow,
            transport: transport as Arc<dyn KbTransport>,
            cancellation: token,
            cache_ttl: Duration::from_secs(3600),
            lexicon: Arc::new(InstallationLexiconSource),
        };
        // Запрос «URL» совпадает с двумя узлами: первое дочитывание гасит
        // токен, и второе уже не должно ни состояться, ни превратить секцию
        // в Ok с молчаливой дырой вместо фрагмента.
        let (request, context) = request("URL", Some("8.3.27"));
        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            !matches!(developer.status, DocumentationSectionStatus::Ok),
            "отменённый посреди ранжирования вызов не публикует Ok-секцию, получено {:?}",
            developer.status
        );
    }

    #[test]
    fn kb_get_denied_by_policy_refuses_without_network() {
        let _serial = kb_test_lock();
        let base = "https://kb.getdeny";
        let (provider, transport) = provider_over(base, standard_site(base), NetworkAccess::Deny);
        let denied = provider
            .get(
                &format!("{base}/1C_Enterprise_Platform/Guides/Developer_Guides/x/?language=en"),
                "en",
                &request("x", None).1,
            )
            .expect("локатор наш");
        let error = denied.expect_err("запрет политики — отказ владельца");
        assert!(
            error.contains("unica.toml"),
            "отказ обязан назвать политику, получено {error}"
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0, "сеть не тронута");
    }

    /// Живой прогон против площадки вендора: включается переменной
    /// `UNICA_KB1CI_LIVE=1`, в CI не требуется. Первый прогон дорогой —
    /// обход двухуровневого оглавления руководства с разнесением обращений;
    /// он же проверяет, что раскладка площадки не уехала в четвёртый раз.
    #[test]
    fn live_kb_answers_url_formats_from_the_developer_guide() {
        if std::env::var("UNICA_KB1CI_LIVE").is_err() {
            eprintln!("UNICA_KB1CI_LIVE не задан — живой прогон пропущен");
            return;
        }
        let _serial = kb_test_lock();
        let provider = Kb1ciProvider {
            base: KB_BASE.to_string(),
            network: NetworkAccess::Allow,
            transport: Arc::new(UreqKbTransport),
            cancellation: crate::domain::cancellation::CancellationToken::default(),
            cache_ttl: Duration::from_secs(3600),
            lexicon: Arc::new(InstallationLexiconSource),
        };
        let (request, context) = request("URL formats", Some("8.3.27"));
        let sections = provider.search(&request, &context);
        let developer = corpus_section(&sections, "kb-developer-guide");
        assert!(
            matches!(developer.status, DocumentationSectionStatus::Ok),
            "живая площадка обязана ответить приложением об адресах, получено {:?}",
            developer.status
        );
        let hit = developer.hits.first().expect("попадание найдено");
        assert!(
            hit.document_id.starts_with("https://kb.1ci.com/"),
            "локатор — адрес площадки, получено {}",
            hit.document_id
        );
        assert_eq!(hit.applicable_version, "8.3.27");
        eprintln!(
            "live kb-1ci: {} совпадений, первое — {} ({} символов фрагмента)",
            developer.hits.len(),
            hit.document_id,
            hit.snippet.chars().count()
        );

        // Вторая нога: попадание — оглавление приложения, и get честно
        // называет его оглавлением, а не отдаёт пустоту за документ.
        let toc = provider
            .get(&hit.document_id, "en", &context)
            .expect("локатор площадки наш")
            .expect_err("оглавление не выдаётся за документ");
        assert!(
            toc.contains("оглавлени"),
            "отказ обязан назвать оглавление, получено {toc}"
        );

        // Третья нога: настоящий лист руководства открывается целиком
        // (ADR-0033) и несёт текст про e1cib — вопрос #296 живьём.
        let leaf = format!(
            "{KB_BASE}/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Developer_Guide/1C_Enterprise_8.3.27_Developer_Guide/Appendix_1._URL_formats/1.2._Internal_links/1.2.1._General_information/?language=en"
        );
        let document = provider
            .get(&leaf, "en", &context)
            .expect("локатор площадки наш")
            .expect("лист открыт целиком");
        assert!(
            document.text.contains("e1cib"),
            "текст листа обязан нести e1cib, получено {} символов",
            document.text.chars().count()
        );
        assert!(
            !document.title.is_empty(),
            "документ обязан нести заголовок страницы"
        );
        eprintln!(
            "live kb-1ci get: {} — {} символов полного текста",
            document.title,
            document.text.chars().count()
        );
    }

    /// Отмена приоритетна и частичный результат не публикуется: до первого же
    /// сетевого запроса отменённый вызов отвечает диагностичной секцией.
    #[test]
    fn cancellation_stops_before_the_next_network_request() {
        let _serial = kb_test_lock();
        let base = "https://kb.cancel";
        let (mut provider, transport) =
            provider_over(base, standard_site(base), NetworkAccess::Allow);
        let cancellation = crate::domain::cancellation::CancellationToken::default();
        cancellation.cancel();
        provider.cancellation = cancellation;
        let (request, context) = request("URL", Some("8.3.27"));

        let sections = provider.search(&request, &context);
        assert_eq!(sections.len(), 1, "одна диагностичная секция: {sections:?}");
        assert!(
            matches!(
                sections[0].status,
                DocumentationSectionStatus::Unavailable { .. }
            ),
            "получено {:?}",
            sections[0].status
        );
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            0,
            "отменённый вызов не должен трогать сеть"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn content_segments_unescapes_dots_inside_a_segment() {
        assert_eq!(
            content_segments(
                r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.Appendix_1\._URL_formats.WebHome"
            ),
            Some(vec![
                "1C_Enterprise_Platform".to_string(),
                "Guides".to_string(),
                "Developer_Guides".to_string(),
                "1C_Enterprise_Developer_Guide".to_string(),
                "1C_Enterprise_8.3.27_Developer_Guide".to_string(),
                "Appendix_1._URL_formats".to_string(),
            ]),
            "экранированная точка — точка внутри сегмента, а не разделитель"
        );
        assert_eq!(
            content_segments("Alien.Space.WebHome"),
            None,
            "id вне OnecInt.KB — чужой, адрес из него не строится"
        );
    }

    #[test]
    fn content_url_builds_the_server_rendered_view() {
        assert_eq!(
            content_url(
                "https://kb.1ci.com",
                r"OnecInt.KB.1C_Enterprise_Platform.Guides.WebHome"
            )
            .as_deref(),
            Some(
                "https://kb.1ci.com/bin/view/OnecInt/KB/1C_Enterprise_Platform/Guides/?language=en"
            ),
            "контент отдаёт серверный рендер /bin/view, а не pretty-href SPA"
        );
    }

    #[test]
    fn read_page_takes_h1_and_refuses_a_shell_without_xwikicontent() {
        let page = read_page(
            r#"<html><body><div id="xwikicontent"><h1>Appendix 1. URL formats</h1><p>The e1cib scheme <b>opens</b> forms.</p></div></body></html>"#,
        )
        .expect("настоящая страница читается");
        assert_eq!(page.title, "Appendix 1. URL formats");
        assert!(
            page.text.contains("The e1cib scheme opens forms."),
            "текст без разметки, получено {}",
            page.text
        );

        let shell = read_page("<html><body><main>SPA shell</main></body></html>");
        assert!(
            shell.is_err(),
            "оболочка без xwikicontent — отсутствие страницы, а не пустой текст"
        );
    }

    /// Живая раскладка страницы (зонд 2026-08-09, четвёртый переезд
    /// площадки): `<h1>` заголовка лежит ВНЕ контентного div, в шапке
    /// страницы; текст — между маркером `xwikicontent` и `</main>`, а после
    /// `</main>` идёт футер с иконками, который не должен попадать в текст.
    /// Страница-оглавление несёт пустой контентный div — это не отказ чтения,
    /// а пустой текст, который вызывающий толкует сам.
    #[test]
    fn read_page_takes_the_header_h1_and_stops_the_text_at_main_end() {
        let page = read_page(
            r#"<html><body><h1 id="document-title">1.2.1. General information</h1><main><div id="xwikicontent" class="col-xs-12"><p>Common internal link format: e1cib.</p></div></main><footer>Search Icon/Social/001 Icon/Social/006</footer></body></html>"#,
        )
        .expect("настоящая страница читается");
        assert_eq!(
            page.title, "1.2.1. General information",
            "заголовок — h1 шапки страницы, он лежит вне контентного div"
        );
        assert!(
            page.text.contains("Common internal link format: e1cib."),
            "текст — содержимое контентной области, получено {}",
            page.text
        );
        assert!(
            !page.text.contains("Icon/Social"),
            "футер после </main> не должен попадать в текст, получено {}",
            page.text
        );

        let toc = read_page(
            r#"<h1>Appendix 1. URL formats</h1><main><div id="xwikicontent" class="col-xs-12"> </div></main><footer>chrome</footer>"#,
        )
        .expect("оглавление читается");
        assert_eq!(toc.title, "Appendix 1. URL formats");
        assert!(
            toc.text.trim().is_empty(),
            "оглавление несёт пустой текст, а не футер, получено {}",
            toc.text
        );
    }

    /// Раскладка версий руководства разработчика двухуровневая (зонд
    /// 2026-08-09): старые версии — прямые дети категории, новые — дети
    /// контейнера «1C:Enterprise Developer Guide» без версии в имени. Обход,
    /// читающий только один уровень, потерял бы либо 8.3.27, либо 8.3.25.
    #[test]
    fn discover_guides_collects_versions_from_both_developer_layouts() {
        let base = "https://kb.test";
        let mut responses: BTreeMap<String, Result<String, String>> = BTreeMap::new();
        responses.insert(
            tree_url(base, "#"),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.WebHome",
                    "1C:Enterprise Platform",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(base, "OnecInt.KB.1C_Enterprise_Platform.WebHome"),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.WebHome",
                    "Guides",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(base, "OnecInt.KB.1C_Enterprise_Platform.Guides.WebHome"),
            Ok(format!(
                "[{},{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.WebHome",
                    "Developer Guides",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.WebHome",
                    "Administrator Guides",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.WebHome",
            ),
            Ok(format!(
                "[{},{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_8\.3\.25_Developer_Guide.WebHome",
                    "1C:Enterprise 8.3.25 Developer Guide",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Development_Standards.WebHome",
                    "1C:Enterprise Development Standards",
                    true
                ),
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.WebHome",
                    "1C:Enterprise Developer Guide",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.WebHome",
            ),
            Ok(format!(
                "[{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.26_Developer_Guide.WebHome",
                    "1C:Enterprise 8.3.26 Developer Guide",
                    true
                ),
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Developer_Guides.1C_Enterprise_Developer_Guide.1C_Enterprise_8\.3\.27_Developer_Guide.WebHome",
                    "1C:Enterprise 8.3.27 Developer Guide",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.WebHome",
            ),
            Ok(format!(
                "[{}]",
                node(
                    "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.WebHome",
                    "Administrator Guide.Client/Server Mode",
                    true
                )
            )),
        );
        responses.insert(
            tree_url(
                base,
                "OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.WebHome",
            ),
            Ok(format!(
                "[{},{}]",
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.1C_Enterprise_8\.5\.1_Administrator_Guide\._Client_Server_Mode.WebHome",
                    "1C:Enterprise 8.5.1 Administrator Guide.Client/Server Mode",
                    true
                ),
                node(
                    r"OnecInt.KB.1C_Enterprise_Platform.Guides.Administrator_Guides.Administrator_Guide_Client_Server_Mode.1C_Enterprise_8\.3\.27_Administrator_Guide\._Client_Server_Mode.WebHome",
                    "1C:Enterprise 8.3.27 Administrator Guide. Client/Server Mode",
                    true
                )
            )),
        );
        let transport = FakeTransport {
            responses,
            calls: AtomicUsize::new(0),
        };

        let catalog = discover_guides(&transport, base).expect("каталог руководств");

        let developer: Vec<(&str, Option<&str>)> = catalog
            .developer
            .iter()
            .map(|guide| (guide.version.as_str(), guide.mode.as_deref()))
            .collect();
        assert!(
            developer.contains(&("8.3.25", None)),
            "версия с первого уровня раскладки потеряна: {developer:?}"
        );
        assert!(
            developer.contains(&("8.3.27", None)),
            "версия из контейнера второго уровня потеряна: {developer:?}"
        );
        assert!(
            !catalog
                .developer
                .iter()
                .any(|guide| guide.node_id.contains("Development_Standards")),
            "категория без версии в имени — не руководство разработчика"
        );

        let administrator: Vec<(&str, Option<&str>)> = catalog
            .administrator
            .iter()
            .map(|guide| (guide.version.as_str(), guide.mode.as_deref()))
            .collect();
        assert!(
            administrator.contains(&("8.5.1", Some("Administrator Guide.Client/Server Mode"))),
            "режим администратора обязан прийти из дерева: {administrator:?}"
        );
        assert!(
            administrator.contains(&("8.3.27", Some("Administrator Guide.Client/Server Mode"))),
            "все версии режима собираются: {administrator:?}"
        );
    }
}
