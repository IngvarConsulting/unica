# Platform Help From Installation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Дать `platform-help` работающий источник — контейнеры справки установленной платформы — через новый публичный инструмент `unica.documentation.search`, и снять загрузчик и корпус `docs-local/1ci` вместе с закрепляющими их проверками.

**Architecture:** Домен получает нейтральный к источнику `DocumentationProvider`, реестр и канонический результат секциями — по образцу `CodeIntelligenceProvider` из ADR-0017, но отдельным контрактом. Инфраструктура читает контейнеры v8 (`*.hbk`) установки: разбирает таблицу записей, достаёт `FileStorage` как ZIP, берёт сигнатуру из `.st` и текст из страницы. Карта имён строится лениво один раз на процесс на версию платформы и живёт в памяти; на диск не пишется ничего. Слой application запускает поставщиков, собирает секции и отдаёт типизированный `data`.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `zip`, существующие `WorkspaceContext` и разрешение платформы, Python-тесты контрактов из `tests/ci/`, ведомость поверхности `spec/architecture/tool-surface-review.json`.

## Global Constraints

- Утверждённый контракт — `docs/design/2026-08-08-platform-help-documentation-provider-design.md` и ADR-0029 (`spec/decisions/0029-dva-istochnika-spravki-platformy.md`). Если реализация вскроет выбор публичного контракта, которого там нет, остановитесь и поправьте записку и ADR, а не изобретайте второй контракт в коде.
- Этот план — первый из двух. Здесь только локальный источник. Сетевые поставщики (`v8std` за общим контрактом, фасады `unica.standards.*`, `kb-1ci`) и файл `unica.toml` — второй план. `unica.standards.*` в этом заходе не трогаются вовсе.
- Публичная поверхность прирастает ровно одним именем: `unica.documentation.search`. Псевдонимов нет, `unica.documentation.get` не вводится.
- Результат инструмента — типизированный `data` по ADR-0023. Проза в `data` не кладётся.
- Секции упорядочены реестром. Ранги и оценки локальны для секции: между секциями ничего не сливается, не пересортировывается и не удаляется.
- Материалы вендора не копируются в репозиторий, не коммитятся, не кладутся в пакет и не кэшируются на диск. Ни один тест не требует наличия установки платформы; проверки против реальной установки включаются переменной окружения `UNICA_PLATFORM_HELP_DIR`.
- Версия обязательна в каждом попадании и равна версии прочитанной установки. Подстановка справки соседней версии запрещена: между 8.3.27 и 8.5.4 расходятся 666 имён API.
- Локальный поставщик не выполняет сетевых обращений. Ни один код этого плана не открывает сокет.
- Идентификаторы поставщиков и корпусов — стабильные строки: `platform-syntax-help`, корпуса `syntax-context` и `platform-guides`.
- Тест пишется до исправления и проверяется на падение. Тест, написанный после правки, закрепляет поведение, но ничего не доказывает.

---

### Task 1: Разбор контейнера v8

Контейнер `.hbk` — блочный формат с шестнадцатибайтовым заголовком и таблицей записей. Подводный камень измерен: в семи контейнерах из тридцати восьми встречается запись с ограничителем `0x7FFFFFFF` в поле адреса тела, и её нужно пропускать, а не считать концом таблицы.

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform_help/container.rs`
- Create: `crates/unica-coder/src/infrastructure/platform_help.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Test: внутри `container.rs`, модуль `#[cfg(test)]`

**Interfaces:**
- Produces: `pub struct V8Container` с `pub fn parse(bytes: &[u8]) -> Result<V8Container, ContainerError>` и `pub fn entry(&self, name: &str) -> Option<&[u8]>`; `pub enum ContainerError { BadSignature, TruncatedBlock, BadBlockHeader }`.

- [ ] **Step 1: Написать падающий тест на разбор синтетического контейнера**

