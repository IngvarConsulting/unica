use crate::application::v13::find::{FindDocument, FindFact, FindFactKind, FindIndex};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::SourceSetKind;
use crate::infrastructure::metadata_kinds::metadata_kind_by_directory;
use crate::infrastructure::platform::filesystem::{
    RetainedChildCapability, RetainedDirectoryCapability,
};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const MAX_SOURCE_SETS: usize = 64;
const DEFAULT_MAX_DOCUMENTS: usize = 65_536;
const DEFAULT_MAX_FACT_BYTES: usize = 16 * 1024 * 1024;
/// Enough of a descriptor head to carry `Name` and `Synonym`. The directory
/// never needs the rest of the file.
const DESCRIPTOR_HEAD_BYTES: usize = 8 * 1024;
const MAX_COLLECTION_ENTRIES: usize = 65_536;
/// Physical child families an object owns as its own files or directories.
const NESTED_FAMILIES: [(&str, NodeKind); 3] = [
    ("Forms", NodeKind::Form),
    ("Templates", NodeKind::Template),
    ("Commands", NodeKind::Command),
];

/// One admitted source-set root. The directory is read through the retained
/// no-follow capability the actor owns; no path is reopened by name.
pub(crate) struct LayoutFindSource<'a> {
    name: &'a str,
    kind: SourceSetKind,
    root: &'a RetainedDirectoryCapability,
}

