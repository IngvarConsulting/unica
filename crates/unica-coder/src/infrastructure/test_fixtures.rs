//! Файловые фикстуры корня конфигурации для модульных тестов.
//!
//! Пустая конфигурация 8.3.27 / 2.20 — выгрузка платформы
//! `tests/fixtures/platform_8_3_27/template-new-xml/seed/` без её отчёта.
//! Тесты, которым нужен корень набора, берут его отсюда, а не строят
//! писателем. Объекты для чтения описания лежат рядом, в `meta_info_surface/`.

use std::path::{Path, PathBuf};

/// Каталог фикстуры: `v8project.yaml` и набор `src/`.
pub(crate) fn empty_configuration_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/empty_configuration")
}

/// Копирует фикстуру в `workspace`: набор `main` в `src/` и его объявление.
pub(crate) fn copy_empty_configuration(workspace: &Path) {
    copy_tree(&empty_configuration_fixture(), workspace);
}

/// Пустое расширение `CorpusExtension` поверх этой конфигурации: корень
/// `ext/` и заимствованный язык, записанные по скиллу `cfe-init` (BOM, CRLF,
/// без роли). Набор в `v8project.yaml` не объявлен: это делает тест.
pub(crate) fn empty_extension_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/empty_extension")
}

/// Копирует корень пустого расширения в `workspace/ext`.
pub(crate) fn copy_empty_extension(workspace: &Path) {
    copy_tree(
        &empty_extension_fixture().join("ext"),
        &workspace.join("ext"),
    );
}

/// Копирует дерево каталога побайтно, создавая недостающие каталоги.
pub(crate) fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Объекты метаданных, однажды записанные писателем 8.3.27 для тестов
/// чтения описания: справочник, константа, регистр сведений, перечисление.
pub(crate) fn meta_info_surface_fixture(part: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/meta_info_surface")
        .join(part)
}

/// Платформенный порядок видов в `ChildObjects` корня, известных фикстурам.
const ROOT_REGISTRATION_ORDER: [&str; 5] = [
    "Language",
    "Constant",
    "Catalog",
    "Enum",
    "InformationRegister",
];

pub(crate) fn register_root_child(workspace: &Path, tag: &str, name: &str) {
    let rank = |candidate: &str| {
        ROOT_REGISTRATION_ORDER
            .iter()
            .position(|known| *known == candidate)
            .unwrap_or(ROOT_REGISTRATION_ORDER.len())
    };
    let path = workspace.join("src/Configuration.xml");
    let xml = std::fs::read_to_string(&path).unwrap();
    let open = xml.find("<ChildObjects>").expect("root ChildObjects") + "<ChildObjects>".len();
    let close = xml
        .rfind("</ChildObjects>")
        .expect("root ChildObjects closes");
    let mut insert_at = xml[..close].rfind('\n').map_or(close, |line| line + 1);
    let mut cursor = open;
    while let Some(relative) = xml[cursor..close].find("\t\t\t<") {
        let line = cursor + relative;
        let tag_end = line + 4 + xml[line + 4..].find('>').unwrap();
        if rank(&xml[line + 4..tag_end]) > rank(tag) {
            insert_at = line;
            break;
        }
        cursor = tag_end;
    }
    let mut updated = xml;
    updated.insert_str(insert_at, &format!("\t\t\t<{tag}>{name}</{tag}>\r\n"));
    std::fs::write(path, updated).unwrap();
}

/// Кладёт объект из фикстуры в набор `main` и регистрирует его в корне.
/// `renames` переименовывает объект в именах файлов и в байтах описаний.
pub(crate) fn add_fixture_object(
    workspace: &Path,
    part: &str,
    tag: &str,
    name: &str,
    renames: &[(&str, &str)],
) {
    fn copy(from: &Path, to: &Path, renames: &[(&str, &str)]) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let mut file_name = entry.file_name().to_string_lossy().into_owned();
            for (from, to) in renames {
                file_name = file_name.replace(from, to);
            }
            let target = to.join(file_name);
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target, renames);
            } else {
                let mut bytes = std::fs::read(entry.path()).unwrap();
                for (from, to) in renames {
                    bytes = String::from_utf8(bytes)
                        .unwrap()
                        .replace(from, to)
                        .into_bytes();
                }
                std::fs::write(target, bytes).unwrap();
            }
        }
    }
    copy(
        &meta_info_surface_fixture(part),
        &workspace.join("src"),
        renames,
    );
    register_root_child(workspace, tag, name);
}