Фикстура собирается в тесте, файлов вендора в репозитории нет.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Собирает блок в формате контейнера: заголовок 31 байт, затем данные.
    fn block(data: &[u8], next: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(format!("{:08x} {:08x} {:08x} ", data.len(), data.len(), next).as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(data);
        out
    }

    /// Заголовок записи: 20 байт служебных, затем имя в UTF-16LE и нулевой разделитель.
    fn entry_header(name: &str) -> Vec<u8> {
        let mut out = vec![0u8; 20];
        for unit in name.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    /// `terminator_at` — индекс в таблице, куда вставить запись-ограничитель.
    /// Тест на пропуск обязан ставить её ПЕРЕД реальной записью: в конце
    /// таблицы `continue` и `break` дают одинаковый результат, и тест
    /// перестаёт различать исправление и дефект.
    fn container_with(entries: &[(&str, &[u8])], terminator_at: Option<usize>) -> Vec<u8> {
        let mut body = Vec::new();
        let mut addresses = Vec::new();
        // Резервируем место под заголовок файла и блок таблицы.
        let toc_len = 12 * (entries.len() + usize::from(terminator_at.is_some()));
        let toc_block_len = 31 + toc_len;
        let mut cursor = 16 + toc_block_len;
        for (name, data) in entries {
            let header = block(&entry_header(name), 0x7FFF_FFFF);
            let payload = block(data, 0x7FFF_FFFF);
            addresses.push((cursor as u32, (cursor + header.len()) as u32));
            cursor += header.len() + payload.len();
            body.extend_from_slice(&header);
            body.extend_from_slice(&payload);
        }
        let terminator_record = [0x7FFF_FFFFu32; 3];
        let mut toc = Vec::new();
        for (index, (head, data)) in addresses.iter().enumerate() {
            if terminator_at == Some(index) {
                // Запись, у которой адрес тела равен ограничителю: встречается в
                // семи контейнерах установки и не является концом таблицы.
                for field in terminator_record {
                    toc.extend_from_slice(&field.to_le_bytes());
                }
            }
            toc.extend_from_slice(&head.to_le_bytes());
            toc.extend_from_slice(&data.to_le_bytes());
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        }
        if terminator_at.is_some_and(|index| index >= addresses.len()) {
            for field in terminator_record {
                toc.extend_from_slice(&field.to_le_bytes());
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        out.extend_from_slice(&512u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&block(&toc, 0x7FFF_FFFF));
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_named_entries() {
        let bytes = container_with(&[("Book", b"book-body"), ("FileStorage", b"zip-body")], None);
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("Book"), Some(&b"book-body"[..]));
        assert_eq!(container.entry("FileStorage"), Some(&b"zip-body"[..]));
        assert_eq!(container.entry("Missing"), None);
    }

    #[test]
    fn terminator_entry_is_skipped_not_treated_as_end_of_table() {
        // Ограничитель стоит ПЕРЕД `FileStorage`: с `break` эта запись стала бы
        // недостижимой, и тест бы упал. С ограничителем в конце таблицы тест
        // прошёл бы при любом поведении и ничего не доказывал.
        let bytes = container_with(&[("Book", b"book-body"), ("FileStorage", b"zip-body")], Some(1));
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("Book"), Some(&b"book-body"[..]));
        assert_eq!(
            container.entry("FileStorage"),
            Some(&b"zip-body"[..]),
            "запись после ограничителя обязана остаться достижимой"
        );
    }

    #[test]
    fn wrong_signature_is_rejected() {
        let mut bytes = container_with(&[("Book", b"x")], None);
        bytes[0] = 0;
        bytes[1] = 0;
        bytes[2] = 0;
        bytes[3] = 0;
        assert!(matches!(V8Container::parse(&bytes), Err(ContainerError::BadSignature)));
    }
}
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder platform_help::container -- --test-threads=1`
Expected: FAIL — `cannot find type V8Container in this scope`.

- [ ] **Step 3: Реализовать разбор**

```rust
use std::collections::BTreeMap;

const BLOCK_HEADER: usize = 31;
const TERMINATOR: u32 = 0x7FFF_FFFF;

#[derive(Debug, PartialEq, Eq)]
pub enum ContainerError {
    BadSignature,
    TruncatedBlock,
    BadBlockHeader,
}

pub struct V8Container {
    entries: BTreeMap<String, Vec<u8>>,
}

struct BlockHeader {
    data_size: usize,
    page_size: usize,
    next_page: u32,
}

fn read_block_header(bytes: &[u8], offset: usize) -> Result<BlockHeader, ContainerError> {
    let end = offset.checked_add(BLOCK_HEADER).ok_or(ContainerError::TruncatedBlock)?;
    let raw = bytes.get(offset..end).ok_or(ContainerError::TruncatedBlock)?;
    if &raw[..2] != b"\r\n" {
        return Err(ContainerError::BadBlockHeader);
    }
    let text = std::str::from_utf8(&raw[2..BLOCK_HEADER - 2]).map_err(|_| ContainerError::BadBlockHeader)?;
    let mut parts = text.split(' ');
    let mut next_hex = || -> Result<u32, ContainerError> {
        let field = parts.next().ok_or(ContainerError::BadBlockHeader)?;
        u32::from_str_radix(field, 16).map_err(|_| ContainerError::BadBlockHeader)
    };
    let data_size = next_hex()? as usize;
    let page_size = next_hex()? as usize;
    let next_page = next_hex()?;
    Ok(BlockHeader { data_size, page_size, next_page })
}

/// Читает блок, следуя цепочке страниц: одна запись может не поместиться в
/// одну страницу, и продолжение адресуется полем `next_page`.
fn read_block(bytes: &[u8], offset: usize) -> Result<Vec<u8>, ContainerError> {
    let header = read_block_header(bytes, offset)?;
    let mut remaining = header.data_size;
    let mut out = Vec::with_capacity(remaining);
    let mut start = offset + BLOCK_HEADER;
    let mut page = header.page_size;
    let mut next = header.next_page;
    loop {
        let take = page.min(remaining);
        let end = start.checked_add(take).ok_or(ContainerError::TruncatedBlock)?;
        out.extend_from_slice(bytes.get(start..end).ok_or(ContainerError::TruncatedBlock)?);
        remaining -= take;
        if remaining == 0 || next == TERMINATOR {
            break;
        }
        let cont = read_block_header(bytes, next as usize)?;
        start = next as usize + BLOCK_HEADER;
        page = cont.page_size;
        next = cont.next_page;
    }
    Ok(out)
}

fn entry_name(header: &[u8]) -> String {
    let tail = header.get(20..).unwrap_or_default();
    let units: Vec<u16> = tail
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

impl V8Container {
    pub fn parse(bytes: &[u8]) -> Result<V8Container, ContainerError> {
        if bytes.len() < 16 || bytes[..4] != TERMINATOR.to_le_bytes() {
            return Err(ContainerError::BadSignature);
        }
        let toc = read_block(bytes, 16)?;
        let mut entries = BTreeMap::new();
        for record in toc.chunks_exact(12) {
            let head = u32::from_le_bytes([record[0], record[1], record[2], record[3]]);
            let data = u32::from_le_bytes([record[4], record[5], record[6], record[7]]);
            // Пропуск, а не выход: запись с ограничителем встречается в
            // середине таблицы у семи контейнеров установки.
            if head == TERMINATOR || data == TERMINATOR {
                continue;
            }
            let name = entry_name(&read_block(bytes, head as usize)?);
            entries.insert(name, read_block(bytes, data as usize)?);
        }
        Ok(V8Container { entries })
    }

    pub fn entry(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }
}
```

Файл `crates/unica-coder/src/infrastructure/platform_help.rs` объявляет модуль:

```rust
pub mod container;
```

В `crates/unica-coder/src/infrastructure/mod.rs` добавьте `pub mod platform_help;` в алфавитном порядке среди существующих объявлений.

- [ ] **Step 4: Запустить тесты и убедиться, что они проходят**

Run: `cargo test -p unica-coder platform_help::container -- --test-threads=1`
Expected: PASS, три теста.

- [ ] **Step 5: Проверить формат и линтер**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: обе команды без вывода ошибок.

- [ ] **Step 6: Коммит**

```bash
git add crates/unica-coder/src/infrastructure/platform_help.rs crates/unica-coder/src/infrastructure/platform_help/container.rs crates/unica-coder/src/infrastructure/mod.rs
git commit -m "feat(platform-help): разобрать контейнер справки платформы"
```

---

### Task 2: Страницы и сигнатуры корпуса

`FileStorage` — обычный ZIP. У Синтакс-помощника страницы лежат с расширением `.html`, рядом файл `.st` с двуязычной сигнатурой. У контейнеров подсистем расширения нет вовсе, и страницу опознают по разметке в начале записи.

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform_help/corpus.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_help.rs`
- Modify: `crates/unica-coder/Cargo.toml`
- Test: внутри `corpus.rs`, модуль `#[cfg(test)]`

**Interfaces:**
- Consumes: `V8Container::parse`, `V8Container::entry` из Task 1.
- Produces: `pub struct CorpusPage { pub path: String, pub title: String, pub text: String, pub signature: Option<Signature> }`; `pub struct Signature { pub ru: Option<String>, pub en: Option<String> }`; `pub fn read_corpus(bytes: &[u8]) -> Result<Vec<CorpusPage>, CorpusError>`; `pub enum CorpusError { Container(ContainerError), MissingFileStorage, BadArchive }`.

- [ ] **Step 1: Добавить зависимость `zip`**

В `crates/unica-coder/Cargo.toml`, в секцию `[dependencies]`, рядом с существующими:

```toml
zip = { version = "2", default-features = false, features = ["deflate"] }
```

- [ ] **Step 2: Написать падающий тест**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options: zip::write::FileOptions<()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in entries {
                writer.start_file(*name, options).expect("запись открыта");
                writer.write_all(data).expect("запись записана");
            }
            writer.finish().expect("архив закрыт");
        }
        buffer.into_inner()
    }

    const ST: &[u8] = br#"{1,
{2,
{"",1,0,"",""},
{0,
{"ru",0,0,"","ПолучитьНавигационнуюСсылку()"}
},
{0,
{"en",0,0,"","GetURL()"}
}
}
}"#;

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
        assert!(page.title.starts_with("Глобальный контекст.ПолучитьНавигационнуюСсылку"));
        assert!(page.text.contains("Синтаксис"));
        let signature = page.signature.as_ref().expect("сигнатура найдена");
        assert_eq!(signature.ru.as_deref(), Some("ПолучитьНавигационнуюСсылку()"));
        assert_eq!(signature.en.as_deref(), Some("GetURL()"));
    }

    #[test]
    fn reads_extensionless_page_by_markup() {
        let archive = zip_with(&[
            ("WebServices", "<html><body><p>Web-сервисы</p></body></html>".as_bytes()),
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
        assert!(matches!(read_corpus(&bytes), Err(CorpusError::MissingFileStorage)));
    }
}
```

Третий тест опирается на фикстуру из Task 1. Модуль `tests_support` в
`container.rs` уже существует и объявлен так:

```rust
#[cfg(test)]
pub(crate) mod tests_support {
    pub(crate) fn block(data: &[u8], next: u32) -> Vec<u8> { /* из Task 1 */ }
    pub(crate) fn entry_header(name: &str) -> Vec<u8> { /* из Task 1 */ }
    pub(crate) fn container_with(entries: &[(&str, &[u8])], terminator_at: Option<usize>) -> Vec<u8> { /* из Task 1 */ }
    pub(crate) fn container_without_file_storage() -> Vec<u8> {
        container_with(&[("Book", b"book-body")], None)
    }
}
```

Ничего в нём не переобъявляйте. Если `container_without_file_storage` там ещё
нет — добавьте только её, ровно в показанном виде. В тестах `corpus.rs`
обращайтесь так:

```rust
use crate::infrastructure::platform_help::container::tests_support::container_without_file_storage;
```

- [ ] **Step 3: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder platform_help::corpus -- --test-threads=1`
Expected: FAIL — `cannot find function read_corpus_from_archive`.

- [ ] **Step 4: Реализовать чтение корпуса**

```rust
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
    let storage = container.entry("FileStorage").ok_or(CorpusError::MissingFileStorage)?;
    read_corpus_from_archive(storage)
}

pub fn read_corpus_from_archive(archive: &[u8]) -> Result<Vec<CorpusPage>, CorpusError> {
    let cursor = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|_| CorpusError::BadArchive)?;
    let mut bodies: Vec<(String, Vec<u8>)> = Vec::with_capacity(zip.len());
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|_| CorpusError::BadArchive)?;
        let mut body = Vec::new();
        file.read_to_end(&mut body).map_err(|_| CorpusError::BadArchive)?;
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
        let stem = name.rsplit_once('.').map(|(head, _)| head).unwrap_or(name.as_str());
        pages.push(CorpusPage {
            path: name.clone(),
            title: if title.is_empty() { name.clone() } else { title },
            text,
            signature: signatures.get(stem).cloned(),
        });
    }
    Ok(pages)
}
```

- [ ] **Step 5: Запустить тесты**

Run: `cargo test -p unica-coder platform_help -- --test-threads=1`
Expected: PASS, шесть тестов (три из Task 1, три новых).

- [ ] **Step 6: Проверить формат и линтер**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: без ошибок.

- [ ] **Step 7: Коммит**

```bash
git add crates/unica-coder/Cargo.toml Cargo.lock crates/unica-coder/src/infrastructure/platform_help.rs crates/unica-coder/src/infrastructure/platform_help/
git commit -m "feat(platform-help): читать страницы и двуязычные сигнатуры корпуса"
```

---

### Task 3: Установка, версия и состав корпусов

Полная справка приходит только с полной установкой: на замеренной машине пять установок 8.5.1 несут по 16 контейнеров и 229 страниц без `shcntx` вовсе. Такая установка обязана давать диагностичный отказ, а не подстановку соседней версии.

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform_help/installation.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_help.rs`
- Test: внутри `installation.rs`, модуль `#[cfg(test)]`

**Interfaces:**
- Produces: `pub struct InstallationCorpora { pub version: String, pub syntax_context: Vec<std::path::PathBuf>, pub platform_guides: Vec<std::path::PathBuf> }`; `pub fn discover(root: &std::path::Path, language: &str) -> Result<InstallationCorpora, InstallationError>`; `pub enum InstallationError { NotFound, HelpMissingForVersion { version: String } }`.

- [ ] **Step 1: Написать падающий тест**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn install(version: &str, files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("временный каталог");
        let root = dir.path().join(version);
        std::fs::create_dir_all(&root).expect("каталог версии");
        for name in files {
            std::fs::write(root.join(name), b"stub").expect("файл");
        }
        dir
    }

    #[test]
    fn full_installation_splits_syntax_and_guides() {
        let dir = install("8.3.27.2074", &["shcntx_ru.hbk", "shlang_ru.hbk", "1cv8_ru.hbk", "mngbase_ru.hbk"]);
        let corpora = discover(&dir.path().join("8.3.27.2074"), "ru").expect("корпуса найдены");
        assert_eq!(corpora.version, "8.3.27.2074");
        assert_eq!(corpora.syntax_context.len(), 1, "syntax-context — только shcntx");
        assert!(corpora.syntax_context[0].ends_with("shcntx_ru.hbk"));
        assert_eq!(corpora.platform_guides.len(), 3, "остальные контейнеры — platform-guides");
    }

    #[test]
    fn client_only_installation_reports_help_missing() {
        // Тонкий клиент: контейнеры подсистем есть, Синтакс-помощника нет.
        let dir = install("8.5.1.1451", &["chartui_ru.hbk", "ecsui_ru.hbk"]);
        let error = discover(&dir.path().join("8.5.1.1451"), "ru").expect_err("отказ");
        assert!(matches!(error, InstallationError::HelpMissingForVersion { ref version } if version == "8.5.1.1451"));
    }

    #[test]
    fn absent_directory_is_not_found() {
        let dir = tempfile::tempdir().expect("временный каталог");
        assert!(matches!(discover(&dir.path().join("missing"), "ru"), Err(InstallationError::NotFound)));
    }
}
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder platform_help::installation -- --test-threads=1`
Expected: FAIL — `cannot find function discover`.

- [ ] **Step 3: Реализовать разбор установки**

```rust
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstallationError {
    NotFound,
    HelpMissingForVersion { version: String },
}