impl<'a> LayoutFindSource<'a> {
    pub(crate) const fn new(
        name: &'a str,
        kind: SourceSetKind,
        root: &'a RetainedDirectoryCapability,
    ) -> Self {
        Self { name, kind, root }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindBuildError {
    code: &'static str,
    message: String,
}

impl FindBuildError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for FindBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Builds the two-way directory between qualified logical addresses and where
/// objects live in the source layout. It reads that layout only: no typed
/// projection, no module source, no revision lease.
#[derive(Debug)]
pub(crate) struct WorkspaceFindDirectoryBuilder {
    max_documents: usize,
    max_total_fact_bytes: usize,
}

impl Default for WorkspaceFindDirectoryBuilder {
    fn default() -> Self {
        Self::with_limits(DEFAULT_MAX_DOCUMENTS, DEFAULT_MAX_FACT_BYTES)
    }
}

struct DirectoryBuild {
    documents: Vec<FindDocument>,
    fact_bytes: usize,
}

impl WorkspaceFindDirectoryBuilder {
    fn with_limits(max_documents: usize, max_total_fact_bytes: usize) -> Self {
        Self {
            max_documents,
            max_total_fact_bytes,
        }
    }

    #[cfg(test)]
    fn with_document_limit(max_documents: usize) -> Self {
        Self::with_limits(max_documents, DEFAULT_MAX_FACT_BYTES)
    }

    pub(crate) fn build(
        &self,
        sources: &[LayoutFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, FindBuildError> {
        if sources.len() > MAX_SOURCE_SETS {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find source-set count exceeds the bounded workspace limit",
            ));
        }
        let mut build = DirectoryBuild {
            documents: Vec::new(),
            fact_bytes: 0,
        };
        for source in sources {
            find_checkpoint(deadline, cancellation)?;
            self.add_source(source, &mut build, deadline, cancellation)?;
        }
        Ok(FindIndex::new(build.documents))
    }

    fn add_source(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        if matches!(
            source.kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        ) {
            return self.add_external_source(source, build, deadline, cancellation);
        }
        let configuration = Path::new("Configuration.xml");
        if let Some(head) = read_descriptor_head(source.root, configuration) {
            let (name, synonym) = descriptor_identity(&head);
            self.push(
                build,
                source,
                &format!("{}:Configuration", source.name),
                NodeKind::Configuration.as_str(),
                name.as_deref().unwrap_or("Configuration"),
                synonym.as_deref(),
                configuration,
            )?;
        }
        for entry in immediate_names(source.root, deadline, cancellation)? {
            find_checkpoint(deadline, cancellation)?;
            let Some(directory) = entry.to_str() else {
                continue;
            };
            let Some(layout) = metadata_kind_by_directory(directory) else {
                continue;
            };
            let Some(RetainedChildCapability::Directory(collection)) =
                retain_child(source.root, &entry)
            else {
                continue;
            };
            for owner in immediate_names(&collection, deadline, cancellation)? {
                find_checkpoint(deadline, cancellation)?;
                let Some(owner_name) = owner.to_str() else {
                    continue;
                };
                match retain_child(&collection, &owner) {
                    Some(RetainedChildCapability::RegularFile(_)) => {
                        let Some(stem) = owner_name.strip_suffix(".xml") else {
                            continue;
                        };
                        let relative = PathBuf::from(directory).join(owner_name);
                        // A file whose name looks like an object is not one:
                        // only a descriptor that declares the expected owner
                        // element and name enters the directory.
                        let Some(head) = read_descriptor_head(source.root, &relative) else {
                            continue;
                        };
                        if !declares_owner(&head, layout.tag, stem) {
                            continue;
                        }
                        let synonym = descriptor_identity(&head).1;
                        self.push(
                            build,
                            source,
                            &format!("{}:{}.{stem}", source.name, layout.tag),
                            layout.tag,
                            stem,
                            synonym.as_deref(),
                            &relative,
                        )?;
                    }
                    Some(RetainedChildCapability::Directory(owner_root)) => {
                        self.add_nested_families(
                            source,
                            build,
                            &owner_root,
                            layout.tag,
                            owner_name,
                            &PathBuf::from(directory).join(owner_name),
                            deadline,
                            cancellation,
                        )?;
                    }
                    _ => continue,
                }
            }
        }
        Ok(())
    }

    /// An external processor or report keeps its single owner descriptor at
    /// the root of the source set.
    fn add_external_source(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        let kind = if source.kind == SourceSetKind::ExternalProcessor {
            NodeKind::ExternalDataProcessor
        } else {
            NodeKind::ExternalReport
        };
        for entry in immediate_names(source.root, deadline, cancellation)? {
            find_checkpoint(deadline, cancellation)?;
            let Some(entry_name) = entry.to_str() else {
                continue;
            };
            let Some(stem) = entry_name.strip_suffix(".xml") else {
                continue;
            };
            if !matches!(
                retain_child(source.root, &entry),
                Some(RetainedChildCapability::RegularFile(_))
            ) {
                continue;
            }
            let relative = PathBuf::from(entry_name);
            // A Designer dump keeps `ConfigDumpInfo.xml` next to the owner
            // descriptor; only a file that declares the expected owner element
            // is an object.
            let Some(head) = read_descriptor_head(source.root, &relative) else {
                continue;
            };
            if !declares_owner(&head, kind.as_str(), stem) {
                continue;
            }
            let synonym = descriptor_identity(&head).1;
            self.push(
                build,
                source,
                &format!("{}:{}.{stem}", source.name, kind.as_str()),
                kind.as_str(),
                stem,
                synonym.as_deref(),
                &relative,
            )?;
            if let Some(RetainedChildCapability::Directory(owner_root)) =
                retain_child(source.root, OsStr::new(stem))
            {
                self.add_nested_families(
                    source,
                    build,
                    &owner_root,
                    kind.as_str(),
                    stem,
                    &PathBuf::from(stem),
                    deadline,
                    cancellation,
                )?;
            }
        }
        Ok(())
    }

    /// Forms and templates own a descriptor file; a command owns only its
    /// directory. Both are addressed straight from the layout.
    #[allow(clippy::too_many_arguments)]
    fn add_nested_families(
        &self,
        source: &LayoutFindSource<'_>,
        build: &mut DirectoryBuild,
        owner_root: &RetainedDirectoryCapability,
        owner_kind: &str,
        owner_name: &str,
        owner_relative: &Path,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), FindBuildError> {
        for (family_directory, family_kind) in NESTED_FAMILIES {
            find_checkpoint(deadline, cancellation)?;
            let Some(RetainedChildCapability::Directory(family)) =
                retain_child(owner_root, OsStr::new(family_directory))
            else {
                continue;
            };
            for entry in immediate_names(&family, deadline, cancellation)? {
                find_checkpoint(deadline, cancellation)?;
                let Some(entry_name) = entry.to_str() else {
                    continue;
                };
                let (child_name, relative) = match retain_child(&family, &entry) {
                    Some(RetainedChildCapability::RegularFile(_)) => {
                        let Some(stem) = entry_name.strip_suffix(".xml") else {
                            continue;
                        };
                        (
                            stem.to_string(),
                            owner_relative.join(family_directory).join(entry_name),
                        )
                    }
                    // A command has no descriptor file: its directory carries
                    // only the module, and the name is the directory itself.
                    Some(RetainedChildCapability::Directory(_))
                        if family_kind == NodeKind::Command =>
                    {
                        (
                            entry_name.to_string(),
                            owner_relative.join(family_directory).join(entry_name),
                        )
                    }
                    _ => continue,
                };
                let synonym = if family_kind == NodeKind::Command {
                    None
                } else {
                    let Some(head) = read_descriptor_head(source.root, &relative) else {
                        continue;
                    };
                    if !declares_owner(&head, family_kind.as_str(), &child_name) {
                        continue;
                    }
                    descriptor_identity(&head).1
                };
                self.push(
                    build,
                    source,
                    &format!(
                        "{}:{owner_kind}.{owner_name}.{}.{child_name}",
                        source.name,
                        family_kind.as_str()
                    ),
                    family_kind.as_str(),
                    &child_name,
                    synonym.as_deref(),
                    &relative,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        build: &mut DirectoryBuild,
        source: &LayoutFindSource<'_>,
        at: &str,
        kind: &str,
        name: &str,
        synonym: Option<&str>,
        relative: &Path,
    ) -> Result<(), FindBuildError> {
        if QualifiedAddress::parse(at).is_err() {
            // A directory entry whose name is not addressable is not an
            // object; the layout simply does not describe it.
            return Ok(());
        }
        let _ = source;
        let path = path_text(relative);
        let mut facts = vec![
            FindFact::new(FindFactKind::Name, name),
            FindFact::new(FindFactKind::ExportPath, &path),
        ];
        if let Some(synonym) = synonym.filter(|value| !value.is_empty() && *value != name) {
            facts.push(FindFact::new(FindFactKind::Synonym, synonym));
        }
        let title = synonym.filter(|value| !value.is_empty()).unwrap_or(name);
        let document = FindDocument::new(at, kind, title, facts).with_path(path);
        if build.documents.len() == self.max_documents {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find directory exceeds the bounded workspace entry limit",
            ));
        }
        let next_total = build
            .fact_bytes
            .saturating_add(document.estimated_identity_bytes());
        if next_total > self.max_total_fact_bytes {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find directory exceeds the bounded workspace byte budget",
            ));
        }
        build.fact_bytes = next_total;
        build.documents.push(document);
        Ok(())
    }
}

fn immediate_names(
    directory: &RetainedDirectoryCapability,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<Vec<std::ffi::OsString>, FindBuildError> {
    directory
        .read_immediate_names_bounded(MAX_COLLECTION_ENTRIES, || {
            find_checkpoint(deadline, cancellation)
                .map_err(|error| std::io::Error::other(error.to_string()))
        })
        .map_err(|error| {
            if cancellation.is_cancelled() {
                FindBuildError::new("cancelled", "find directory build was cancelled")
            } else if deadline.remaining().is_zero() {
                FindBuildError::new("provider_deadline", "find directory build deadline elapsed")
            } else {
                FindBuildError::new(
                    "provider_unavailable",
                    format!("find could not read the source layout: {error}"),
                )
            }
        })
}

fn retain_child(
    directory: &RetainedDirectoryCapability,
    name: &OsStr,
) -> Option<RetainedChildCapability> {
    directory.retain_immediate_child_nofollow(name).ok()
}

fn read_descriptor_head(root: &RetainedDirectoryCapability, relative: &Path) -> Option<Vec<u8>> {
    root.read_relative_regular_bounded(relative, DESCRIPTOR_HEAD_BYTES)
        .ok()
}

/// Reads `Name` and the first localized `Synonym` out of a descriptor head
/// without parsing the document: a malformed or truncated descriptor simply
/// contributes no synonym instead of failing the directory.
/// Whether a descriptor head declares the expected owner element and name.
fn declares_owner(head: &[u8], kind: &str, name: &str) -> bool {
    let text = String::from_utf8_lossy(head);
    let opens = text.contains(&format!("<{kind} ")) || text.contains(&format!("<{kind}>"));
    opens && between(&text, "<Name>", "</Name>").is_some_and(|declared| declared == name)
}

fn descriptor_identity(head: &[u8]) -> (Option<String>, Option<String>) {
    let text = String::from_utf8_lossy(head);
    let name = between(&text, "<Name>", "</Name>").map(str::to_string);
    let synonym = text.find("<Synonym>").and_then(|start| {
        let rest = &text[start..];
        between(rest, "<v8:content>", "</v8:content>").map(str::to_string)
    });
    (name, synonym)
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim()).filter(|value| !value.is_empty())
}

fn path_text(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn find_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), FindBuildError> {
    if cancellation.is_cancelled() {
        return Err(FindBuildError::new(
            "cancelled",
            "find directory build was cancelled",
        ));
    }
    if deadline.remaining().is_zero() {
        return Err(FindBuildError::new(
            "provider_deadline",
            "find directory build deadline elapsed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LayoutFindSource, WorkspaceFindDirectoryBuilder};
    use crate::application::v13::find::FindRequest;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_sources::SourceSetKind;
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn owner(name: &str, kind: &str, synonym: &str, children: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{synonym}</v8:content></v8:item></Synonym></Properties>
    <ChildObjects>{children}</ChildObjects>
  </{kind}>
</MetaDataObject>"#
        )
    }

    struct Fixture {
        _root: tempfile::TempDir,
        source: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().canonicalize().unwrap().join("src");
            write(
                &source.join("Configuration.xml"),
                &owner(
                    "Магазин",
                    "Configuration",
                    "Магазин",
                    "<Catalog>Валюты</Catalog>",
                ),
            );
            write(
                &source.join("Catalogs/Валюты.xml"),
                &owner(
                    "Валюты",
                    "Catalog",
                    "Валюты и курсы",
                    "<Form>ФормаЭлемента</Form>",
                ),
            );
            write(
                &source.join("Catalogs/Валюты/Forms/ФормаЭлемента.xml"),
                &owner("ФормаЭлемента", "Form", "Форма элемента", ""),
            );
            write(
                &source.join("Catalogs/Валюты/Templates/Печать.xml"),
                &owner("Печать", "Template", "Печатная форма", ""),
            );
            write(
                &source.join("Catalogs/Валюты/Commands/Обновить/Ext/CommandModule.bsl"),
                "&AtClient\nProcedure CommandProcessing()\nEndProcedure\n",
            );
            // Module bodies must not be read by the directory at all.
            write(
                &source.join("Catalogs/Валюты/Ext/ObjectModule.bsl"),
                "Procedure СекретныйМетод()\nEndProcedure\n",
            );
            Self {
                _root: root,
                source,
            }
        }

        fn directory(&self) -> crate::application::v13::find::FindIndex {
            let root = RetainedDirectoryCapability::open(&self.source).unwrap();
            WorkspaceFindDirectoryBuilder::default()
                .build(
                    &[LayoutFindSource::new(
                        "main",
                        SourceSetKind::Configuration,
                        &root,
                    )],
                    ProviderDeadline::from_budget(Duration::from_secs(7)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }

    fn single(index: &crate::application::v13::find::FindIndex, query: &str) -> (String, String) {
        let found = index.find(FindRequest::new(query).unwrap());
        let candidate = found
            .candidates()
            .first()
            .unwrap_or_else(|| panic!("{query}: {found:?}"));
        assert!(!found.is_nearest(), "{query}: {found:?}");
        (
            candidate.at().to_string(),
            candidate.path().unwrap_or_default().to_string(),
        )
    }

    #[test]
    fn a_name_resolves_to_the_address_and_the_file_that_carries_it() {
        let index = Fixture::new().directory();
        for (query, at, path) in [
            ("Валюты", "main:Catalog.Валюты", "Catalogs/Валюты.xml"),
            (
                "main:Catalog.Валюты",
                "main:Catalog.Валюты",
                "Catalogs/Валюты.xml",
            ),
            (
                "ФормаЭлемента",
                "main:Catalog.Валюты.Form.ФормаЭлемента",
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
            ),
            (
                "Печать",
                "main:Catalog.Валюты.Template.Печать",
                "Catalogs/Валюты/Templates/Печать.xml",
            ),
            (
                "Обновить",
                "main:Catalog.Валюты.Command.Обновить",
                "Catalogs/Валюты/Commands/Обновить",
            ),
            ("Магазин", "main:Configuration", "Configuration.xml"),
        ] {
            assert_eq!(single(&index, query), (at.to_string(), path.to_string()));
        }
    }

    #[test]
    fn a_file_path_resolves_back_to_its_object_address() {
        let index = Fixture::new().directory();
        for (query, at) in [
            ("Catalogs/Валюты.xml", "main:Catalog.Валюты"),
            (
                "Catalogs/Валюты/Forms/ФормаЭлемента.xml",
                "main:Catalog.Валюты.Form.ФормаЭлемента",
            ),
            // The caller pastes what the shell gave them: the stored path is
            // the tail of an absolute or workspace-relative path.
            (
                "/home/user/project/src/Catalogs/Валюты.xml",
                "main:Catalog.Валюты",
            ),
            ("src/Catalogs/Валюты.xml", "main:Catalog.Валюты"),
        ] {
            assert_eq!(single(&index, query).0, at, "{query}");
        }
    }

    #[test]
    fn a_synonym_resolves_to_its_object() {
        let index = Fixture::new().directory();
        assert_eq!(single(&index, "Валюты и курсы").0, "main:Catalog.Валюты");
    }

    #[test]
    fn the_directory_holds_objects_and_never_code_symbols_or_inner_nodes() {
        let index = Fixture::new().directory();
        for query in [
            "СекретныйМетод",
            "main:Catalog.Валюты.Module.Object",
            "main:Catalog.Валюты.Attribute.Код",
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "the directory answered a non-object query {query}: {found:?}"
            );
        }
    }

    #[test]
    fn the_directory_refuses_to_grow_past_its_entry_bound() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::with_document_limit(1)
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "provider_limit_exceeded");
    }

    #[test]
    fn the_directory_observes_cancellation() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &cancellation,
            )
            .unwrap_err();
        assert_eq!(error.code(), "cancelled");
    }

