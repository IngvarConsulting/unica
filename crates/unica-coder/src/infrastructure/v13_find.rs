use crate::application::v13::find::{FindDocument, FindFact, FindFactKind, FindIndex};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use roxmltree::{Document, Node};
use std::io;
use std::path::{Path, PathBuf};

const MAX_SOURCE_SETS: usize = 64;
const DEFAULT_MAX_DOCUMENTS: usize = 65_536;
const DEFAULT_MAX_FACT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONFIGURATION_BYTES: usize = 8 * 1024 * 1024;
const MAX_DESCRIPTOR_BYTES: usize = 2 * 1024 * 1024;

/// One source-set root retained by the workspace actor. The builder never
/// reopens a workspace path or discovers ambient roots on its own.
#[derive(Clone, Copy)]
pub(crate) struct ActorFindSource<'a> {
    name: &'a str,
    root: &'a RetainedDirectoryCapability,
}

impl<'a> ActorFindSource<'a> {
    pub(crate) const fn new(name: &'a str, root: &'a RetainedDirectoryCapability) -> Self {
        Self { name, root }
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

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceFindIndexBuilder {
    max_documents: usize,
    max_total_fact_bytes: usize,
}

impl Default for WorkspaceFindIndexBuilder {
    fn default() -> Self {
        Self {
            max_documents: DEFAULT_MAX_DOCUMENTS,
            max_total_fact_bytes: DEFAULT_MAX_FACT_BYTES,
        }
    }
}

impl WorkspaceFindIndexBuilder {
    #[cfg(test)]
    fn with_document_limit(max_documents: usize) -> Self {
        Self {
            max_documents,
            max_total_fact_bytes: DEFAULT_MAX_FACT_BYTES,
        }
    }

    #[cfg(test)]
    fn with_limits(max_documents: usize, max_total_fact_bytes: usize) -> Self {
        Self {
            max_documents,
            max_total_fact_bytes,
        }
    }

    pub(crate) fn build(
        &self,
        sources: &[ActorFindSource<'_>],
    ) -> Result<FindIndex, FindBuildError> {
        if sources.len() > MAX_SOURCE_SETS {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find source-set count exceeds the bounded workspace limit",
            ));
        }
        let mut documents = Vec::new();
        let mut total_fact_bytes = 0;
        for source in sources {
            source
                .root
                .validate_named_identity()
                .map_err(|error| provider_error("source root identity", error))?;
            self.add_source(source, &mut documents, &mut total_fact_bytes)?;
            source
                .root
                .validate_named_identity()
                .map_err(|error| provider_error("source root identity", error))?;
        }
        Ok(FindIndex::new(documents))
    }

    fn add_source(
        &self,
        source: &ActorFindSource<'_>,
        documents: &mut Vec<FindDocument>,
        total_fact_bytes: &mut usize,
    ) -> Result<(), FindBuildError> {
        let bytes = source
            .root
            .read_relative_regular_bounded(Path::new("Configuration.xml"), MAX_CONFIGURATION_BYTES)
            .map_err(|error| provider_error("Configuration.xml", error))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            FindBuildError::new(
                "provider_unavailable",
                "Configuration.xml is not valid UTF-8",
            )
        })?;
        let document = Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
            FindBuildError::new(
                "provider_unavailable",
                format!("Configuration.xml is not valid XML: {error}"),
            )
        })?;
        let configuration = document
            .root_element()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "Configuration")
            .ok_or_else(|| {
                FindBuildError::new(
                    "provider_unavailable",
                    "Configuration.xml has no Configuration owner",
                )
            })?;
        let configuration_at = format!("{}:Configuration", source.name);
        validate_address(&configuration_at)?;
        let configuration_identity = descriptor_identity(configuration);
        self.push_document(
            documents,
            total_fact_bytes,
            identity_document(
                configuration_at,
                NodeKind::Configuration,
                configuration_identity,
                Some("Configuration.xml"),
            ),
        )?;

        let child_objects = configuration
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects");
        for registration in child_objects
            .into_iter()
            .flat_map(|children| children.children().filter(Node::is_element))
        {
            let kind_text = registration.tag_name().name();
            let Some(layout) = metadata_kind(kind_text) else {
                return Err(FindBuildError::new(
                    "provider_unavailable",
                    format!("Configuration.xml registers unsupported kind `{kind_text}`"),
                ));
            };
            let kind = NodeKind::parse(kind_text).map_err(|_| {
                FindBuildError::new(
                    "provider_unavailable",
                    format!("Configuration.xml registers unaddressable kind `{kind_text}`"),
                )
            })?;
            let name = registration.text().unwrap_or_default().trim();
            if !valid_platform_name(name) {
                return Err(FindBuildError::new(
                    "provider_unavailable",
                    format!("Configuration.xml registers an invalid {kind_text} name"),
                ));
            }
            let at = format!("{}:{kind_text}.{name}", source.name);
            validate_address(&at)?;
            let relative = PathBuf::from(layout.directory).join(format!("{name}.xml"));
            let descriptor = optional_descriptor(source.root, &relative)?;
            let identity = descriptor
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| Document::parse(text.trim_start_matches('\u{feff}')).ok())
                .and_then(|descriptor| {
                    descriptor
                        .root_element()
                        .children()
                        .find(|node| node.is_element() && node.tag_name().name() == kind_text)
                        .map(descriptor_identity)
                })
                .unwrap_or_else(|| DescriptorIdentity::named(name));
            let export_path = format!("{}/{}.xml", layout.directory, name);
            self.push_document(
                documents,
                total_fact_bytes,
                identity_document(at, kind, identity, Some(&export_path)),
            )?;
        }
        Ok(())
    }

    fn push_document(
        &self,
        documents: &mut Vec<FindDocument>,
        total_fact_bytes: &mut usize,
        document: FindDocument,
    ) -> Result<(), FindBuildError> {
        if documents.len() == self.max_documents {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find document count exceeds the bounded workspace limit",
            ));
        }
        let next_total = total_fact_bytes
            .checked_add(document.estimated_identity_bytes())
            .ok_or_else(|| {
                FindBuildError::new(
                    "provider_limit_exceeded",
                    "find identity facts exceed the bounded workspace byte budget",
                )
            })?;
        if next_total > self.max_total_fact_bytes {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find identity facts exceed the bounded workspace byte budget",
            ));
        }
        *total_fact_bytes = next_total;
        documents.push(document);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct DescriptorIdentity {
    name: String,
    synonyms: Vec<String>,
}