#[derive(Debug, Clone)]
pub struct InstallationCorpora {
    pub version: String,
    pub syntax_context: Vec<PathBuf>,
    pub platform_guides: Vec<PathBuf>,
}

pub fn discover(root: &Path, language: &str) -> Result<InstallationCorpora, InstallationError> {
    let version = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(InstallationError::NotFound)?
        .to_string();
    let entries = std::fs::read_dir(root).map_err(|_| InstallationError::NotFound)?;
    let suffix = format!("_{language}.hbk");
    let mut syntax_context = Vec::new();
    let mut platform_guides = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.ends_with(&suffix) {
            continue;
        }
        if name.starts_with("shcntx_") {
            syntax_context.push(path);
        } else {
            platform_guides.push(path);
        }
    }
    if syntax_context.is_empty() {
        // Клиентская поставка несёт справку подсистем, но не Синтакс-помощник.
        // Подставлять корпус соседней версии запрещено.
        return Err(InstallationError::HelpMissingForVersion { version });
    }
    syntax_context.sort();
    platform_guides.sort();
    Ok(InstallationCorpora { version, syntax_context, platform_guides })
}
```

Добавьте `pub mod installation;` в `platform_help.rs` и `tempfile` в `[dev-dependencies]` крейта, если его там ещё нет.

- [ ] **Step 4: Запустить тесты**

Run: `cargo test -p unica-coder platform_help -- --test-threads=1`
Expected: PASS, девять тестов.

- [ ] **Step 5: Коммит**

```bash
git add crates/unica-coder/Cargo.toml Cargo.lock crates/unica-coder/src/infrastructure/platform_help.rs crates/unica-coder/src/infrastructure/platform_help/installation.rs
git commit -m "feat(platform-help): различать полную и клиентскую установку по составу корпусов"
```

---

### Task 4: Доменный контракт поставщика документации

Контракт отделён от `CodeIntelligenceProvider`: данные, версии, доверие и лицензии у них различны, общего доменного контракта ADR-0029 не вводит.

**Files:**
- Create: `crates/unica-coder/src/domain/documentation.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: внутри `documentation.rs`, модуль `#[cfg(test)]`

