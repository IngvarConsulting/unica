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
        let title = text.split(". ").next().unwrap_or(&text).trim().to_string();
        let stem = name
            .rsplit_once('.')
            .map(|(head, _)| head)
            .unwrap_or(name.as_str());
        pages.push(CorpusPage {
            path: name.clone(),
            title: if title.is_empty() {
                name.clone()
            } else {
                title
            },
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

    // `br#"..."#` (сырой *байтовый* строковый литерал) не допускает не-ASCII
    // символы в исходном тексте, а кириллическое значение локали ниже как раз
    // такой символ содержит; `r#"..."#.as_bytes()` даёт тот же набор байт без
    // этого ограничения — так же, как `PAGE` ниже получает `&[u8]` из
    // литерала с кириллицей.
    const ST: &[u8] = r#"{1,
{2,
{"",1,0,"",""},
{0,
{"ru",0,0,"","ПолучитьНавигационнуюСсылку()"}
},
{0,
{"en",0,0,"","GetURL()"}
}
}
}"#
    .as_bytes();

    const PAGE: &[u8] = "<html><body><p>Глобальный контекст.ПолучитьНавигационнуюСсылку (Global context.GetURL)</p><p>Синтаксис: ПолучитьНавигационнуюСсылку()</p></body></html>".as_bytes();

    #[test]
    fn reads_html_page_with_bilingual_signature() {
        let archive = zip_with(&[
            ("objects/Global context/methods/GetURL3758.html", PAGE),
            ("objects/Global context/methods/GetURL3758.st", ST),
        ]);
        let pages = read_corpus_from_archive(&archive).expect("корпус прочитан");
        assert_eq!(pages.len(), 1, "файл .st страницей не считается");
        let page = &pages[0];
        assert_eq!(page.path, "objects/Global context/methods/GetURL3758.html");
        assert!(page
            .title
            .starts_with("Глобальный контекст.ПолучитьНавигационнуюСсылку"));
        assert!(page.text.contains("Синтаксис"));
        let signature = page.signature.as_ref().expect("сигнатура найдена");
        assert_eq!(
            signature.ru.as_deref(),
            Some("ПолучитьНавигационнуюСсылку()")
        );
        assert_eq!(signature.en.as_deref(), Some("GetURL()"));
    }

    #[test]
    fn reads_extensionless_page_by_markup() {
        let archive = zip_with(&[
            (
                "WebServices",
                "<html><body><p>Web-сервисы</p></body></html>".as_bytes(),
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
