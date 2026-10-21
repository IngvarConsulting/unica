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
pub const TIMESTAMP_MARGIN: Duration = Duration::from_secs(2);
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
pub(super) fn sources_unchanged_since(
    workspace_hint: &str,
    since: SystemTime,
    discover_workspace_root: &dyn Fn(&str) -> Result<PathBuf, String>,
    discover_source_set_paths: &dyn Fn(&Path) -> Result<Vec<String>, String>,
) -> bool {
    let deadline = Instant::now() + MAX_CHECK_DURATION;
    let Some(threshold) = since.checked_sub(TIMESTAMP_MARGIN) else {
        return false;
    };
    let Ok(root) = discover_workspace_root(workspace_hint) else {
        return false;
    };
    let manifest = root.join("v8project.yaml");
    match std::fs::symlink_metadata(&manifest) {
        Ok(metadata) if !older_than(&metadata, threshold) => return false,
        Ok(_) => {}
        // Без файла проекта источники не объявлены; появление файла позже
        // увидит следующая проверка по его времени.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    let Ok(source_set_paths) = discover_source_set_paths(&root) else {
        return false;
    };
    let mut budget = Budget {
        deadline,
        entries: 0,
    };
    source_set_paths.iter().all(|source_path| {
        let path = Path::new(source_path);
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
                    == std::ffi::OsStr::new(unica_workspace::paths::GENERATED_DIR_NAME)
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
    unica_workspace::paths::metadata_change_time(metadata).is_none_or(|changed| changed < threshold)
}
