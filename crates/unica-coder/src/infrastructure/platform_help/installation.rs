//! Перечисление корпусов справки в каталоге установленной версии платформы.
//!
//! Задача этого модуля — чисто файловая: разобрать каталог одной установленной
//! версии на два корпуса контейнеров `.hbk`. Разбор содержимого контейнеров
//! (`container`, `corpus`) сюда не входит.
//!
//! Измерено на реальной машине: полная установка 8.3.27.2074 несёт 38
//! русскоязычных контейнеров, из которых `shcntx_ru.hbk` — Синтакс-помощник, а
//! остальные 37 — справка конфигуратора и подсистем. Пять установок 8.5.1 на
//! той же машине оказались клиентскими: по 16 контейнеров подсистем и ни
//! одного `shcntx`. Полная справка (примеры кода из Синтакс-помощника) есть
//! только в первом случае, поэтому корпуса ровно два — `syntax_context` и
//! `platform_guides` — и клиентская установка обязана давать диагностичный
//! отказ, называющий версию, а не подстановку корпуса соседней версии.

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

/// Разбирает каталог одной установленной версии (`root` — сам каталог версии,
/// например `.../8.3.27.2074`) на два корпуса контейнеров `.hbk` заданного
/// языка. Признак разделения — префикс имени файла `shcntx_`, а не что-либо
/// ещё: это единственный контейнер Синтакс-помощника.
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
    Ok(InstallationCorpora {
        version,
        syntax_context,
        platform_guides,
    })
}

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
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ru.hbk",
                "shlang_ru.hbk",
                "1cv8_ru.hbk",
                "mngbase_ru.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "ru").expect("корпуса найдены");
        assert_eq!(corpora.version, "8.3.27.2074");
        assert_eq!(
            corpora.syntax_context.len(),
            1,
            "syntax-context — только shcntx"
        );
        assert!(corpora.syntax_context[0].ends_with("shcntx_ru.hbk"));
        assert_eq!(
            corpora.platform_guides.len(),
            3,
            "остальные контейнеры — platform-guides"
        );
        // Не только количество, но и состав: перепутанное деление (например,
        // если shcntx отправить в platform_guides, а остальное — в
        // syntax_context) даёт здесь другой набор имён при том же count(3),
        // потому что в фикстуре ровно один файл начинается с "shcntx_".
        let mut guide_names: Vec<&str> = corpora
            .platform_guides
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("имя файла")
            })
            .collect();
        guide_names.sort();
        assert_eq!(
            guide_names,
            vec!["1cv8_ru.hbk", "mngbase_ru.hbk", "shlang_ru.hbk"],
            "platform-guides — все контейнеры, кроме shcntx, и ровно они"
        );
    }

    #[test]
    fn client_only_installation_reports_help_missing() {
        // Тонкий клиент: контейнеры подсистем есть, Синтакс-помощника нет.
        let dir = install("8.5.1.1451", &["chartui_ru.hbk", "ecsui_ru.hbk"]);
        let error = discover(&dir.path().join("8.5.1.1451"), "ru").expect_err("отказ");
        assert!(matches!(
            error,
            InstallationError::HelpMissingForVersion { ref version } if version == "8.5.1.1451"
        ));
    }

    #[test]
    fn absent_directory_is_not_found() {
        let dir = tempfile::tempdir().expect("временный каталог");
        assert!(matches!(
            discover(&dir.path().join("missing"), "ru"),
            Err(InstallationError::NotFound)
        ));
    }
}