**Interfaces:**
- Produces: `pub struct DocumentationProviderId(String)`; `pub enum SourceKind { PlatformHelp, DevelopmentStandard }`; `pub enum Authority { Vendor, Community }`; `pub enum DocumentationSectionStatus { Ok, Empty, Unavailable { reason: UnavailableReason, detail: String }, Failed { diagnostic: String } }`; `pub enum UnavailableReason { NotConfigured, VersionMissing, PolicyDenied, Timeout }`; `pub struct DocumentationHit`; `pub struct DocumentationSection`; `pub struct DocumentationSearchRequest`; `pub struct DocumentationContext`; `pub trait DocumentationProvider` с `search(...) -> Vec<DocumentationSection>`; `pub struct DocumentationRegistry` с `new(...) -> Result<Self, String>` и `providers()`.

- [ ] **Step 1: Написать падающий тест**

```rust
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
        fn search(&self, _: &DocumentationSearchRequest, _: &DocumentationContext) -> Vec<DocumentationSection> {
            vec![DocumentationSection::empty(self.id(), "corpus", SourceKind::PlatformHelp, Authority::Vendor)]
        }
    }

    #[test]
    fn registry_preserves_declared_order() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Fake { id: "first" }) as Arc<dyn DocumentationProvider>,
            Arc::new(Fake { id: "second" }),
        ])
        .expect("реестр собран");
        let ids: Vec<String> = registry.providers().map(|p| p.id().to_string()).collect();
        assert_eq!(ids, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn duplicate_provider_ids_are_rejected() {
        let error = DocumentationRegistry::new(vec![
            Arc::new(Fake { id: "same" }) as Arc<dyn DocumentationProvider>,
            Arc::new(Fake { id: "same" }),
        ])
        .expect_err("дубликат отклонён");
        assert!(error.contains("same"));
    }

    #[test]
    fn hit_requires_provenance_and_version() {
        let hit = DocumentationHit {
            rank: 1,
            provider_score: 0.5,
            document_id: "platform-syntax-help:syntax-context:objects/x.html".to_string(),
            title: "Заголовок".to_string(),
            signature: None,
            snippet: "текст".to_string(),
            applicable_version: "8.3.27.2074".to_string(),
        };
        assert!(!hit.applicable_version.is_empty(), "версия обязательна в каждом попадании");
        assert!(hit.document_id.starts_with("platform-syntax-help:"), "локатор несёт поставщика и корпус");
    }
}
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder domain::documentation -- --test-threads=1`
Expected: FAIL — модуль не найден.

- [ ] **Step 3: Реализовать доменные типы**

```rust
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
    Unavailable { reason: UnavailableReason, detail: String },
    Failed { diagnostic: String },
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
    pub status: DocumentationSectionStatus,
    pub hits: Vec<DocumentationHit>,
}

impl DocumentationSection {
    pub fn empty(
        provider: DocumentationProviderId,
        corpus: &str,
        source_kind: SourceKind,
        authority: Authority,
    ) -> Self {
        Self {
            provider,
            corpus: corpus.to_string(),
            source_kind,
            authority,
            status: DocumentationSectionStatus::Empty,
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
```

Добавьте `pub mod documentation;` в `crates/unica-coder/src/domain/mod.rs`.

- [ ] **Step 4: Запустить тесты**

Run: `cargo test -p unica-coder domain::documentation -- --test-threads=1`
Expected: PASS, три теста.

- [ ] **Step 5: Коммит**

```bash
git add crates/unica-coder/src/domain/documentation.rs crates/unica-coder/src/domain/mod.rs
git commit -m "feat(documentation): ввести нейтральный к источнику контракт поставщика"
```

---

### Task 5: Поставщик справки установленной платформы

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform_help/provider.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_help.rs`
- Test: внутри `provider.rs`, модуль `#[cfg(test)]`; плюс `crates/unica-coder/tests/platform_syntax_help.rs`

**Interfaces:**
- Consumes: `read_corpus` из Task 2, `discover` из Task 3, доменные типы из Task 4.
- Produces: `pub struct PlatformSyntaxHelpProvider` с `pub fn new(language: &str) -> Self`, реализующий `DocumentationProvider` и возвращающий по секции на каждый из двух корпусов; `pub fn rank_pages(pages: &[CorpusPage], query: &str, limit: usize, version: &str, corpus: &str) -> Vec<DocumentationHit>`.

