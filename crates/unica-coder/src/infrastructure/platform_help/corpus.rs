use std::io::Read;

use super::container::{ContainerError, V8Container};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub ru: Option<String>,
    pub en: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CorpusPage {
    pub path: String,
    pub title: String,
    pub text: String,
    pub signature: Option<Signature>,
}

#[derive(Debug)]
pub enum CorpusError {
    Container(ContainerError),
    MissingFileStorage,
    BadArchive,
}

/// Признак страницы: разметка в начале записи. Отбирать по расширению нельзя —
/// в контейнерах подсистем страницы лежат без расширения, рядом с картинками.
fn looks_like_markup(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(200)];
    head.contains(&b'<')
}

/// Заголовок страницы — содержимое первого `<h1>`. Измерено на установке
/// 8.3.27: у Синтакс-помощника это `<h1 class="V8SH_pagetitle">`, у контейнеров
/// подсистем — `<H1>` в верхнем регистре, поэтому сопоставление
/// регистронезависимо. Заголовок есть у 385 из 401 страницы `1cv8_ru.hbk` и у
/// 146 из 151 в `mngbase_ru.hbk`; для остальных вызывающий берёт начало текста.
///
/// Брать «первую фразу» текста нельзя: на реальной странице до первого «. »
/// укладываются заголовок, владелец, имя члена и строка «Доступен, начиная с
/// версии 8.2» — сто тридцать символов вместо заголовка.
fn page_title(raw: &str) -> Option<String> {
    let lower = raw.to_lowercase();
    let open = lower.find("<h1")?;
    let content_start = open + lower[open..].find('>')? + 1;
    let close = lower[content_start..].find("</h1")? + content_start;
    let inner = strip_markup(&raw[content_start..close]);
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

fn strip_markup(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
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

/// Сигнатура из `.st`: запись локали имеет вид {"ru",0,0,"","Имя()"} —
/// последнее поле записи и есть сигнатура. Границей записи служит `}`: без неё
/// поиск ушёл бы в запись следующей локали и вернул её значение.
fn parse_signature(raw: &str) -> Option<Signature> {
    let mut signature = Signature { ru: None, en: None };
    for locale in ["ru", "en"] {
        let needle = format!("\"{locale}\",");
        let Some(start) = raw.find(&needle) else {
            continue;
        };
        let tail = &raw[start + needle.len()..];
        let record = tail.split('}').next().unwrap_or(tail);
        let Some(open) = record.rfind('"') else {
            continue;
        };
        let before = &record[..open];
        let Some(value_start) = before.rfind('"') else {
            continue;
        };
        let value = &before[value_start + 1..];
        if value.is_empty() {
            continue;
        }
        match locale {
            "ru" => signature.ru = Some(value.to_string()),
            _ => signature.en = Some(value.to_string()),
        }
    }
    if signature.ru.is_none() && signature.en.is_none() {
        None
    } else {
        Some(signature)
    }
}

pub fn read_corpus(bytes: &[u8]) -> Result<Vec<CorpusPage>, CorpusError> {
    let container = V8Container::parse(bytes).map_err(CorpusError::Container)?;
    let storage = container
        .entry("FileStorage")
        .ok_or(CorpusError::MissingFileStorage)?;
    read_corpus_from_archive(storage)
}

pub fn read_corpus_from_archive(archive: &[u8]) -> Result<Vec<CorpusPage>, CorpusError> {
    let cursor = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|_| CorpusError::BadArchive)?;
    let mut bodies: Vec<(String, Vec<u8>)> = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|_| CorpusError::BadArchive)?;
        let mut body = Vec::new();
        file.read_to_end(&mut body)
            .map_err(|_| CorpusError::BadArchive)?;
        bodies.push((file.name().to_string(), body));
    }
    let signatures: std::collections::BTreeMap<String, Signature> = bodies
        .iter()
        .filter(|(name, _)| name.ends_with(".st"))
        .filter_map(|(name, body)| {
            let raw = String::from_utf8_lossy(body);
            parse_signature(&raw).map(|s| (name.trim_end_matches(".st").to_string(), s))
        })
        .collect();
    let mut pages = Vec::new();
    for (name, body) in &bodies {
        if name.ends_with(".st") || !looks_like_markup(body) {
            continue;
        }
        let raw = String::from_utf8_lossy(body);
        let text = strip_markup(&raw);
        let title = page_title(&raw).unwrap_or_else(|| text.chars().take(120).collect());
        let stem = name
            .rsplit_once('.')
            .map(|(head, _)| head)
            .unwrap_or(name.as_str());
        pages.push(CorpusPage {
            path: name.clone(),
            title,
            text,
            signature: signatures.get(stem).cloned(),
        });
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                writer.start_file(*name, options).expect("запись открыта");
                writer.write_all(data).expect("запись записана");
            }
            writer.finish().expect("архив закрыт");
        }
        buffer.into_inner()
    }

    /// Сигнатура с угловой скобкой в параметре — не редкость, а норма: на
    /// установке 8.3.27 такие `<` в первых 200 байтах имеют 5518 файлов `.st`
    /// из 22751, то есть 24,3%. Поэтому `.st` исключается по расширению, а не
    /// по признаку разметки: без этого четверть сигнатур стала бы «страницами».
    const ST: &str = r#"{1,
{2,
{"",1,0,"",""},
{0,
{"ru",0,0,"","ПолучитьНавигационнуюСсылку(<Объект>)"}
},
{0,
{"en",0,0,"","GetURL(<Object>)"}
}
}
}"#;

    const PAGE: &str = "<html><body><h1 class=\"V8SH_pagetitle\">Глобальный контекст.ПолучитьНавигационнуюСсылку (Global context.GetURL)</h1><p>Доступен, начиная с версии 8.2. Синтаксис: ПолучитьНавигационнуюСсылку()</p></body></html>";

    #[test]
    fn reads_html_page_with_bilingual_signature() {
        let archive = zip_with(&[
            (
                "objects/Global context/methods/GetURL3758.html",
                PAGE.as_bytes(),
            ),
            (
                "objects/Global context/methods/GetURL3758.st",
                ST.as_bytes(),
            ),
        ]);
        let pages = read_corpus_from_archive(&archive).expect("корпус прочитан");
        // Сигнатура содержит `<Объект>`, поэтому признак разметки её не отсеет:
        // отсеивает именно расширение. Уберите проверку `.st` — станет две
        // страницы, и тест упадёт.
        assert_eq!(pages.len(), 1, "файл .st страницей не считается");
        let page = &pages[0];
        assert_eq!(page.path, "objects/Global context/methods/GetURL3758.html");
        // Точное равенство: заголовок обязан быть содержимым <h1>, а не началом
        // текста страницы. Первая фраза текста дотянулась бы до «версии 8.2».
        assert_eq!(
            page.title,
            "Глобальный контекст.ПолучитьНавигационнуюСсылку (Global context.GetURL)"
        );
        assert!(page.text.contains("Синтаксис"));
        let signature = page.signature.as_ref().expect("сигнатура найдена");
        assert_eq!(
            signature.ru.as_deref(),
            Some("ПолучитьНавигационнуюСсылку(<Объект>)")
        );
        assert_eq!(signature.en.as_deref(), Some("GetURL(<Object>)"));
    }

    #[test]
    fn reads_extensionless_page_by_markup() {
        let archive = zip_with(&[
            // Верхний регистр тегов — как в реальных контейнерах подсистем.
            (
                "WebServices",
                "<HTML><BODY><H1>Web-сервисы</H1></BODY></HTML>".as_bytes(),
            ),
            ("navIcon", &[0x89, 0x50, 0x4e, 0x47]),
        ]);
        let pages = read_corpus_from_archive(&archive).expect("корпус прочитан");
        assert_eq!(pages.len(), 1, "изображение страницей не считается");
        assert_eq!(pages[0].path, "WebServices");
        assert_eq!(pages[0].title, "Web-сервисы");
        assert!(pages[0].signature.is_none());
    }

    #[test]
    fn missing_file_storage_is_reported() {
        let bytes = crate::infrastructure::platform_help::container::tests_support::container_without_file_storage();
        assert!(matches!(
            read_corpus(&bytes),
            Err(CorpusError::MissingFileStorage)
        ));
    }
}