    #[test]
    fn an_external_root_publishes_its_owner_and_never_the_dump_sidecar() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap().join("processor");
        write(
            &source.join("Импорт.xml"),
            &owner(
                "Импорт",
                "ExternalDataProcessor",
                "Импорт данных",
                "<Form>Основная</Form>",
            ),
        );
        write(
            &source.join("Импорт/Forms/Основная.xml"),
            &owner("Основная", "Form", "Основная форма", ""),
        );
        // A Designer dump keeps this sidecar beside the owner descriptor.
        write(
            &source.join("ConfigDumpInfo.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><ConfigDumpInfo xmlns="http://v8.1c.ru/8.3/xcf/dumpinfo" format="Hierarchical" version="2.20"/>"#,
        );
        let retained = RetainedDirectoryCapability::open(&source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "processor",
                    SourceSetKind::ExternalProcessor,
                    &retained,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            single(&index, "Импорт"),
            (
                "processor:ExternalDataProcessor.Импорт".to_string(),
                "Импорт.xml".to_string()
            )
        );
        assert_eq!(
            single(&index, "Основная").0,
            "processor:ExternalDataProcessor.Импорт.Form.Основная"
        );
        for query in ["ConfigDumpInfo", "ConfigDumpInfo.xml"] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "the dump sidecar became an object: {found:?}"
            );
        }
        // An external source set has no configuration root, so nothing may
        // answer its export path.
        let fabricated = index.find(FindRequest::new("Configuration.xml").unwrap());
        assert!(
            fabricated
                .candidates()
                .iter()
                .all(|candidate| candidate.reason() != "exportPath"),
            "an external source set advertised a configuration export path: {fabricated:?}"
        );
    }

    #[test]
    fn the_directory_observes_its_operation_deadline() {
        let fixture = Fixture::new();
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let error = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::ZERO),
                &CancellationToken::new(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "provider_deadline");
    }

    #[test]
    fn a_file_that_is_not_an_owner_descriptor_never_becomes_an_object() {
        let fixture = Fixture::new();
        // A stray file whose name looks like an object, a descriptor of the
        // wrong kind, and one whose declared name disagrees with the file.
        write(
            &fixture.source.join("Catalogs/Резервная копия.xml"),
            "<!-- not a descriptor -->",
        );
        write(
            &fixture.source.join("Catalogs/Подделка.xml"),
            &owner("Подделка", "Document", "Подделка", ""),
        );
        write(
            &fixture.source.join("Catalogs/Валюты/Forms/Чужая.xml"),
            &owner("Другая", "Form", "Другая форма", ""),
        );
        let root = RetainedDirectoryCapability::open(&fixture.source).unwrap();
        let index = WorkspaceFindDirectoryBuilder::default()
            .build(
                &[LayoutFindSource::new(
                    "main",
                    SourceSetKind::Configuration,
                    &root,
                )],
                ProviderDeadline::from_budget(Duration::from_secs(7)),
                &CancellationToken::new(),
            )
            .unwrap();
        for query in [
            "Резервная копия",
            "main:Catalog.Подделка",
            "main:Catalog.Валюты.Form.Чужая",
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert!(
                found.is_nearest() || found.candidates().is_empty(),
                "a file that declares no matching owner became an object: {query}: {found:?}"
            );
        }
        // The real objects beside them are still there.
        assert_eq!(single(&index, "Валюты").0, "main:Catalog.Валюты");
        assert_eq!(
            single(&index, "ФормаЭлемента").0,
            "main:Catalog.Валюты.Form.ФормаЭлемента"
        );
    }

    #[test]
    fn find_address_path_directory_contract_is_complete() {
        a_name_resolves_to_the_address_and_the_file_that_carries_it();
        a_file_path_resolves_back_to_its_object_address();
        a_synonym_resolves_to_its_object();
        the_directory_holds_objects_and_never_code_symbols_or_inner_nodes();
        the_directory_refuses_to_grow_past_its_entry_bound();
        the_directory_observes_cancellation();
        the_directory_observes_its_operation_deadline();
        an_external_root_publishes_its_owner_and_never_the_dump_sidecar();
        a_file_that_is_not_an_owner_descriptor_never_becomes_an_object();
    }
}