impl DescriptorIdentity {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            synonyms: Vec::new(),
        }
    }
}

fn descriptor_identity(owner: Node<'_, '_>) -> DescriptorIdentity {
    let properties = owner
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Properties");
    let name = properties
        .and_then(|properties| {
            properties
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "Name")
        })
        .and_then(|node| node.text())
        .unwrap_or_else(|| owner.tag_name().name())
        .trim()
        .to_string();
    let mut synonyms = properties
        .and_then(|properties| {
            properties
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "Synonym")
        })
        .into_iter()
        .flat_map(|synonym| synonym.descendants())
        .filter(|node| node.is_element() && node.tag_name().name() == "content")
        .filter_map(|node| node.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    synonyms.sort();
    synonyms.dedup();
    DescriptorIdentity { name, synonyms }
}

fn identity_document(
    at: String,
    kind: NodeKind,
    identity: DescriptorIdentity,
    export_path: Option<&str>,
) -> FindDocument {
    let mut facts = vec![FindFact::new(FindFactKind::Name, identity.name.clone())];
    facts.extend(
        identity
            .synonyms
            .iter()
            .map(|synonym| FindFact::new(FindFactKind::Synonym, synonym)),
    );
    if let Some(export_path) = export_path {
        facts.push(FindFact::new(FindFactKind::ExportPath, export_path));
    }
    let title = identity
        .synonyms
        .first()
        .cloned()
        .unwrap_or_else(|| identity.name.clone());
    FindDocument::new(at, kind.as_str(), title, facts)
}

fn optional_descriptor(
    root: &RetainedDirectoryCapability,
    relative: &Path,
) -> Result<Option<Vec<u8>>, FindBuildError> {
    match root.read_relative_regular_bounded(relative, MAX_DESCRIPTOR_BYTES) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(provider_error("metadata descriptor", error)),
    }
}

fn valid_platform_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
}

fn validate_address(at: &str) -> Result<(), FindBuildError> {
    QualifiedAddress::parse(at)
        .map(|_| ())
        .map_err(|error| FindBuildError::new("provider_unavailable", error.to_string()))
}

