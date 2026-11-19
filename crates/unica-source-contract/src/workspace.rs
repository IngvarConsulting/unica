use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub cache_root: PathBuf,
    pub workspace_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceRoot {
    pub source_set: Option<String>,
    pub path: PathBuf,
}

/// Longest name of a workspace space, in bytes.
// resource-limit: workspace.space-name-bytes
// class: format
// unit: bytes
// scope: record
// value: 64
// resource: Длина имени пространства рабочей папки: сегмент адреса unica:// двери LSP и значение `workspace` двери MCP.
// status: unproven
// origin: #2173
// problem: Имя входит в адрес документа LSP, чья длина ограничена протоколом (lsp.wire-space-bytes).
// on-reach: Длинное имя укорачивается и получает короткий хэш пути папки.
// issue: #2173
pub const MAX_SPACE_BYTES: usize = 64;

/// The space names of the workspace folders one connection was given, in
/// their order: the folder name in lower case, or the directory name when
/// the folder has none. A name longer than [`MAX_SPACE_BYTES`] or shared by
/// several folders gets a short hash of the folder path. The one owner of
/// this naming: the LSP door names its documents' spaces with it, the MCP
/// door names the directories a call may select.
pub fn space_names(folders: &[(&str, &std::path::Path)]) -> Vec<String> {
    use std::collections::BTreeMap;
    let base: Vec<String> = folders
        .iter()
        .map(|(name, root)| space_base_name(name, root))
        .collect();
    let mut uses = BTreeMap::<&str, usize>::new();
    for name in &base {
        *uses.entry(name.as_str()).or_default() += 1;
    }
    let mut taken = BTreeMap::<String, &std::path::Path>::new();
    let mut names = Vec::with_capacity(folders.len());
    for ((_, root), name) in folders.iter().zip(&base) {
        let space = if name.len() > MAX_SPACE_BYTES || uses[name.as_str()] > 1 {
            let hash = space_path_hash(root);
            let mut suffix = format!("-{hash}");
            let mut index = 0;
            loop {
                let budget = MAX_SPACE_BYTES - suffix.len();
                let prefix = &name[..name.floor_char_boundary(budget)];
                let candidate = format!("{prefix}{suffix}");
                if !uses.contains_key(candidate.as_str())
                    && taken.get(&candidate).is_none_or(|known| known == root)
                {
                    break candidate;
                }
                index += 1;
                suffix = format!("-{hash}-{index}");
            }
        } else {
            name.clone()
        };
        taken.entry(space.clone()).or_insert(root);
        names.push(space);
    }
    names
}

fn space_base_name(name: &str, root: &std::path::Path) -> String {
    let name = name.trim();
    if name.is_empty() {
        root.file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("workspace")
            .to_lowercase()
    } else {
        name.to_lowercase()
    }
}

/// The short hash of a folder path that tells apart spaces of one name.
pub fn space_path_hash(root: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
