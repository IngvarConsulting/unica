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
///
/// Приведение — именно ASCII: смещения ищутся в приведённой копии, а режется
/// ими оригинал, поэтому копия обязана совпадать с оригиналом побайтово по
/// длине. `to_lowercase()` этого не гарантирует (U+0130 растёт на байт), и
/// срез оригинала попадает внутрь символа — паника уносит разбор всего
/// корпуса. Все искомые подстроки (`<h1`, `>`, `</h1`) состоят из ASCII, так
/// что `to_ascii_lowercase()` находит ровно то же самое.
fn page_title(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
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

/// Точечное чтение ОДНОЙ страницы контейнера по её пути — имени записи ZIP
/// внутри `FileStorage`. Разбирается только таблица записей контейнера и
/// инфлейтится одна запись (плюс соседняя `<stem>.st` ради сигнатуры), а не
/// все страницы корпуса: `unica.documentation.get` этим отличается от
/// индексирования, и полный `read_corpus` стоил ему 4.4 секунды на вызов.
/// Битая СОСЕДНЯЯ запись точечному чтению не мешает: её поток не читается.
/// `Ok(None)` — «в этом контейнере такой страницы нет».
pub fn read_page_from_container(
    bytes: &[u8],
    path: &str,
) -> Result<Option<CorpusPage>, CorpusError> {
    let container = V8Container::parse(bytes).map_err(CorpusError::Container)?;
    let storage = container
        .entry("FileStorage")
        .ok_or(CorpusError::MissingFileStorage)?;
    let cursor = std::io::Cursor::new(storage);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|_| CorpusError::BadArchive)?;
    let mut body = Vec::new();
    match zip.by_name(path) {
        Ok(mut file) => {
            file.read_to_end(&mut body)
                .map_err(|_| CorpusError::BadArchive)?;
        }
        Err(_) => return Ok(None),
    }
    if !looks_like_markup(&body) {
        return Ok(None);
    }
    let raw = String::from_utf8_lossy(&body);
    let text = strip_markup(&raw);
    let title = page_title(&raw).unwrap_or_else(|| text.chars().take(120).collect());
    // Сигнатура — из соседней записи `<stem>.st`, как и при полном разборе.
    // Её отсутствие или порча — не отказ страницы: сигнатуры есть не у всех.
    let stem = path.rsplit_once('.').map(|(head, _)| head).unwrap_or(path);
    let signature = zip
        .by_name(&format!("{stem}.st"))
        .ok()
        .and_then(|mut sibling| {
            let mut raw = Vec::new();
            sibling.read_to_end(&mut raw).ok()?;
            parse_signature(&String::from_utf8_lossy(&raw))
        });
    Ok(Some(CorpusPage {
        path: path.to_string(),
        title,
        text,
        signature,
    }))
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

    /// `to_lowercase()` не сохраняет длину в байтах: `İ` (U+0130) в нижнем
    /// регистре занимает на байт больше. Смещения, найденные в приведённой
    /// копии, режут тогда оригинал не по границе символа, и `page_title`
    /// паникует — унося разбор ВСЕГО корпуса, а не одной страницы. Здесь
    /// шесть таких символов в заголовке сдвигают закрывающий `</h1` на шесть
    /// байт, и срез оригинала попадает внутрь первой кириллической буквы.
    #[test]
    fn page_title_survives_a_case_fold_that_changes_byte_length() {
        let archive = zip_with(&[(
            "objects/Turkish.html",
            "<h1>İİİİİİ</h1>Тексты страницы".as_bytes(),
        )]);
        let pages = read_corpus_from_archive(&archive).expect("корпус прочитан");
        assert_eq!(pages.len(), 1, "страница обязана дожить до результата");
        assert_eq!(
            pages[0].title, "İİİİİİ",
            "заголовок обязан быть содержимым <h1>, а разбор — не паниковать"
        );
    }

    /// Заголовка нет у 16 из 401 страницы `1cv8_ru.hbk` и у 5 из 151 в
    /// `mngbase_ru.hbk` — это измеренный путь, а не теоретический. Запасной
    /// заголовок берётся из первых 120 символов ТЕКСТА (разметка снята), а не
    /// из первых 120 байт исходника: иначе в заголовок попали бы теги.
    #[test]
    fn page_without_h1_falls_back_to_the_beginning_of_its_text() {
        let body = "б".repeat(200);
        let archive = zip_with(&[(
            "objects/NoTitle",
            format!("<HTML><BODY><p>{body}</p></BODY></HTML>").as_bytes(),
        )]);
        let pages = read_corpus_from_archive(&archive).expect("корпус прочитан");
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0].title,
            "б".repeat(120),
            "запасной заголовок — первые 120 символов текста без разметки"
        );
    }

    /// Портит deflate-поток ИМЕННО ВТОРОЙ записи архива: находит второй
    /// локальный заголовок `PK\x03\x04` и переписывает байты сжатых данных
    /// за ним. Первая запись и центральный каталог не затронуты.
    fn corrupt_second_entry(archive: &mut [u8]) {
        let signature = [0x50, 0x4B, 0x03, 0x04];
        let mut headers = Vec::new();
        for start in 0..archive.len().saturating_sub(4) {
            if archive[start..start + 4] == signature {
                headers.push(start);
            }
        }
        assert!(headers.len() >= 2, "фикстуре нужны две записи");
        let second = headers[1];
        // Смещение до сжатых данных: 30 байт заголовка + имя + extra.
        let name_len = u16::from_le_bytes([archive[second + 26], archive[second + 27]]) as usize;
        let extra_len = u16::from_le_bytes([archive[second + 28], archive[second + 29]]) as usize;
        let data = second + 30 + name_len + extra_len;
        for byte in archive[data..data + 8].iter_mut() {
            *byte = 0xFF;
        }
    }

    /// Точечное чтение открывает только запрошенную запись: битый
    /// deflate-поток СОСЕДНЕЙ страницы ему не мешает, тогда как полный
    /// `read_corpus` на том же архиве отказывает целиком. Это и есть
    /// разница между «прочитать одну страницу» и «разобрать корпус» —
    /// и по устойчивости, и по цене (4.4 с на вызов до правки).
    #[test]
    fn a_single_page_is_read_even_next_to_a_corrupted_sibling_entry() {
        let mut archive = zip_with(&[
            (
                "objects/Good.html",
                "<html><body><h1>Хорошая страница</h1><p>Текст хорошей страницы.</p></body></html>"
                    .as_bytes(),
            ),
            (
                "objects/Broken.html",
                "<html><body><h1>Сосед</h1><p>Этот поток будет испорчен.</p></body></html>"
                    .as_bytes(),
            ),
        ]);
        corrupt_second_entry(&mut archive);
        let bytes = crate::infrastructure::platform_help::container::tests_support::container_with(
            &[("FileStorage", archive.as_slice())],
            None,
        );

        assert!(
            read_corpus(&bytes).is_err(),
            "полный разбор обязан споткнуться о битую запись — иначе порча не настоящая"
        );

        let page = read_page_from_container(&bytes, "objects/Good.html")
            .expect("точечное чтение не должно спотыкаться о соседа")
            .expect("страница найдена");
        assert_eq!(page.title, "Хорошая страница");
        assert!(page.text.contains("Текст хорошей страницы."));

        assert!(
            read_page_from_container(&bytes, "objects/Missing.html")
                .expect("отсутствие страницы — не ошибка контейнера")
                .is_none(),
            "чужого пути в контейнере нет"
        );
    }

    /// Сигнатура при точечном чтении приходит из соседней записи `.st` —
    /// как и при полном разборе.
    #[test]
    fn a_single_page_read_carries_its_signature() {
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
        let bytes = crate::infrastructure::platform_help::container::tests_support::container_with(
            &[("FileStorage", archive.as_slice())],
            None,
        );
        let page =
            read_page_from_container(&bytes, "objects/Global context/methods/GetURL3758.html")
                .expect("контейнер читается")
                .expect("страница найдена");
        let signature = page.signature.expect("сигнатура найдена");
        assert_eq!(
            signature.ru.as_deref(),
            Some("ПолучитьНавигационнуюСсылку(<Объект>)")
        );
        assert_eq!(signature.en.as_deref(), Some("GetURL(<Object>)"));
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
