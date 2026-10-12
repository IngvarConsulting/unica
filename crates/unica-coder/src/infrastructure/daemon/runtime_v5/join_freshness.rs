//! Свежесть исходников для присоединения чтения (#1376).
//!
//! Присоединившийся вызов получает результат обхода, начатого раньше него.
//! Это допустимо, только если исходники, которые он прочитал бы сам, с начала
//! того обхода не менялись — ни через Unica, ни внешней правкой. Проверка
//! дешёвая: обход метаданных каталогов наборов исходников и `v8project.yaml`
//! без чтения содержимого. Любое сомнение — ошибка чтения, превышение срока
//! или числа записей, время изменения не раньше начала обхода — означает
//! «менялись», и вызов исполняется сам.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Запас на грубую отметку времени файловой системы и на отставание её
/// часов: FAT хранит время с точностью 2 с, а ядро ставит отметку по
/// огрублённым часам. Файл, изменённый в пределах запаса до начала обхода,
/// считается изменённым после него.
pub(crate) const TIMESTAMP_MARGIN: Duration = Duration::from_secs(2);
/// Предел записей одной проверки. Больше — проверка не дешёвая, вызов
/// исполняется сам.
pub(super) const MAX_CHECKED_ENTRIES: usize = 500_000;
/// Предел времени одной проверки: она идёт внутри бюджета прямого ответа
/// присоединяющегося вызова.
pub(super) const MAX_CHECK_DURATION: Duration = Duration::from_secs(2);

/// Не менялись ли исходники рабочего пространства `workspace_hint` с момента
/// `since`: ни `v8project.yaml`, ни один файл или каталог объявленных наборов
/// исходников. Служебный каталог `.build` в наборах не учитывается: в нём
/// живут собственные кеши Unica.
pub(super) fn sources_unchanged_since(workspace_hint: &str, since: SystemTime) -> bool {
    let deadline = Instant::now() + MAX_CHECK_DURATION;
    let Some(threshold) = since.checked_sub(TIMESTAMP_MARGIN) else {
        return false;
    };
    let Ok(context) =
        crate::infrastructure::workspace::discover_workspace(Some(PathBuf::from(workspace_hint)))
    else {
        return false;
    };
    let root = context.workspace_root;
    let manifest = root.join("v8project.yaml");
    match std::fs::symlink_metadata(&manifest) {
        Ok(metadata) if !older_than(&metadata, threshold) => return false,
        Ok(_) => {}
        // Без файла проекта источники не объявлены; появление файла позже
        // увидит следующая проверка по его времени.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    let Ok(source_map) = crate::infrastructure::project_sources::discover_project_source_map(&root)
    else {
        return false;
    };
    let mut budget = Budget {
        deadline,
        entries: 0,
    };
    source_map.source_sets.iter().all(|source_set| {
        let path = Path::new(&source_set.path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        tree_unchanged(&path, threshold, &mut budget)
    })
}

struct Budget {
    deadline: Instant,
    entries: usize,
}

impl Budget {
    fn spend(&mut self) -> bool {
        self.entries += 1;
        self.entries <= MAX_CHECKED_ENTRIES && Instant::now() < self.deadline
    }
}

fn tree_unchanged(root: &Path, threshold: SystemTime, budget: &mut Budget) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        if !budget.spend() {
            return false;
        }
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if older_than(&metadata, threshold) => {}
            // A source set not created yet is unchanged only while absent.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && directory == root => {
                continue;
            }
            _ => return false,
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            if !budget.spend() {
                return false;
            }
            let Ok(file_type) = entry.file_type() else {
                return false;
            };
            if file_type.is_dir() {
                if entry.file_name()
                    == std::ffi::OsStr::new(crate::infrastructure::source_roots::GENERATED_DIR_NAME)
                {
                    continue;
                }
                pending.push(entry.path());
                continue;
            }
            match std::fs::symlink_metadata(entry.path()) {
                Ok(metadata) if older_than(&metadata, threshold) => {}
                _ => return false,
            }
        }
    }
    true
}

/// Время изменения содержимого и, где платформа его отдаёт вместе
/// с метаданными, время изменения метаданных (`ctime`) раньше порога.
fn older_than(metadata: &std::fs::Metadata, threshold: SystemTime) -> bool {
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    if modified >= threshold {
        return false;
    }
    // Where the platform reports no metadata change time with the metadata
    // (Windows), only the content time is compared.
    crate::infrastructure::platform::filesystem::metadata_change_time(metadata)
        .is_none_or(|changed| changed < threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str =
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n";

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("v8project.yaml"), MANIFEST).unwrap();
        std::fs::create_dir_all(root.path().join("src/CommonModules/Общий/Ext")).unwrap();
        std::fs::write(
            root.path().join("src/CommonModules/Общий/Ext/Module.bsl"),
            "Процедура А() КонецПроцедуры\n",
        )
        .unwrap();
        root
    }

    fn hint(root: &tempfile::TempDir) -> String {
        root.path().to_string_lossy().into_owned()
    }

    /// The walk started after every file of the fixture was written: the
    /// fixture's own timestamps lie before it by more than the margin.
    fn walk_started_after_fixture() -> SystemTime {
        SystemTime::now() + TIMESTAMP_MARGIN + Duration::from_secs(1)
    }

    #[test]
    fn unchanged_sources_allow_the_join() {
        let root = workspace();
        assert!(sources_unchanged_since(
            &hint(&root),
            walk_started_after_fixture()
        ));
    }

    #[test]
    fn a_file_edited_after_the_walk_started_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let module = root.path().join("src/CommonModules/Общий/Ext/Module.bsl");
        std::fs::File::options()
            .write(true)
            .open(&module)
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn a_file_edited_within_the_margin_before_the_walk_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let module = root.path().join("src/CommonModules/Общий/Ext/Module.bsl");
        std::fs::File::options()
            .write(true)
            .open(&module)
            .unwrap()
            .set_modified(since - TIMESTAMP_MARGIN / 2)
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn an_edited_project_file_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        std::fs::File::options()
            .write(true)
            .open(root.path().join("v8project.yaml"))
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn the_unica_cache_directory_inside_a_source_set_is_not_a_source() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let cache = root.path().join("src/.build/unica");
        std::fs::create_dir_all(&cache).unwrap();
        let index = cache.join("index.bin");
        std::fs::write(&index, b"cache").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&index)
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(sources_unchanged_since(&hint(&root), since));
    }
}
