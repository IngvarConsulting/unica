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
//!
//! Локали двух корпусов разрешаются по отдельности, потому что вендор
//! поставляет их по-разному. Измерено на 8.3.27.2074 и 8.5.4.1306: справка
//! подсистем есть в 24 локалях (`_ru`, `_root`, `_de`, `_fr`, `_zh`, …), а
//! Синтакс-помощник — ровно в двух, `shcntx_ru.hbk` и `shcntx_root.hbk`.
//! `_root` — английский контейнер: те же 25 511 страниц, но заголовками
//! «ClientApplicationWindow.GetURL» вместо
//! «ОкноКлиентскогоПриложения.ПолучитьНавигационнуюСсылку (…)». Файла
//! `shcntx_en.hbk` не существует ни в одной из семи установок машины.
//!
//! Отсюда правило: запрошенная локаль, если корпус в ней есть, иначе
//! `root`, иначе `ru`, иначе первая по алфавиту. Требовать
//! `shcntx_<язык>.hbk` буквально значило бы отвечать отказом на всё, кроме
//! `ru` и `root`, — включая `en`, для которого английский контейнер лежит
//! рядом. Разрешение по корпусам, а не одно на вызов: запрос на `de` тогда
//! читает немецкую справку подсистем и английский Синтакс-помощник, а не
//! теряет немецкую справку из-за отсутствия немецкого `shcntx`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Режимы отказа разделены намеренно. Задача поставщика — говорить, что именно
/// не так, а не отвечать одинаково на «каталога нет», «каталог не читается» и
/// «версию не вывести из пути»: вызывающий по-разному их сообщает.
#[derive(Debug)]
pub enum InstallationError {
    /// Каталога установки нет.
    NotFound,
    /// Из пути не выводится версия — у корня нет последнего сегмента.
    VersionUndetermined,
    /// Каталог существует, но перечислить его не вышло: права, не каталог,
    /// ошибка ввода-вывода. Текст ошибки сохраняется для диагностики.
    Unreadable { detail: String },
    /// Установка есть, но Синтакс-помощника в ней нет — клиентская поставка.
    HelpMissingForVersion { version: String },
}

/// Контейнеры одного корпуса и локаль, в которой они нашлись. Локаль хранится
/// рядом с файлами, а не одна на установку: корпуса разрешают её по
/// отдельности (см. заголовок модуля), и ответ обязан называть ту локаль,
/// которая на самом деле ответила, — иначе запрос на `en`, отвеченный
/// английским `_root`, неотличим от отвеченного русским `_ru`.
#[derive(Debug, Clone)]
pub struct CorpusContainers {
    pub language: String,
    pub containers: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct InstallationCorpora {
    pub version: String,
    pub syntax_context: CorpusContainers,
    pub platform_guides: CorpusContainers,
}

/// Локаль, в которой корпус будет прочитан. Запрошенная, если корпус в ней
/// есть; иначе `root` — так вендор называет английский контейнер; иначе `ru`;
/// иначе первая по алфавиту, чтобы выбор не зависел от порядка обхода
/// каталога. Порядок именно такой, а не «первая по алфавиту» сразу: на полной
/// установке первой по алфавиту оказывается `ar`, и запрос на `de` получал бы
/// арабскую справку.
fn resolve_language(available: &BTreeSet<String>, requested: &str) -> Option<String> {
    for candidate in [requested, "root", "ru"] {
        if available.contains(candidate) {
            return Some(candidate.to_string());
        }
    }
    available.iter().next().cloned()
}

/// Контейнеры `.hbk` одного каталога, разложенные на Синтакс-помощник и
/// справку подсистем. Имя контейнера — `<основа>_<локаль>.hbk`, и локаль
/// отделяется по ПЕРВОМУ подчёркиванию: подчёркивание есть в локали
/// (`1cv8_pt_BR.hbk`), но ни в одной из 38 основ установки его нет, поэтому
/// `rsplit_once` дал бы там локаль `BR` и потерял бы бразильский корпус в
/// пользу несуществующей локали `BR` с основой `1cv8_pt`.
type LocalizedContainers = (Vec<(String, PathBuf)>, Vec<(String, PathBuf)>);

fn hbk_containers(directory: &Path) -> Result<LocalizedContainers, InstallationError> {
    let entries = std::fs::read_dir(directory).map_err(|error| InstallationError::Unreadable {
        detail: error.to_string(),
    })?;
    let mut syntax: Vec<(String, PathBuf)> = Vec::new();
    let mut guides: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".hbk") else {
            continue;
        };
        let Some((base, locale)) = stem.split_once('_') else {
            continue;
        };
        if base == "shcntx" {
            syntax.push((locale.to_string(), path));
        } else {
            guides.push((locale.to_string(), path));
        }
    }
    Ok((syntax, guides))
}