- [ ] **Step 1: Написать падающий тест на поведение поставщика**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::documentation::*;

    fn request(query: &str) -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: query.to_string(),
            source_kinds: vec![SourceKind::PlatformHelp],
            limit: 20,
            language: "ru".to_string(),
        }
    }

    #[test]
    fn missing_installation_is_unavailable_not_failed() {
        let provider = PlatformSyntaxHelpProvider::new("ru");
        let context = DocumentationContext { platform_version: None, installation_root: None };
        let sections = provider.search(&request("ПолучитьНавигационнуюСсылку"), &context);
        assert_eq!(sections.len(), 1, "без установки — одна диагностичная секция");
        let section = &sections[0];
        assert!(matches!(
            section.status,
            DocumentationSectionStatus::Unavailable { reason: UnavailableReason::NotConfigured, .. }
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
        assert_eq!(sections.len(), 1, "без Синтакс-помощника — одна диагностичная секция");
        let section = &sections[0];
        assert!(matches!(
            section.status,
            DocumentationSectionStatus::Unavailable { reason: UnavailableReason::VersionMissing, .. }
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
        assert!(hits.iter().all(|hit| hit.applicable_version == "8.3.27.2074"));
    }
}
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder platform_help::provider -- --test-threads=1`
Expected: FAIL — `cannot find type PlatformSyntaxHelpProvider`.

- [ ] **Step 3: Реализовать поставщика**

```rust
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
        Self { language: language.to_string(), cache: Arc::new(Mutex::new(CorpusCache::default())) }
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
                    detail: "установка платформы не разрешена для рабочего пространства".to_string(),
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
            Err(InstallationError::NotFound) => {
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
            let hits = rank_pages(pages, &request.query, request.limit, &corpora.version, corpus);
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
```

- [ ] **Step 4: Написать проверку против реальной установки, включаемую переменной окружения**

Create: `crates/unica-coder/tests/platform_syntax_help.rs`

```rust
//! Проверка против реальной установки платформы. Пропускается, когда
//! `UNICA_PLATFORM_HELP_DIR` не задан: материалы вендора в репозиторий не
//! попадают и в CI не требуются.

#[test]
fn real_installation_answers_navigation_link_question() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        eprintln!("UNICA_PLATFORM_HELP_DIR не задан — проверка пропущена");
        return;
    };
    let provider = unica_coder::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider::new("ru");
    let request = unica_coder::domain::documentation::DocumentationSearchRequest {
        query: "ПолучитьНавигационнуюСсылку".to_string(),
        source_kinds: vec![unica_coder::domain::documentation::SourceKind::PlatformHelp],
        limit: 10,
        language: "ru".to_string(),
    };
    let context = unica_coder::domain::documentation::DocumentationContext {
        platform_version: None,
        installation_root: Some(std::path::PathBuf::from(root)),
    };
    let sections = <unica_coder::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider
        as unica_coder::domain::documentation::DocumentationProvider>::search(&provider, &request, &context);
    let section = sections
        .iter()
        .find(|section| section.corpus == "syntax-context")
        .expect("секция Синтакс-помощника");
    assert!(matches!(
        section.status,
        unica_coder::domain::documentation::DocumentationSectionStatus::Ok
    ));
    let hit = section.hits.first().expect("попадание найдено");
    assert!(hit.title.contains("ПолучитьНавигационнуюСсылку"));
    assert!(!hit.applicable_version.is_empty());
}
```

- [ ] **Step 5: Запустить тесты**

Run: `cargo test -p unica-coder platform_help -- --test-threads=1`
Expected: PASS. Проверка против установки печатает сообщение о пропуске.

Затем разово, вручную, против реальной установки:

```bash
UNICA_PLATFORM_HELP_DIR=/opt/1cv8/8.3.27.2074 cargo test -p unica-coder --test platform_syntax_help -- --nocapture
```
Expected: PASS с найденным попаданием.

- [ ] **Step 6: Коммит**

```bash
git add crates/unica-coder/src/infrastructure/platform_help/ crates/unica-coder/src/infrastructure/platform_help.rs crates/unica-coder/tests/platform_syntax_help.rs
git commit -m "feat(platform-help): отдать справку установки секцией поставщика"
```

---

### Task 6: Публичный `unica.documentation.search`

**Files:**
- Create: `crates/unica-coder/src/application/documentation.rs`
- Modify: `crates/unica-coder/src/application/mod.rs` (перечисление `ToolHandler` около строки 40; список `ToolSpec` около строки 525)
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `spec/architecture/tool-surface-review.json`
- Modify: `spec/architecture/tool-surface.md` (генерируется скриптом)
- Test: внутри `application/documentation.rs`; плюс `tests/ci/test_tool_surface_ledger.py` проходит без правок

**Interfaces:**
- Consumes: `DocumentationRegistry`, `DocumentationProvider` из Task 4; `PlatformSyntaxHelpProvider` из Task 5.
- Produces: `pub fn search(registry: &DocumentationRegistry, request: &DocumentationSearchRequest, context: &DocumentationContext) -> Result<serde_json::Value, String>`; вариант `ToolHandler::Documentation { operation: &'static str }`.

- [ ] **Step 1: Написать падающий тест на сборку результата**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::documentation::*;
    use std::sync::Arc;

    struct Stub {
        id: &'static str,
        section: DocumentationSection,
    }

    impl DocumentationProvider for Stub {
        fn id(&self) -> DocumentationProviderId {
            DocumentationProviderId::new(self.id)
        }
        fn corpora(&self) -> Vec<DocumentationCorpus> {
            Vec::new()
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn search(&self, _: &DocumentationSearchRequest, _: &DocumentationContext) -> Vec<DocumentationSection> {
            vec![self.section.clone()]
        }
    }

    fn ok_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            provider: DocumentationProviderId::new(id),
            corpus: "syntax-context".to_string(),
            source_kind: SourceKind::PlatformHelp,
            authority: Authority::Vendor,
            status: DocumentationSectionStatus::Ok,
            hits: vec![DocumentationHit {
                rank: 1,
                provider_score: 1.0,
                document_id: format!("{id}:syntax-context:page.html"),
                title: "Заголовок".to_string(),
                signature: Some("Имя()".to_string()),
                snippet: "текст".to_string(),
                applicable_version: "8.3.27.2074".to_string(),
            }],
        }
    }

    fn failed_section(id: &str) -> DocumentationSection {
        DocumentationSection {
            status: DocumentationSectionStatus::Failed { diagnostic: "сломалось".to_string() },
            hits: Vec::new(),
            ..ok_section(id)
        }
    }

    fn request() -> DocumentationSearchRequest {
        DocumentationSearchRequest {
            query: "x".to_string(),
            source_kinds: Vec::new(),
            limit: 20,
            language: "ru".to_string(),
        }
    }

    fn context() -> DocumentationContext {
        DocumentationContext { platform_version: None, installation_root: None }
    }

    #[test]
    fn sections_follow_registry_order_and_carry_provenance() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub { id: "first", section: ok_section("first") }) as Arc<dyn DocumentationProvider>,
            Arc::new(Stub { id: "second", section: ok_section("second") }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["provider"], "first");
        assert_eq!(sections[1]["provider"], "second");
        assert_eq!(sections[0]["sourceKind"], "platform-help");
        assert_eq!(sections[0]["authority"], "vendor");
        assert_eq!(sections[0]["hits"][0]["applicableVersion"], "8.3.27.2074");
    }

    #[test]
    fn one_failed_provider_does_not_hide_the_other() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub { id: "broken", section: failed_section("broken") }) as Arc<dyn DocumentationProvider>,
            Arc::new(Stub { id: "working", section: ok_section("working") }),
        ])
        .expect("реестр");
        let value = search(&registry, &request(), &context()).expect("результат");
        let sections = value["sections"].as_array().expect("массив секций");
        assert_eq!(sections[0]["status"], "failed");
        assert_eq!(sections[1]["status"], "ok");
        assert_eq!(sections[1]["hits"].as_array().expect("попадания").len(), 1);
    }

    #[test]
    fn all_providers_failed_is_an_error() {
        let registry = DocumentationRegistry::new(vec![
            Arc::new(Stub { id: "broken", section: failed_section("broken") }) as Arc<dyn DocumentationProvider>,
        ])
        .expect("реестр");
        assert!(search(&registry, &request(), &context()).is_err());
    }
}
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `cargo test -p unica-coder application::documentation -- --test-threads=1`
Expected: FAIL — `cannot find function search`.