fn provider_error(subject: &str, error: io::Error) -> FindBuildError {
    FindBuildError::new(
        "provider_unavailable",
        format!("cannot read {subject}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{ActorFindSource, WorkspaceFindIndexBuilder};
    use crate::application::v13::find::FindRequest;
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use std::fs;

    fn identity_source(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("src");
        fs::create_dir_all(source.join("Catalogs/Товары/Ext")).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Configuration><Properties><Name>Store</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Магазин</v8:content></v8:item></Synonym></Properties><ChildObjects><Catalog>Товары</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("Catalogs/Товары.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Catalog><Properties><Name>Товары</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(source.join("Catalogs/Товары/Ext/ObjectModule.bsl"), body).unwrap();
        let source = fs::canonicalize(source).unwrap();
        (fixture, source)
    }

    #[test]
    fn actor_owned_platform_xml_registry_builds_identity_only_find_index() {
        let (_fixture, source) =
            identity_source("Procedure СовершенноСекретноеСлово()\nEndProcedure");
        let root = RetainedDirectoryCapability::open(&source).unwrap();
        let index = WorkspaceFindIndexBuilder::default()
            .build(&[ActorFindSource::new("main", &root)])
            .unwrap();

        for (query, reason) in [
            ("Товары", "name"),
            ("Номенклатура", "synonym"),
            ("Catalog", "kind"),
            ("Catalogs/Товары.xml", "exportPath"),
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert_eq!(found.candidates()[0].at(), "main:Catalog.Товары");
            assert_eq!(found.candidates()[0].reason(), reason);
        }
        let content = index.find(FindRequest::new("СовершенноСекретноеСлово").unwrap());
        assert!(content.is_nearest());
        assert!(content.candidates().iter().all(|candidate| {
            candidate.reason().starts_with("nearest:name")
                || candidate.reason().starts_with("nearest:address")
        }));
    }

    #[test]
    fn find_identity_facts_and_results_are_independent_of_bsl_body_bytes() {
        let (_left_fixture, left_source) =
            identity_source("Procedure ОдинСекретныйМетод()\nEndProcedure");
        let (_right_fixture, right_source) = identity_source(
            "Procedure СовсемДругойМетод()\nСообщить(\"другой текст\");\nEndProcedure",
        );
        let left_root = RetainedDirectoryCapability::open(&left_source).unwrap();
        let right_root = RetainedDirectoryCapability::open(&right_source).unwrap();
        let builder = WorkspaceFindIndexBuilder::default();
        let left = builder
            .build(&[ActorFindSource::new("main", &left_root)])
            .unwrap();
        let right = builder
            .build(&[ActorFindSource::new("main", &right_root)])
            .unwrap();

        assert_eq!(left.fact_bytes(), right.fact_bytes());
        for query in ["Номенклатура", "Номенклатур"] {
            let left_result = left.find(FindRequest::new(query).unwrap());
            let right_result = right.find(FindRequest::new(query).unwrap());
            assert_eq!(
                serde_json::to_vec(&left_result).unwrap(),
                serde_json::to_vec(&right_result).unwrap(),
            );
            assert!(left_result.candidates().iter().all(|candidate| {
                !left_result.is_nearest()
                    || candidate.reason().starts_with("nearest:name")
                    || candidate.reason().starts_with("nearest:address")
            }));
        }
    }

    #[test]
    fn workspace_find_builder_refuses_registry_materialization_above_bound() {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("src");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>One</Catalog><Catalog>Two</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let source = fs::canonicalize(source).unwrap();
        let root = RetainedDirectoryCapability::open(&source).unwrap();

        let error = WorkspaceFindIndexBuilder::with_document_limit(2)
            .build(&[ActorFindSource::new("main", &root)])
            .unwrap_err();

        assert_eq!(error.code(), "provider_limit_exceeded");
    }

    #[test]
    fn workspace_find_builder_refuses_identity_facts_above_byte_budget() {
        let (_fixture, source) = identity_source("Procedure Any()\nEndProcedure");
        let root = RetainedDirectoryCapability::open(&source).unwrap();

        let error = WorkspaceFindIndexBuilder::with_limits(10, 64)
            .build(&[ActorFindSource::new("main", &root)])
            .unwrap_err();

        assert_eq!(error.code(), "provider_limit_exceeded");
    }
}
