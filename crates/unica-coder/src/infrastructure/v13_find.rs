use crate::application::v13::find::{FindDocument, FindFact, FindFactKind, FindIndex};
use crate::application::v13::view::{ViewFilter, ViewReadAuthority};
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_SOURCE_SETS: usize = 64;
const DEFAULT_MAX_DOCUMENTS: usize = 65_536;
const DEFAULT_MAX_FACT_BYTES: usize = 16 * 1024 * 1024;

/// One source-set root retained by the workspace actor. The builder never
/// reopens a workspace path or discovers ambient roots on its own.
pub(crate) struct ActorFindSource<'a> {
    name: &'a str,
    reader: &'a dyn ViewReadAuthority,
}

impl<'a> ActorFindSource<'a> {
    pub(crate) const fn new(name: &'a str, reader: &'a dyn ViewReadAuthority) -> Self {
        Self { name, reader }
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

pub(crate) struct BuiltFindIndex {
    pub(crate) index: FindIndex,
    pub(crate) revision: String,
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
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<FindIndex, FindBuildError> {
        self.build_with_revision(sources, deadline, cancellation)
            .map(|built| built.index)
    }

    pub(crate) fn build_with_revision(
        &self,
        sources: &[ActorFindSource<'_>],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<BuiltFindIndex, FindBuildError> {
        if sources.len() > MAX_SOURCE_SETS {
            return Err(FindBuildError::new(
                "provider_limit_exceeded",
                "find source-set count exceeds the bounded workspace limit",
            ));
        }
        let mut documents = Vec::new();
        let mut total_fact_bytes = 0;
        let mut revisions = Vec::with_capacity(sources.len());
        for source in sources {
            find_checkpoint(deadline, cancellation)?;
            let admitted = self.add_source(
                source,
                &mut documents,
                &mut total_fact_bytes,
                deadline,
                cancellation,
            )?;
            revisions.push((
                source.name.to_string(),
                admitted.source_set_identity,
                admitted.revision,
            ));
        }
        Ok(BuiltFindIndex {
            index: FindIndex::new(documents),
            revision: aggregate_revision(&revisions),
        })
    }

    fn add_source(
        &self,
        source: &ActorFindSource<'_>,
        documents: &mut Vec<FindDocument>,
        total_fact_bytes: &mut usize,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<crate::application::v13::view::ViewSourceSnapshot, FindBuildError> {
        let root_at = QualifiedAddress::parse(&format!("{}:Configuration", source.name))
            .map_err(|error| FindBuildError::new("provider_unavailable", error.to_string()))?;
        let admitted = source.reader.snapshot(&root_at).map_err(view_error)?;
        let mut queue = VecDeque::from([root_at.clone()]);
        let mut queued = HashSet::new();
        let mut fallbacks = HashMap::<String, Value>::new();
        queued.insert(queue[0].to_string());
        while let Some(address) = queue.pop_front() {
            find_checkpoint(deadline, cancellation)?;
            let projection =
                match source
                    .reader
                    .read_exact(&address, &ViewFilter::default(), &admitted)
                {
                    Ok(projection) => projection,
                    Err(error) if error.code() == "not_found" && address != root_at => {
                        if let Some(fallback) = fallbacks.get(&address.to_string()) {
                            self.push_identity_value(documents, total_fact_bytes, fallback, None)?;
                        }
                        continue;
                    }
                    Err(error)
                        if error.code() == "provider_unavailable"
                            && source.reader.permits_identity_fallback(&address) =>
                    {
                        let fallback = fallbacks.get(&address.to_string()).ok_or_else(|| {
                            FindBuildError::new(
                                "provider_unavailable",
                                "typed identity fallback was not projected by its parent",
                            )
                        })?;
                        self.push_identity_value(documents, total_fact_bytes, fallback, None)?;
                        continue;
                    }
                    Err(error) => {
                        let mapped = view_error(error);
                        return Err(FindBuildError::new(
                            mapped.code(),
                            format!("find could not read logical identity `{address}`: {mapped}"),
                        ));
                    }
                };
            let value = serde_json::to_value(projection)
                .map_err(|error| FindBuildError::new("provider_unavailable", error.to_string()))?;
            let export_path = source
                .reader
                .identity_export_path(&address)
                .map_err(view_error)?;
            self.push_identity_value(documents, total_fact_bytes, &value, export_path.as_deref())?;
            for child in logical_children(&value)? {
                let Some(at) = child.get("at").and_then(Value::as_str) else {
                    continue;
                };
                let address = QualifiedAddress::parse(at).map_err(|error| {
                    FindBuildError::new("provider_unavailable", error.to_string())
                })?;
                if address
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.kind() == NodeKind::Body)
                {
                    continue;
                }
                if child.get("kind").is_some() && child.get("title").is_some() {
                    fallbacks.insert(at.to_string(), child.clone());
                }
                if queued.insert(at.to_string()) {
                    if address.source_set() != source.name {
                        return Err(FindBuildError::new(
                            "provider_unavailable",
                            "typed logical child belongs to another source set",
                        ));
                    }
                    if queued.len() > self.max_documents {
                        return Err(FindBuildError::new(
                            "provider_limit_exceeded",
                            "find logical-tree queue exceeds the bounded document limit",
                        ));
                    }
                    queue.push_back(address);
                }
            }
        }
        find_checkpoint(deadline, cancellation)?;
        let current = source.reader.snapshot(&root_at).map_err(view_error)?;
        if current != admitted {
            return Err(FindBuildError::new(
                "stale_revision",
                "source revision changed during find index construction",
            ));
        }
        Ok(admitted)
    }

    fn push_identity_value(
        &self,
        documents: &mut Vec<FindDocument>,
        total_fact_bytes: &mut usize,
        value: &Value,
        export_path: Option<&str>,
    ) -> Result<(), FindBuildError> {
        let Some(document) = identity_document_from_view(value, export_path)? else {
            return Ok(());
        };
        if documents
            .iter()
            .any(|existing| existing.at() == document.at())
        {
            return Ok(());
        }
        self.push_document(documents, total_fact_bytes, document)
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

fn aggregate_revision(revisions: &[(String, String, String)]) -> String {
    if let [(_, _, revision)] = revisions {
        return revision.clone();
    }
    let mut digest = Sha256::new();
    digest.update(b"unica-find-source-revision-v1\0");
    for (name, identity, revision) in revisions {
        for value in [name, identity, revision] {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value.as_bytes());
        }
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn logical_children(value: &Value) -> Result<Vec<&Value>, FindBuildError> {
    let object = value.as_object().ok_or_else(|| {
        FindBuildError::new(
            "provider_unavailable",
            "typed logical view is not an object",
        )
    })?;
    let mut children = Vec::new();
    for key in ["branches", "items"] {
        if let Some(values) = object.get(key) {
            let values = values.as_array().ok_or_else(|| {
                FindBuildError::new(
                    "provider_unavailable",
                    format!("typed logical view `{key}` is not an array"),
                )
            })?;
            children.extend(values.iter().filter(|value| value.get("at").is_some()));
        }
    }
    Ok(children)
}

fn identity_document_from_view(
    value: &Value,
    export_path: Option<&str>,
) -> Result<Option<FindDocument>, FindBuildError> {
    let Some(at) = value.get("at").and_then(Value::as_str) else {
        return Ok(None);
    };
    let address = QualifiedAddress::parse(at)
        .map_err(|error| FindBuildError::new("provider_unavailable", error.to_string()))?;
    let Some(kind) = value.get("kind").and_then(Value::as_str) else {
        return Ok(None);
    };
    NodeKind::parse(kind).map_err(|_| {
        FindBuildError::new(
            "provider_unavailable",
            format!("typed node has unaddressable kind `{kind}`"),
        )
    })?;
    let Some(title) = value.get("title").and_then(Value::as_str) else {
        return Ok(None);
    };
    let name = address
        .segments()
        .last()
        .and_then(|segment| segment.name())
        .unwrap_or(kind);
    let mut facts = vec![FindFact::new(FindFactKind::Name, name)];
    if title != name && title != kind {
        facts.push(FindFact::new(FindFactKind::Synonym, title));
    }
    if let Some(props) = value.get("props").and_then(Value::as_object) {
        for key in ["name", "synonym"] {
            if let Some(identity) = props.get(key).and_then(Value::as_str) {
                if !identity.is_empty() {
                    facts.push(FindFact::new(
                        if key == "synonym" {
                            FindFactKind::Synonym
                        } else {
                            FindFactKind::Name
                        },
                        identity,
                    ));
                }
            }
        }
    }
    if let Some(path) = export_path {
        facts.push(FindFact::new(FindFactKind::ExportPath, path));
    }
    Ok(Some(FindDocument::new(at, kind, title, facts)))
}

fn find_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), FindBuildError> {
    if cancellation.is_cancelled() {
        return Err(FindBuildError::new(
            "cancelled",
            "find index construction was cancelled",
        ));
    }
    if deadline.remaining().is_zero() {
        return Err(FindBuildError::new(
            "provider_deadline",
            "find index construction deadline elapsed",
        ));
    }
    Ok(())
}

fn view_error(error: crate::application::v13::view::ViewError) -> FindBuildError {
    FindBuildError::new(error.code(), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{ActorFindSource, WorkspaceFindIndexBuilder};
    use crate::application::v13::find::FindRequest;
    use crate::application::v13::view::{
        ViewError, ViewFilter, ViewReadAuthority, ViewSourceSnapshot,
    };
    use crate::domain::address::QualifiedAddress;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
    use crate::domain::platform_profile::PlatformProfile;
    use crate::domain::project_sources::SourceSetKind;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
    use crate::infrastructure::source_revision::SourceRevisionService;
    use crate::infrastructure::v13_read::{
        module_branch_for_parent, project_module_branch, LogicalViewReadAuthority,
    };
    use serde::Deserialize;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn build_index(
        builder: WorkspaceFindIndexBuilder,
        source: &std::path::Path,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<crate::application::v13::find::FindIndex, super::FindBuildError> {
        let workspace_root = source.parent().unwrap().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace_root.clone(),
            workspace_root: workspace_root.clone(),
            cache_root: workspace_root.join(".cache"),
            workspace_epoch: 1,
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        let revisions =
            Arc::new(SourceRevisionService::new_reconciling_for_test(&context, source).unwrap());
        let authority = LogicalViewReadAuthority::new(
            cancellation,
            "main",
            "find-fixture-main",
            SourceSetKind::Configuration,
            revisions,
            root,
            PlatformProfile::v8_3_27(),
        );
        builder.build(
            &[ActorFindSource::new("main", &authority)],
            deadline,
            cancellation,
        )
    }

    fn deadline() -> ProviderDeadline {
        ProviderDeadline::from_budget(Duration::from_secs(7))
    }

    struct MutatingRevisionReader {
        snapshots: AtomicUsize,
    }

    impl ViewReadAuthority for MutatingRevisionReader {
        fn snapshot(&self, _at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            let revision = if self.snapshots.fetch_add(1, Ordering::SeqCst) == 0 {
                "rev-a"
            } else {
                "rev-b"
            };
            Ok(ViewSourceSnapshot {
                source_set_identity: "source-a".to_string(),
                revision: revision.to_string(),
            })
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            Ok(NodeViewData::Node(NodeView::new(
                at.to_string(),
                "Configuration",
                "Configuration",
                serde_json::Map::new(),
            )))
        }
    }

    struct CancellingReader {
        cancellation: CancellationToken,
    }

    struct UnreadableWebSocketModuleReader;

    impl ViewReadAuthority for UnreadableWebSocketModuleReader {
        fn snapshot(&self, _at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            Ok(ViewSourceSnapshot {
                source_set_identity: "source-a".to_string(),
                revision: "rev-a".to_string(),
            })
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            match at.to_string().as_str() {
                "main:Configuration" => Ok(NodeViewData::Node(
                    NodeView::new(
                        at.to_string(),
                        "Configuration",
                        "Configuration",
                        serde_json::Map::new(),
                    )
                    .with_branches(vec![BranchRef::new("main:WebSocketClient", 1)]),
                )),
                "main:WebSocketClient" => Ok(NodeViewData::Collection(CollectionView::new(
                    NodeView::new(
                        at.to_string(),
                        "WebSocketClient",
                        "WebSocketClient",
                        serde_json::Map::new(),
                    ),
                    vec![serde_json::to_value(
                        NodeView::new(
                            "main:WebSocketClient.Telephony",
                            "WebSocketClient",
                            "Telephony",
                            serde_json::Map::new(),
                        )
                        .with_branches(vec![BranchRef::new(
                            "main:WebSocketClient.Telephony.Module",
                            1,
                        )]),
                    )
                    .unwrap()],
                ))),
                "main:WebSocketClient.Telephony" => Ok(NodeViewData::Node(
                    NodeView::new(
                        at.to_string(),
                        "WebSocketClient",
                        "Telephony",
                        serde_json::Map::new(),
                    )
                    .with_branches(vec![BranchRef::new(
                        "main:WebSocketClient.Telephony.Module",
                        1,
                    )]),
                )),
                "main:WebSocketClient.Telephony.Module" => {
                    Ok(NodeViewData::Collection(CollectionView::new(
                        NodeView::new(at.to_string(), "Module", "Module", serde_json::Map::new()),
                        vec![serde_json::to_value(NodeView::new(
                            "main:WebSocketClient.Telephony.Module.WebSocketClient",
                            "Module",
                            "WebSocketClient module Telephony",
                            serde_json::Map::new(),
                        ))
                        .unwrap()],
                    )))
                }
                "main:WebSocketClient.Telephony.Module.WebSocketClient" => Err(ViewError::new(
                    "provider_unavailable",
                    "WebSocketClient source layout is intentionally unavailable",
                )),
                other => Err(ViewError::new("not_found", format!("unknown {other}"))),
            }
        }

        fn permits_identity_fallback(&self, at: &QualifiedAddress) -> bool {
            at.to_string() == "main:WebSocketClient.Telephony.Module.WebSocketClient"
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ModuleMatrixFixture {
        module_capabilities: Vec<ModuleMatrixCase>,
    }

    #[derive(Debug, Deserialize)]
    struct ModuleMatrixCase {
        at: String,
        exists: bool,
    }

    struct ProfileModuleMatrixReader {
        source: String,
        modules: Vec<String>,
    }

    impl ProfileModuleMatrixReader {
        fn new(source: &str, modules: &[String]) -> Self {
            Self {
                source: source.to_string(),
                modules: modules
                    .iter()
                    .filter(|address| address.starts_with(&format!("{source}:")))
                    .cloned()
                    .collect(),
            }
        }

        fn root_owners(&self) -> BTreeSet<String> {
            self.modules
                .iter()
                .filter_map(|module| {
                    let logical = module.split_once(':')?.1;
                    if logical.starts_with("Module.") {
                        return None;
                    }
                    let owner = module
                        .split_once(".Module.")
                        .map_or(module.as_str(), |(owner, _)| owner);
                    let tokens = owner.split_once(':')?.1.split('.').collect::<Vec<_>>();
                    (tokens.len() >= 2)
                        .then(|| format!("{}:{}.{}", self.source, tokens[0], tokens[1]))
                })
                .collect()
        }

        fn nested_owners(&self, root: &str) -> BTreeSet<String> {
            self.modules
                .iter()
                .filter_map(|module| module.split_once(".Module.").map(|(owner, _)| owner))
                .filter(|owner| owner.starts_with(&format!("{root}.")))
                .filter(|owner| owner.split_once(':').unwrap().1.split('.').count() == 4)
                .map(str::to_string)
                .collect()
        }

        fn owner_node(&self, at: &QualifiedAddress) -> NodeViewData {
            let mut branches = Vec::new();
            if let Some(module_branch) = module_branch_for_parent(at, PlatformProfile::v8_3_27()) {
                branches.push(module_branch);
            }
            let nested = self.nested_owners(&at.to_string());
            let mut child_counts = BTreeMap::<String, usize>::new();
            for child in nested {
                let logical = child
                    .split_once(':')
                    .unwrap()
                    .1
                    .split('.')
                    .collect::<Vec<_>>();
                *child_counts.entry(logical[2].to_string()).or_default() += 1;
            }
            branches.extend(
                child_counts
                    .into_iter()
                    .map(|(kind, count)| BranchRef::new(format!("{at}.{kind}"), count)),
            );
            let segment = at.segments().last().unwrap();
            NodeViewData::Node(
                NodeView::new(
                    at.to_string(),
                    segment.kind().as_str(),
                    segment.name().unwrap_or(segment.kind().as_str()),
                    serde_json::Map::new(),
                )
                .with_branches(branches),
            )
        }

        fn module_collection(&self, at: &QualifiedAddress) -> NodeViewData {
            project_module_branch(at, PlatformProfile::v8_3_27()).unwrap()
        }
    }

    impl ViewReadAuthority for ProfileModuleMatrixReader {
        fn snapshot(&self, _at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            Ok(ViewSourceSnapshot {
                source_set_identity: format!("{}-source", self.source),
                revision: "rev-a".to_string(),
            })
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            let text = at.to_string();
            if text == format!("{}:Configuration", self.source) {
                let roots = self.root_owners();
                let mut counts = BTreeMap::<String, usize>::new();
                for owner in &roots {
                    let kind = owner.split_once(':').unwrap().1.split('.').next().unwrap();
                    *counts.entry(kind.to_string()).or_default() += 1;
                }
                let mut branches = counts
                    .into_iter()
                    .map(|(kind, count)| BranchRef::new(format!("{}:{kind}", self.source), count))
                    .collect::<Vec<_>>();
                let root = QualifiedAddress::parse(&text).unwrap();
                if self.source == "main" {
                    branches.extend(module_branch_for_parent(&root, PlatformProfile::v8_3_27()));
                }
                return Ok(NodeViewData::Node(
                    NodeView::new(
                        text,
                        "Configuration",
                        "Configuration",
                        serde_json::Map::new(),
                    )
                    .with_branches(branches),
                ));
            }
            if PlatformProfile::v8_3_27().module_capability(at).is_some() {
                if text == "main:Document.ЕщеНеВыгружен.Module.Object" {
                    return Err(ViewError::new(
                        "not_found",
                        "registered owner has no retained Module.bsl source",
                    ));
                }
                if at
                    .segments()
                    .last()
                    .is_some_and(|segment| segment.name() == Some("WebSocketClient"))
                {
                    return Err(ViewError::new(
                        "provider_unavailable",
                        "WebSocketClient source layout is intentionally unavailable",
                    ));
                }
                return Ok(NodeViewData::Node(NodeView::new(
                    text,
                    "Module",
                    "Module",
                    serde_json::Map::new(),
                )));
            }
            if !PlatformProfile::v8_3_27().module_children(at).is_empty() {
                return Ok(self.module_collection(at));
            }
            let logical = text
                .split_once(':')
                .unwrap()
                .1
                .split('.')
                .collect::<Vec<_>>();
            if logical.len() == 1 {
                let roots = self.root_owners();
                let items = roots
                    .iter()
                    .filter(|owner| owner.split_once(':').unwrap().1.starts_with(logical[0]))
                    .map(|owner| {
                        let owner = QualifiedAddress::parse(owner).unwrap();
                        serde_json::to_value(self.owner_node(&owner)).unwrap()
                    })
                    .collect();
                return Ok(NodeViewData::Collection(CollectionView::new(
                    NodeView::new(text.clone(), logical[0], logical[0], serde_json::Map::new()),
                    items,
                )));
            }
            if logical.len() == 3 {
                let root = format!("{}:{}.{}", self.source, logical[0], logical[1]);
                let nested = self.nested_owners(&root);
                let items = nested
                    .iter()
                    .filter(|owner| {
                        owner.split_once(':').unwrap().1.split('.').nth(2) == Some(logical[2])
                    })
                    .map(|owner| {
                        let owner = QualifiedAddress::parse(owner).unwrap();
                        serde_json::to_value(self.owner_node(&owner)).unwrap()
                    })
                    .collect();
                return Ok(NodeViewData::Collection(CollectionView::new(
                    NodeView::new(text.clone(), logical[2], logical[2], serde_json::Map::new()),
                    items,
                )));
            }
            let roots = self.root_owners();
            let nested = roots
                .iter()
                .flat_map(|root| self.nested_owners(root))
                .collect::<BTreeSet<_>>();
            if roots.contains(&text) || nested.contains(&text) {
                return Ok(self.owner_node(at));
            }
            Err(ViewError::new("not_found", format!("unknown {text}")))
        }

        fn permits_identity_fallback(&self, at: &QualifiedAddress) -> bool {
            PlatformProfile::v8_3_27()
                .module_capability(at)
                .is_some_and(|capability| {
                    capability.role()
                        == crate::domain::platform_profile::ModuleRole::WebSocketClient
                })
        }
    }

    impl ViewReadAuthority for CancellingReader {
        fn snapshot(&self, _at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            Ok(ViewSourceSnapshot {
                source_set_identity: "source-a".to_string(),
                revision: "rev-a".to_string(),
            })
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            self.cancellation.cancel();
            Ok(NodeViewData::Node(NodeView::new(
                at.to_string(),
                "Configuration",
                "Configuration",
                serde_json::Map::new(),
            )))
        }
    }

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
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Товары</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(source.join("Catalogs/Товары/Ext/ObjectModule.bsl"), body).unwrap();
        let source = fs::canonicalize(source).unwrap();
        (fixture, source)
    }

    fn common_module_identity_source(statement: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("src");
        fs::create_dir_all(source.join("CommonModules/IdentityModule/Ext")).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>IdentityModule</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("CommonModules/IdentityModule.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>IdentityModule</Name><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("CommonModules/IdentityModule/Ext/Module.bsl"),
            format!(
                "#Region PublicApi\nProcedure StableMethod(Value) Export\n    {statement};\nEndProcedure\n#EndRegion\n"
            ),
        )
        .unwrap();
        let source = fs::canonicalize(source).unwrap();
        (fixture, source)
    }

    #[test]
    fn actor_owned_platform_xml_registry_builds_identity_only_find_index() {
        let (_fixture, source) =
            identity_source("Procedure StableMethod()\nСовершенноСекретноеСлово();\nEndProcedure");
        let cancellation = CancellationToken::new();
        let index = build_index(
            WorkspaceFindIndexBuilder::default(),
            &source,
            &cancellation,
            deadline(),
        )
        .unwrap();

        for (query, reason, expected_at) in [
            ("Товары", "name", "main:Catalog.Товары"),
            ("Номенклатура", "synonym", "main:Catalog.Товары"),
            ("Catalog", "name", "main:Catalog"),
            ("Catalogs/Товары.xml", "exportPath", "main:Catalog.Товары"),
        ] {
            let found = index.find(FindRequest::new(query).unwrap());
            assert_eq!(found.candidates()[0].at(), expected_at);
            assert_eq!(found.candidates()[0].reason(), reason);
        }
        let english_kind = index.find(FindRequest::new("Catalog").unwrap());
        assert!(english_kind.candidates().iter().any(|candidate| {
            candidate.at() == "main:Catalog.Товары" && candidate.reason() == "kind"
        }));
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
            identity_source("Procedure StableMethod()\nОдинСекретныйОператор();\nEndProcedure");
        let (_right_fixture, right_source) =
            identity_source("Procedure StableMethod()\nСовсемДругойОператор();\nEndProcedure");
        let builder = WorkspaceFindIndexBuilder::default();
        let cancellation = CancellationToken::new();
        let left = build_index(builder.clone(), &left_source, &cancellation, deadline()).unwrap();
        let right = build_index(builder, &right_source, &cancellation, deadline()).unwrap();

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
    fn find_reads_only_module_declarations_and_never_indexes_the_body_projection() {
        let (_left_fixture, left_source) = common_module_identity_source("LeftBodyOnlyStatement()");
        let (_right_fixture, right_source) =
            common_module_identity_source("RightBodyOnlyStatement()");
        let cancellation = CancellationToken::new();
        let builder = WorkspaceFindIndexBuilder::default();
        let left = build_index(builder.clone(), &left_source, &cancellation, deadline()).unwrap();
        let right = build_index(builder, &right_source, &cancellation, deadline()).unwrap();

        assert_eq!(left.fact_bytes(), right.fact_bytes());
        for address in [
            "main:CommonModule.IdentityModule.Method.StableMethod",
            "main:CommonModule.IdentityModule.Region.PublicApi",
        ] {
            let left_result = left.find(FindRequest::new(address).unwrap());
            let right_result = right.find(FindRequest::new(address).unwrap());
            assert!(!left_result.is_nearest(), "{left_result:?}");
            assert!(left_result
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == address));
            assert_eq!(
                serde_json::to_vec(&left_result).unwrap(),
                serde_json::to_vec(&right_result).unwrap(),
            );
        }
        for (index, query) in [
            (&left, "main:CommonModule.IdentityModule.Body"),
            (&left, "LeftBodyOnlyStatement"),
            (&right, "RightBodyOnlyStatement"),
        ] {
            let result = index.find(FindRequest::new(query).unwrap());
            assert!(
                result.is_nearest(),
                "body identity leaked for {query}: {result:?}"
            );
            assert!(result.candidates().iter().all(|candidate| {
                candidate.at() != "main:CommonModule.IdentityModule.Body"
                    && (candidate.reason().starts_with("nearest:name")
                        || candidate.reason().starts_with("nearest:address"))
            }));
        }
    }

    #[test]
    fn find_indexes_nested_addressable_metadata_identity() {
        let (_fixture, source) = identity_source("Procedure Any()\nEndProcedure");
        fs::write(
            source.join("Catalogs/Товары.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Товары</Name></Properties><ChildObjects><Attribute uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>КодПоставщика</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Код поставщика</v8:content></v8:item></Synonym><Type><v8:Type>xs:string</v8:Type></Type></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let index = build_index(
            WorkspaceFindIndexBuilder::default(),
            &source,
            &cancellation,
            deadline(),
        )
        .unwrap();

        let result = index.find(FindRequest::new("Код поставщика").unwrap());
        assert!(!result.is_nearest(), "{result:?}");
        assert_eq!(
            result.candidates()[0].at(),
            "main:Catalog.Товары.Attribute.КодПоставщика"
        );
    }

    #[test]
    fn find_fails_closed_when_a_registered_descriptor_is_malformed() {
        let (_fixture, source) = identity_source("Procedure Any()\nEndProcedure");
        fs::write(source.join("Catalogs/Товары.xml"), "<broken>").unwrap();
        let cancellation = CancellationToken::new();
        let error = build_index(
            WorkspaceFindIndexBuilder::default(),
            &source,
            &cancellation,
            deadline(),
        )
        .unwrap_err();

        assert_eq!(error.code(), "provider_unavailable");
    }

    #[test]
    fn find_fails_when_the_exact_source_revision_changes_during_the_walk() {
        let reader = MutatingRevisionReader {
            snapshots: AtomicUsize::new(0),
        };
        let cancellation = CancellationToken::new();
        let error = WorkspaceFindIndexBuilder::default()
            .build(
                &[ActorFindSource::new("main", &reader)],
                deadline(),
                &cancellation,
            )
            .unwrap_err();

        assert_eq!(error.code(), "stale_revision");
    }

    #[test]
    fn find_observes_cancellation_inside_the_logical_tree_walk() {
        let cancellation = CancellationToken::new();
        let reader = CancellingReader {
            cancellation: cancellation.clone(),
        };
        let error = WorkspaceFindIndexBuilder::default()
            .build(
                &[ActorFindSource::new("main", &reader)],
                deadline(),
                &cancellation,
            )
            .unwrap_err();

        assert_eq!(error.code(), "cancelled");
    }

    #[test]
    fn find_observes_one_aggregate_provider_deadline() {
        let reader = MutatingRevisionReader {
            snapshots: AtomicUsize::new(0),
        };
        let cancellation = CancellationToken::new();
        let error = WorkspaceFindIndexBuilder::default()
            .build(
                &[ActorFindSource::new("main", &reader)],
                ProviderDeadline::from_budget(Duration::ZERO),
                &cancellation,
            )
            .unwrap_err();

        assert_eq!(error.code(), "provider_deadline");
    }

    #[test]
    fn find_keeps_profile_identity_for_the_typed_unreadable_websocket_module() {
        let cancellation = CancellationToken::new();
        let built = WorkspaceFindIndexBuilder::default().build(
            &[ActorFindSource::new(
                "main",
                &UnreadableWebSocketModuleReader,
            )],
            deadline(),
            &cancellation,
        );
        let index = built.expect("the typed unreadable module must not collapse other identities");
        let address = "main:WebSocketClient.Telephony.Module.WebSocketClient";
        let found = index.find(FindRequest::new(address).unwrap());
        assert!(!found.is_nearest(), "{found:?}");
        assert!(found
            .candidates()
            .iter()
            .any(|candidate| candidate.at() == address));
    }

    #[test]
    fn profile_matrix_supplements_production_module_capability_coverage() {
        let fixture: ModuleMatrixFixture = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/v013/address-profile-8.3.27.json"
        ))
        .unwrap();
        let expected = fixture
            .module_capabilities
            .into_iter()
            .filter(|case| case.exists)
            .map(|case| case.at)
            .collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 25);
        let main =
            ProfileModuleMatrixReader::new("main", &expected.iter().cloned().collect::<Vec<_>>());
        let epf =
            ProfileModuleMatrixReader::new("epf", &expected.iter().cloned().collect::<Vec<_>>());
        let erf =
            ProfileModuleMatrixReader::new("erf", &expected.iter().cloned().collect::<Vec<_>>());
        let cancellation = CancellationToken::new();
        let index = WorkspaceFindIndexBuilder::default()
            .build(
                &[
                    ActorFindSource::new("main", &main),
                    ActorFindSource::new("epf", &epf),
                    ActorFindSource::new("erf", &erf),
                ],
                deadline(),
                &cancellation,
            )
            .unwrap();
        assert!(main.root_owners().contains("main:Document.ЕщеНеВыгружен"));
        for address in expected {
            let found = index.find(FindRequest::new(&address).unwrap().with_limit(100).unwrap());
            assert!(
                !found.is_nearest()
                    && found
                        .candidates()
                        .iter()
                        .any(|candidate| candidate.at() == address),
                "missing module identity {address}: {found:?}",
            );
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
        fs::create_dir_all(source.join("Catalogs")).unwrap();
        for name in ["One", "Two"] {
            fs::write(
                source.join(format!("Catalogs/{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>{name}</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        let source = fs::canonicalize(source).unwrap();
        let cancellation = CancellationToken::new();
        let error = build_index(
            WorkspaceFindIndexBuilder::with_document_limit(2),
            &source,
            &cancellation,
            deadline(),
        )
        .unwrap_err();

        assert_eq!(error.code(), "provider_limit_exceeded");
    }

    #[test]
    fn workspace_find_builder_refuses_identity_facts_above_byte_budget() {
        let (_fixture, source) = identity_source("Procedure Any()\nEndProcedure");
        let cancellation = CancellationToken::new();
        let error = build_index(
            WorkspaceFindIndexBuilder::with_limits(10, 64),
            &source,
            &cancellation,
            deadline(),
        )
        .unwrap_err();

        assert_eq!(error.code(), "provider_limit_exceeded");
    }

    #[test]
    fn find_identity_only_contract_is_complete() {
        actor_owned_platform_xml_registry_builds_identity_only_find_index();
        find_reads_only_module_declarations_and_never_indexes_the_body_projection();
        find_indexes_nested_addressable_metadata_identity();
        crate::infrastructure::v13_read::tests::production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories();
        crate::infrastructure::v13_read::tests::one_find_reads_each_module_source_once_per_actor_revision();
        profile_matrix_supplements_production_module_capability_coverage();
        find_keeps_profile_identity_for_the_typed_unreadable_websocket_module();
        find_fails_closed_when_a_registered_descriptor_is_malformed();
        find_fails_when_the_exact_source_revision_changes_during_the_walk();
        find_observes_cancellation_inside_the_logical_tree_walk();
        find_observes_one_aggregate_provider_deadline();
        workspace_find_builder_refuses_registry_materialization_above_bound();
        workspace_find_builder_refuses_identity_facts_above_byte_budget();
        crate::application::invocation::tests::assert_operation_budget_survives_handoff_and_completes_once(
            crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
        );
        crate::infrastructure::v13_read::tests::find_uses_each_typed_readers_real_export_path_without_publishing_it_in_view_props();
    }
}