- [ ] **Step 3: Реализовать сборку результата**

```rust
use serde_json::{json, Value};

use crate::domain::documentation::*;

fn status_fields(status: &DocumentationSectionStatus) -> (&'static str, Value) {
    match status {
        DocumentationSectionStatus::Ok => ("ok", Value::Null),
        DocumentationSectionStatus::Empty => ("empty", Value::Null),
        DocumentationSectionStatus::Unavailable { reason, detail } => (
            "unavailable",
            json!({ "reason": reason.as_str(), "detail": detail }),
        ),
        DocumentationSectionStatus::Failed { diagnostic } => {
            ("failed", json!({ "diagnostic": diagnostic }))
        }
    }
}

/// Поиск успешен, если хотя бы один применимый поставщик вернул `ok` или
/// `empty`. Отказ одного поставщика не скрывает результаты остальных.
pub fn search(
    registry: &DocumentationRegistry,
    request: &DocumentationSearchRequest,
    context: &DocumentationContext,
) -> Result<Value, String> {
    let mut sections = Vec::new();
    let mut any_usable = false;
    for provider in registry.providers() {
        for section in provider.search(request, context) {
        let (status, diagnostic) = status_fields(&section.status);
        if matches!(
            section.status,
            DocumentationSectionStatus::Ok | DocumentationSectionStatus::Empty
        ) {
            any_usable = true;
        }
        let hits: Vec<Value> = section
            .hits
            .iter()
            .map(|hit| {
                json!({
                    "rank": hit.rank,
                    "providerScore": hit.provider_score,
                    "documentId": hit.document_id,
                    "title": hit.title,
                    "signature": hit.signature,
                    "snippet": hit.snippet,
                    "applicableVersion": hit.applicable_version,
                })
            })
            .collect();
        sections.push(json!({
            "provider": section.provider.to_string(),
            "corpus": section.corpus,
            "sourceKind": section.source_kind.as_str(),
            "authority": section.authority.as_str(),
            "status": status,
            "diagnostic": diagnostic,
            "hits": hits,
        }));
        }
    }
    if !any_usable {
        return Err("ни один поставщик документации не дал результата".to_string());
    }
    Ok(json!({ "sections": sections }))
}
```

- [ ] **Step 4: Объявить инструмент на публичной поверхности**

В `crates/unica-coder/src/application/mod.rs`, в перечисление `ToolHandler` (около строки 40) добавьте вариант рядом с `StandardsAdapter`:

```rust
    Documentation {
        operation: &'static str,
    },
```

В список `ToolSpec` (около строки 525, рядом с объявлением `unica.standards.search`) добавьте:

```rust
        ToolSpec {
            name: "unica.documentation.search",
            description:
                "Search platform help and development standards across documentation providers.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::Documentation {
                operation: "search",
            },
        },
```

В `crates/unica-coder/src/infrastructure/application_ports.rs`, в `match` по `ToolHandler` (рядом с веткой `ToolHandler::StandardsAdapter`, около строки 387) добавьте ветку:

```rust
            ToolHandler::Documentation { operation } => {
                if operation != "search" {
                    return Err(format!("unknown documentation operation: {operation}"));
                }
                let request = crate::domain::documentation::DocumentationSearchRequest {
                    query: args
                        .get("query")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "unica.documentation.search requires query".to_string())?
                        .to_string(),
                    source_kinds: Vec::new(),
                    limit: args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(20)
                        .min(200) as usize,
                    language: args
                        .get("language")
                        .and_then(Value::as_str)
                        .unwrap_or("ru")
                        .to_string(),
                };
                let requested_version = args.get("platformVersion").and_then(Value::as_str);
                let context = crate::domain::documentation::DocumentationContext {
                    platform_version: requested_version.map(str::to_string),
                    installation_root: resolve_platform_installation_root(requested_version),
                };
                let registry = documentation_registry()?;
                let data = crate::application::documentation::search(&registry, &request, &context)?;
                Ok(HandlerOutcome::with_data(
                    AdapterOutcome {
                        status: AdapterStatus::Completed,
                        summary: "unica.documentation.search completed".to_string(),
                    },
                    data,
                ))
            }
```

Рядом, в том же файле, добавьте сборку реестра и разрешение установки. Реестр собирается здесь — в корне композиции, — а не в доменном слое, чтобы тесты могли внедрить подменных поставщиков:

```rust
/// Корень композиции: реестр поставщиков документации. Порядок объявления
/// задаёт порядок секций публичного результата.
fn documentation_registry() -> Result<crate::domain::documentation::DocumentationRegistry, String> {
    use std::sync::Arc;

    crate::domain::documentation::DocumentationRegistry::new(vec![Arc::new(
        crate::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider::new("ru"),
    )
        as Arc<dyn crate::domain::documentation::DocumentationProvider>])
}

/// Корень установки платформы берётся из того же перечня корней, которым уже
/// пользуется публикация полной выгрузки: `default_platform_roots()` в
/// `infrastructure::platform::full_dump_publication`. Второго механизма поиска
/// установки в проекте не появляется.
///
/// Из корня выбирается подкаталог версии: явная версия из аргумента вызова,
/// иначе — старшая по лексикографическому порядку среди присутствующих. Полный
/// порядок с версией проекта приходит во втором плане вместе с `unica.toml`.
fn resolve_platform_installation_root(requested: Option<&str>) -> Option<std::path::PathBuf> {
    for root in crate::infrastructure::platform::full_dump_publication::default_platform_roots() {
        let Ok(children) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut versions: Vec<std::path::PathBuf> = children
            .flatten()
            .map(|child| child.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort();
        if let Some(version) = requested {
            if let Some(hit) = versions.iter().find(|path| {
                path.file_name().and_then(|name| name.to_str()) == Some(version)
            }) {
                return Some(hit.clone());
            }
            continue;
        }
        if let Some(latest) = versions.last() {
            return Some(latest.clone());
        }
    }
    None
}
```