/// Разбирает каталог одной установленной версии (`root` — сам каталог версии,
/// например `.../8.3.27.2074`) на два корпуса контейнеров `.hbk`. Признак
/// разделения — префикс имени файла `shcntx_`, а не что-либо ещё: это
/// единственный контейнер Синтакс-помощника. Контейнеры ищутся в корне
/// версии, а при его пустоте — в подкаталоге `bin` (раскладка Windows).
pub fn discover(root: &Path, language: &str) -> Result<InstallationCorpora, InstallationError> {
    if !root.exists() {
        return Err(InstallationError::NotFound);
    }
    let version = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(InstallationError::VersionUndetermined)?
        .to_string();
    let (mut syntax, mut guides) = hbk_containers(root)?;
    // Раскладка Windows кладёт контейнеры в `bin` каталога версии, рядом с
    // исполняемыми файлами, а не в его корень, как Linux и macOS. Запасной
    // взгляд в `bin` — только когда корень не нёс НИ ОДНОГО `.hbk`: смешение
    // двух раскладок в одной установке не встречается, а объединение списков
    // удвоило бы контейнеры на гипотетическом гибриде. Нечитаемый `bin` при
    // пустом корне — та же поломка установки, что и нечитаемый корень.
    if syntax.is_empty() && guides.is_empty() {
        let bin = root.join("bin");
        if bin.is_dir() {
            (syntax, guides) = hbk_containers(&bin)?;
        }
    }
    let syntax_languages: BTreeSet<String> =
        syntax.iter().map(|(locale, _)| locale.clone()).collect();
    let Some(syntax_language) = resolve_language(&syntax_languages, language) else {
        // Клиентская поставка несёт справку подсистем, но не Синтакс-помощник
        // ни в одной локали. Подставлять корпус соседней версии запрещено.
        return Err(InstallationError::HelpMissingForVersion { version });
    };
    let guide_languages: BTreeSet<String> =
        guides.iter().map(|(locale, _)| locale.clone()).collect();
    // Локаль справки подсистем разрешается независимо: у неё локалей 24, а у
    // Синтакс-помощника две, и общая на вызов локаль отняла бы у запроса на
    // `de` немецкую справку подсистем, которая на диске есть.
    let guides_language =
        resolve_language(&guide_languages, language).unwrap_or_else(|| language.to_string());
    let pick = |list: Vec<(String, PathBuf)>, wanted: &str| -> Vec<PathBuf> {
        let mut chosen: Vec<PathBuf> = list
            .into_iter()
            .filter(|(locale, _)| locale == wanted)
            .map(|(_, path)| path)
            .collect();
        chosen.sort();
        chosen
    };
    Ok(InstallationCorpora {
        version,
        syntax_context: CorpusContainers {
            containers: pick(syntax, &syntax_language),
            language: syntax_language,
        },
        platform_guides: CorpusContainers {
            containers: pick(guides, &guides_language),
            language: guides_language,
        },
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
            corpora.syntax_context.containers.len(),
            1,
            "syntax-context — только shcntx"
        );
        assert!(corpora.syntax_context.containers[0].ends_with("shcntx_ru.hbk"));
        assert_eq!(
            corpora.platform_guides.containers.len(),
            3,
            "остальные контейнеры — platform-guides"
        );
        // Не только количество, но и состав: перепутанное деление (например,
        // если shcntx отправить в platform_guides, а остальное — в
        // syntax_context) даёт здесь другой набор имён при том же count(3),
        // потому что в фикстуре ровно один файл начинается с "shcntx_".
        let mut guide_names: Vec<&str> = corpora
            .platform_guides
            .containers
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

    #[test]
    fn root_without_a_last_segment_reports_version_undetermined() {
        // У корня файловой системы нет последнего сегмента, значит версию из
        // пути не вывести. Это не то же самое, что отсутствие каталога, и
        // сообщается отдельно.
        assert!(matches!(
            discover(std::path::Path::new("/"), "ru"),
            Err(InstallationError::VersionUndetermined)
        ));
    }

    #[test]
    fn path_that_is_a_file_is_reported_as_unreadable() {
        // Каталог существует по имени, но перечислить его нельзя: это не
        // «установки нет», а «с установкой что-то не так».
        let dir = tempfile::tempdir().expect("временный каталог");
        let path = dir.path().join("8.3.27.2074");
        std::fs::write(&path, b"not a directory").expect("файл вместо каталога");
        let error = discover(&path, "ru").expect_err("отказ");
        assert!(
            matches!(error, InstallationError::Unreadable { ref detail } if !detail.is_empty()),
            "ожидался Unreadable с текстом ошибки, получено {error:?}"
        );
    }

    /// Имена контейнеров корпуса, отсортированные: сравнивать состав, а не
    /// одни количества.
    fn names(corpus: &CorpusContainers) -> Vec<String> {
        let mut out: Vec<String> = corpus
            .containers
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("имя файла")
                    .to_string()
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn mixed_language_installation_filters_by_language() {
        // Фильтр по языку — часть контракта: расширение суффикса до любого
        // `.hbk` втянуло бы чужую локаль, и этот тест обязан это поймать.
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ru.hbk",
                "shcntx_en.hbk",
                "1cv8_ru.hbk",
                "1cv8_en.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "ru").expect("корпуса найдены");
        assert_eq!(names(&corpora.syntax_context), vec!["shcntx_ru.hbk"]);
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_ru.hbk"]);
        assert_eq!(corpora.syntax_context.language, "ru");
    }

    /// Зеркало предыдущего на ДРУГОЙ локали. Без него `discover`, всегда
    /// читающий `ru`, проходит `mixed_language_installation_filters_by_language`
    /// незамеченным: там запрошенная локаль и есть `ru`.
    #[test]
    fn a_requested_locale_that_is_installed_is_the_one_that_answers() {
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ru.hbk",
                "shcntx_en.hbk",
                "1cv8_ru.hbk",
                "1cv8_en.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "en").expect("корпуса найдены");
        assert_eq!(names(&corpora.syntax_context), vec!["shcntx_en.hbk"]);
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_en.hbk"]);
        assert_eq!(corpora.syntax_context.language, "en");
        assert_eq!(corpora.platform_guides.language, "en");
    }

    /// Реальная установка 8.3.27.2074 несёт Синтакс-помощник ровно в двух
    /// локалях — `shcntx_ru.hbk` и `shcntx_root.hbk` (последний английский), —
    /// а `shcntx_en.hbk` не существует. Требовать `shcntx_<язык>.hbk` буквально
    /// значит отвечать `VersionMissing` на `language: "en"` и ронять весь вызов,
    /// хотя английский контейнер лежит рядом.
    #[test]
    fn a_locale_without_its_own_syntax_container_falls_back_instead_of_refusing() {
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ru.hbk",
                "shcntx_root.hbk",
                "1cv8_ru.hbk",
                "1cv8_root.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "en")
            .expect("отсутствующая локаль обязана подменяться, а не отказывать");
        assert_eq!(names(&corpora.syntax_context), vec!["shcntx_root.hbk"]);
        assert_eq!(
            corpora.syntax_context.language, "root",
            "ответ обязан называть локаль, которая ответила, а не запрошенную"
        );
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_root.hbk"]);
        assert_eq!(corpora.platform_guides.language, "root");
    }

    /// Локали корпусов разрешаются по отдельности. Немецкая справка подсистем
    /// на установке есть (`1cv8_de.hbk`), немецкого Синтакс-помощника нет —
    /// одна локаль на вызов отняла бы у запроса на `de` немецкие руководства.
    #[test]
    fn each_corpus_resolves_its_own_locale() {
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ru.hbk",
                "shcntx_root.hbk",
                "1cv8_ru.hbk",
                "1cv8_root.hbk",
                "1cv8_de.hbk",
                "frntend_de.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "de").expect("корпуса найдены");
        assert_eq!(
            names(&corpora.syntax_context),
            vec!["shcntx_root.hbk"],
            "немецкого Синтакс-помощника нет — отвечает английский"
        );
        assert_eq!(corpora.syntax_context.language, "root");
        assert_eq!(
            names(&corpora.platform_guides),
            vec!["1cv8_de.hbk", "frntend_de.hbk"],
            "немецкая справка подсистем есть и обязана ответить именно она"
        );
        assert_eq!(corpora.platform_guides.language, "de");
    }

    /// Подмена идёт не «первой попавшейся» локалью: на полной установке первой
    /// по алфавиту оказывается `ar`, и запрос на `de` получил бы арабскую
    /// справку. Предпочтение — `root`, затем `ru`.
    #[test]
    fn the_fallback_prefers_root_over_the_alphabetically_first_locale() {
        let dir = install(
            "8.3.27.2074",
            &[
                "shcntx_ar.hbk",
                "shcntx_root.hbk",
                "shcntx_ru.hbk",
                "1cv8_ar.hbk",
                "1cv8_root.hbk",
            ],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "de").expect("корпуса найдены");
        assert_eq!(names(&corpora.syntax_context), vec!["shcntx_root.hbk"]);
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_root.hbk"]);
    }

    /// Локаль отделяется по ПЕРВОМУ подчёркиванию, а не по последнему:
    /// `1cv8_pt_BR.hbk` — это локаль `pt_BR`, а не `BR` при основе `1cv8_pt`.
    /// Разделение с конца потеряло бы бразильский корпус для запроса `pt_BR`.
    #[test]
    fn a_locale_that_contains_an_underscore_is_not_split_in_half() {
        let dir = install(
            "8.3.27.2074",
            &["shcntx_ru.hbk", "1cv8_pt_BR.hbk", "1cv8_ru.hbk"],
        );
        let corpora = discover(&dir.path().join("8.3.27.2074"), "pt_BR").expect("корпуса найдены");
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_pt_BR.hbk"]);
        assert_eq!(corpora.platform_guides.language, "pt_BR");
    }

    /// Раскладка Windows: исполняемые файлы и контейнеры справки лежат в
    /// подкаталоге `bin` каталога версии (`C:\Program Files\1cv8\<версия>\bin`),
    /// а не в его корне, как на Linux и macOS. Перечисление одного лишь корня
    /// версии на такой установке не находит ни одного `.hbk`, и полная
    /// поставка отвечала бы ложным «Синтакс-помощника нет ни в одной локали».
    #[test]
    fn containers_in_the_bin_subdirectory_are_discovered() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let root = dir.path().join("8.3.27.2074");
        std::fs::create_dir_all(root.join("bin")).expect("каталог bin");
        for name in ["shcntx_ru.hbk", "1cv8_ru.hbk"] {
            std::fs::write(root.join("bin").join(name), b"stub").expect("файл");
        }
        // Рядом с bin лежат служебные каталоги установки — они не корпуса.
        std::fs::create_dir_all(root.join("docs")).expect("служебный каталог");

        let corpora = discover(&root, "ru").expect("корпуса найдены в bin");
        assert_eq!(corpora.version, "8.3.27.2074");
        assert_eq!(names(&corpora.syntax_context), vec!["shcntx_ru.hbk"]);
        assert_eq!(names(&corpora.platform_guides), vec!["1cv8_ru.hbk"]);
    }

    /// Клиентская поставка не несёт Синтакс-помощника ВОВСЕ, ни в одной
    /// локали, — и только это по-прежнему отказ. Проверяется на языке, которого
    /// в каталоге нет, чтобы отказ нельзя было объяснить одной лишь локалью.
    #[test]
    fn client_only_installation_still_refuses_for_every_locale() {
        let dir = install("8.5.1.1451", &["chartui_ru.hbk", "chartui_root.hbk"]);
        for language in ["ru", "en", "de"] {
            let error = discover(&dir.path().join("8.5.1.1451"), language).expect_err("отказ");
            assert!(
                matches!(
                    error,
                    InstallationError::HelpMissingForVersion { ref version }
                        if version == "8.5.1.1451"
                ),
                "язык {language}: ожидался HelpMissingForVersion, получено {error:?}"
            );
        }
    }
}
