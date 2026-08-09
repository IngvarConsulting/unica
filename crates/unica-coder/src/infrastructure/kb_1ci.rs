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
        {
            let mut last = LAST_REQUEST
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(previous) = *last {
                let elapsed = previous.elapsed();
                if elapsed < REQUEST_SPACING {
                    std::thread::sleep(REQUEST_SPACING - elapsed);
                }
            }
            *last = Some(Instant::now());
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
    pub title: String,
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

/// Читает серверно отрендеренную страницу: заголовок из первого `<h1>`,
/// текст без разметки от начала контентной области. Оболочка без
/// `xwikicontent` — то самое «перенаправление на корень считается
/// отсутствием страницы», а не пустой текст.
pub fn read_page(html: &str) -> Result<KbPage, String> {
    let lower = html.to_ascii_lowercase();
    let Some(content_start) = lower.find("id=\"xwikicontent\"") else {
        return Err(
            "страница не отдана: серверного рендера нет — оболочка или перенаправление на корень"
                .to_string(),
        );
    };
    let content = &html[content_start..];
    let content_lower = &lower[content_start..];
    let title = content_lower.find("<h1").and_then(|open| {
        let after_open = open + content_lower[open..].find('>')? + 1;
        let close = content_lower[after_open..].find("</h1")? + after_open;
        let inner = strip_markup_skipping_scripts(&content[after_open..close]);
        (!inner.is_empty()).then_some(inner)
    });
    let text = strip_markup_skipping_scripts(content);
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
                        title: child.title.clone(),
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
                                title: grandchild.title.clone(),
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
                        title: child.title.clone(),
                        node_id: child.id.clone(),
                        mode: Some(mode.title.clone()),
                    });
                }
            }
        }
    }
    Ok(catalog)
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
        // На проводе обратный слэш живёт как `\\.`: живой ответ площадки —
        // валидный JSON, и декодированный id несёт один слэш. Фикстура
        // кодирует так же, иначе она была бы невалидным JSON, которого
        // площадка не отдаёт.
        let id = id.replace('\\', "\\\\");
        format!(
            r#"{{"id":"{id}","text":"{text}","children":{children},"a_attr":{{"title":"{text}","href":"/x/?language=en"}}}}"#
        )
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
                .any(|guide| guide.title.contains("Development Standards")),
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