Функция `default_platform_roots` сейчас приватная — сделайте её `pub(crate)` в `full_dump_publication.rs` и ничего в ней не меняйте: перечень корней остаётся один на проект.

Переменная `UNICA_PLATFORM_HELP_DIR` остаётся исключительно тестовой и в рабочий путь разрешения не входит.

- [ ] **Step 5: Обновить ведомость поверхности**

В `spec/architecture/tool-surface-review.json` добавьте запись, сохраняя алфавитный порядок ключей:

```json
  "unica.documentation.search": {
    "result": {
      "contract": "typed",
      "now": "`data`: секции поставщиков документации с происхождением и версией (ADR-0023)",
      "target": "достигнут"
    },
    "scenarios": [
      "Уточнить сигнатуру и доступность метода платформы до написания кода",
      "Проверить поведение механизма платформы для конкретной версии установки",
      "Отличить справку платформы от стандарта разработки в одном ответе"
    ],
    "scope": "in"
  },
```

Затем перегенерируйте `spec/architecture/tool-surface.md`:

Run: `python3 scripts/ci/generate-tool-surface.py`

- [ ] **Step 6: Запустить проверки**

Run: `cargo test -p unica-coder -- --test-threads=1`
Expected: PASS.

Run: `python3 scripts/ci/generate-tool-surface.py --check && /opt/homebrew/bin/python3.12 -m unittest tests.ci.test_tool_surface_ledger tests.ci.test_architecture_registry`
Expected: PASS.

- [ ] **Step 7: Проверить формат и линтер**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: без ошибок.

- [ ] **Step 8: Коммит**

```bash
git add crates/unica-coder/src/application/documentation.rs crates/unica-coder/src/application/mod.rs crates/unica-coder/src/infrastructure/application_ports.rs spec/architecture/tool-surface-review.json spec/architecture/tool-surface.md
git commit -m "feat(documentation): опубликовать unica.documentation.search секциями поставщиков"
```

---

### Task 7: Скилл `platform-help`

**Files:**
- Modify: `plugins/unica/skills/platform-help/SKILL.md`
- Test: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: имя `unica.documentation.search` и поля результата из Task 6.

- [ ] **Step 1: Написать падающий тест правил скилла**

В `tests/ci/test_unica_skills.py` добавьте класс:

```python
class PlatformHelpRoutingTests(unittest.TestCase):
    """Скилл получил источник: отказ перестаёт быть штатным ответом."""

    def setUp(self) -> None:
        self.text = (
            REPO_ROOT / "plugins" / "unica" / "skills" / "platform-help" / "SKILL.md"
        ).read_text(encoding="utf-8")

    def test_routes_platform_questions_to_documentation_search(self) -> None:
        self.assertIn("unica.documentation.search", self.text)

    def test_keeps_the_standards_reading_rule(self) -> None:
        # Секция стандартов может прийти в том же ответе; правило вызова
        # превращается в правило чтения и должно остаться дословным.
        self.assertIn("development-standard", self.text)
        self.assertIn("не закрывает вопрос", self.text)

    def test_contract_gap_is_no_longer_the_default_answer(self) -> None:
        # Отказ сохраняется только для случая, когда ни один поставщик не
        # подтвердил ответ.
        self.assertNotIn(
            "Until it is exposed by public MCP `unica`, report this as a `platform-help` contract gap",
            self.text,
        )

    def test_states_the_source_boundary(self) -> None:
        self.assertIn("версию установки", self.text)
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_unica_skills.PlatformHelpRoutingTests`
Expected: FAIL — `unica.documentation.search` в скилле отсутствует.

- [ ] **Step 3: Переписать скилл**

Замените разделы `MCP routing`, `Stop rules` и `MCP examples` в `plugins/unica/skills/platform-help/SKILL.md`:

```markdown
## MCP routing

- For platform API and mechanics, use MCP `unica` tool `unica.documentation.search` with `sourceKinds: ["platform-help"]`.
- Каждое попадание несёт `sourceKind`, `authority` и `applicableVersion`. Ответ обязан называть источник, версию установки и локатор страницы.
- Секция со смыслом источника `development-standard` не закрывает вопрос о сигнатуре или механике платформы, каким бы уместным ни выглядел её текст. Это правило чтения, а не правило вызова.
- For project context, use `unica.code.search`, `unica.project.map`, and `unica.runtime.execute`.
- Use object-specific `unica.*.info` tools when the API question depends on metadata structure.
- Do not call internal standards, runtime, or package adapters directly.

## Workflow

1. State the exact platform/API question: object, method/property, platform version, infobase mode, client/server context.
2. Call `unica.documentation.search` with the object or member name.
3. Read `applicableVersion` in the hit. Если она расходится с версией проекта, назовите расхождение в ответе.
4. Validate against local project context with `unica.project.map` and targeted `unica.code.search` if the answer depends on project conventions.
5. For code examples, run `unica.runtime.execute` with `operation=syntax` when feasible.

## Stop rules

- Do not present a `development-standard` section as proof of platform API behavior or exact method signatures.
- Справка отвечает, что и с какими типами вызывать. Целостное описание механизма — за пределами источника: сообщите границу источника вместо ответа по памяти.
- Если секция вернула `unavailable` с причиной `version-missing`, назовите, какой установки не хватает. Не подставляйте справку соседней версии.
- Если ни один поставщик не дал подтверждения, сообщите `platform-help contract gap` и назовите требуемую версию и контекст.
```

- [ ] **Step 4: Запустить тесты скиллов**

Run: `/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_unica_skills`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add plugins/unica/skills/platform-help/SKILL.md tests/ci/test_unica_skills.py
git commit -m "feat(platform-help): маршрутизировать вопрос об API в поиск по документации"
```

---

### Task 8: Снять загрузчик и корпус `docs-local/1ci`

**Files:**
- Delete: `scripts/dev/download-1ci-guides.py`
- Delete: `tests/dev/test_download_1ci_guides.py`
- Modify: `AGENTS.md` (раздел «Локальная документация платформы 1Ci», строки 222–234)
- Modify: `tests/ci/test_product_contracts.py:302`

**Interfaces:**
- Consumes: работающий `unica.documentation.search` из Task 6 — именно он делает корпус ненужным.

- [ ] **Step 1: Переписать проверку продуктового контракта так, чтобы она падала**

В `tests/ci/test_product_contracts.py` замените `test_local_1ci_corpus_is_ignored_and_agent_discoverable` на:

```python
    def test_downloader_and_local_corpus_contract_are_retired(self) -> None:
        """Справка платформы приходит из установки, а не из скачанного корпуса.

        Загрузчик закреплял ровно ту болезнь, ради которой заведена #254:
        полная загрузка в каждом рабочем дереве ради точечного вопроса.
        """
        downloader = REPO_ROOT / "scripts" / "dev" / "download-1ci-guides.py"
        downloader_test = REPO_ROOT / "tests" / "dev" / "test_download_1ci_guides.py"
        agents = (REPO_ROOT / "AGENTS.md").read_text(encoding="utf-8")

        self.assertFalse(downloader.exists(), "загрузчик удалён вместе с контрактом корпуса")
        self.assertFalse(downloader_test.exists(), "тест загрузчика удалён вместе с ним")
        self.assertNotIn("download-1ci-guides.py", agents)
        self.assertNotIn("docs-local/1ci/8.3.27/en/", agents)
        self.assertNotIn("kb.1ci.com/bin/download", agents)

    def test_vendor_help_never_enters_the_package(self) -> None:
        """Материалы вендора читаются на машине пользователя и не пакуются."""
        package_script = (
            REPO_ROOT / "scripts" / "ci" / "package-unica-plugin.py"
        ).read_text(encoding="utf-8")
        self.assertNotIn("docs-local", package_script)
        self.assertNotIn(".hbk", package_script)
```

- [ ] **Step 2: Запустить тест и убедиться, что он падает**

Run: `/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_product_contracts`
Expected: FAIL — загрузчик ещё на месте, `AGENTS.md` ещё требует его запускать.

- [ ] **Step 3: Удалить загрузчик и его тест**

```bash
git rm scripts/dev/download-1ci-guides.py tests/dev/test_download_1ci_guides.py
```

- [ ] **Step 4: Убрать раздел из `AGENTS.md`**

Удалите целиком раздел «## Локальная документация платформы 1Ci» (строки 222–234) вместе с исключением для `https://kb.1ci.com/bin/download/*`. В разделе «Гигиена поиска» замените строку про `docs-local`:

```markdown
- `docs-local` (локальный исследовательский материал; в поставку не входит и
  источником справки платформы не является — её отдаёт `unica.documentation.search`)
```

Общий игнор `docs-local/` в `.gitignore` не трогайте: он нужен другим локальным исследованиям.

- [ ] **Step 5: Запустить тесты**

Run: `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/ci -p 'test_product_contracts.py'`
Expected: PASS.

Run: `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/dev`
Expected: PASS, без удалённого теста.

- [ ] **Step 6: Коммит**

```bash
git add -A AGENTS.md tests/ci/test_product_contracts.py scripts/dev tests/dev
git commit -m "refactor(docs): снять загрузчик и контракт корпуса 1Ci"
```

---

### Task 9: Полный прогон и синхронизация архитектуры

**Files:**
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/decisions/0029-dva-istochnika-spravki-platformy.md` (статус)
- Modify: `spec/decisions/README.md` (перенос записи в принятые)

- [ ] **Step 1: Добавить выведенное правило в реестр инвариантов**

В `spec/architecture/invariants.md`, в раздел `## MCP`, добавьте запись в алфавитном порядке среди соседей:

```markdown
### INV-MCP-DOCUMENTATION-SECTIONS — Поиск по документации сохраняет независимые секции поставщиков

- Rule: `unica.documentation.search` возвращает по одной секции на поставщика в
  порядке реестра. Каждое попадание несёт поставщика, корпус, смысл источника,
  авторитетность и применимую версию. Ранги и оценки локальны для секции: Unica
  не объединяет, не пересортировывает и не удаляет совпадения между секциями.
  Отказ одного поставщика остаётся в его секции.
- Decision: ADR-0029
- Проверка: `cargo test -p unica-coder application::documentation`
```

И в раздел `## APP`:

```markdown
### INV-APP-DOCUMENTATION-NO-DISK-STATE — Разбор корпуса справки не создаёт состояния на диске

- Rule: поставщик справки платформы читает контейнеры установки и держит карту
  корпусов в памяти процесса, привязанной к версии платформы. Индекс, кеш и
  распакованные страницы на диск не пишутся, в рабочее дерево не попадают и в
  пакет не включаются.
- Decision: ADR-0029
- Проверка: `cargo test -p unica-coder platform_help`, `python3.12 -m unittest tests.ci.test_product_contracts`
```

- [ ] **Step 2: Перевести ADR-0029 в принятые**

В `spec/decisions/0029-dva-istochnika-spravki-platformy.md` замените `- Статус: `proposed`` на `- Статус: `accepted``.

В `spec/decisions/README.md` перенесите строку записи из раздела «Предложенные решения» в конец раздела «Принятые решения» и верните в раздел предложенных текст-заглушку:

```markdown
## Предложенные решения

Сейчас предложенных решений нет. Новая запись в статусе `proposed` ещё не
действует и не заменяет принятые решения; после ревью она либо переводится в
`accepted` вместе с реализацией и выведенными проверяемыми правилами, либо
остаётся проектным предложением.
```

- [ ] **Step 3: Обновить проектную записку**

В `docs/design/2026-08-08-platform-help-documentation-provider-design.md` замените `- Status: `draft`` на `- Status: `approved``.

- [ ] **Step 4: Полный прогон**

Run: `cargo fmt --all -- --check`
Expected: без вывода.

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: без предупреждений.

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS.

Run: `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/ci`
Expected: PASS.

Run: `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/dev`
Expected: PASS.

Run: `python3 scripts/ci/generate-tool-surface.py --check && python3 scripts/ci/check-architecture-sync.py`
Expected: PASS.

- [ ] **Step 5: Коммит**

```bash
git add spec/architecture/invariants.md spec/decisions/0029-dva-istochnika-spravki-platformy.md spec/decisions/README.md docs/design/2026-08-08-platform-help-documentation-provider-design.md
git commit -m "docs(spec): принять ADR-0029 и вывести проверяемые правила"
```

---

## Что остаётся второму плану

Здесь сознательно не сделано, и это не пробел, а граница:

- файл `unica.toml` и `unica.local.toml` с политикой сетевого выхода, правилом «неясность — отказ» и порядком разрешения значений;
- перенос `v8std` за `DocumentationProvider` и сохранение `unica.standards.search` и `unica.standards.explain` как совместимых фасадов;
- поставщик `kb-1ci`: обход навигационного дерева площадки от объявленных корней, выбор документа под версию платформы, два корпуса — руководство разработчика и оба руководства администратора;
- `unica.documentation.get` — за задачей #242.

До второго плана `unica.documentation.search` возвращает одну секцию, и это корректное состояние: контракт секционный с самого начала, поставщик в реестре пока один.
