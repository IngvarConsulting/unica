//! Служба LSP над настоящим актором рабочей области, без продукта.
//!
//! Допуск папки здесь простой: набор `main` в каталоге, имя которого лежит в
//! `source-root` (по умолчанию `src`). Модуль читается с диска поверх буферов
//! сессии из оверлея актора. Допуск продукта и его чтение проверяет
//! `unica-coder`; здесь — то, что служба делает с сессиями, буферами
//! и акторами сама.

use crate::{
    AdmittedWorkspace, LspActor, LspModuleReads, LspWorkspaces, ModuleSourceRead, SourceSetLayout,
    UnicaLspHost,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use unica_address::source_target::MetadataAddress;
use unica_bsl::profile::PlatformProfile;
use unica_execution_contract::view_error::ViewError;
use unica_lsp_contract::host::{LspCallContext, LspClientChannel, LspHost};
use unica_lsp_contract::protocol::{
    V5LspCall, V5LspContentChange, V5LspFailure, V5LspNotification, V5LspOutcome, V5LspPosition,
    V5LspRange, V5LspSessionId, V5LspWorkspaceFolder, V5LspWorkspaceSpace,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_source_designer::module_layout::module_relative;
use unica_workspace::actor::{
    ProviderRootBinding, WorkspaceActor, WorkspaceActorRegistry, WorkspaceSourceSetInput,
};
use unica_workspace::documents::SessionDocuments;

const DISK: &str = "Procedure Disk() EndProcedure\n";
const BODY_URI: &str = "unica://work/main/CommonModule.Main.Body";

type Actor = WorkspaceActor<(), ()>;

mod symbols;

/// Admits a folder whose `main` lies in `source-root` (or `src`).
#[derive(Default)]
struct DiskWorkspaces {
    actors: WorkspaceActorRegistry<()>,
    symbols: Arc<symbols::ScriptedSymbols>,
}

impl DiskWorkspaces {
    fn actor(&self, folder: &Path) -> Option<Arc<Actor>> {
        self.admitted(folder).map(|(actor, _)| actor)
    }

    fn admitted(&self, folder: &Path) -> Option<(Arc<Actor>, ProviderRootBinding)> {
        let folder = folder.canonicalize().ok()?;
        let name = fs::read_to_string(folder.join("source-root")).unwrap_or_else(|_| "src".into());
        let root = folder.join(name.trim());
        if !root.is_dir() {
            return None;
        }
        let context = WorkspaceContext {
            cwd: folder.clone(),
            workspace_root: folder.clone(),
            cache_root: folder.join(".unica-cache"),
            workspace_epoch: 0,
        };
        let actor = self
            .actors
            .get_or_create(
                &context,
                [WorkspaceSourceSetInput::new(
                    "main",
                    root.clone(),
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                "lsp-service-test",
            )
            .ok()?;
        let binding = actor.bind_provider_root("main", &root).ok()?;
        Some((actor, binding))
    }
}

impl LspWorkspaces for DiskWorkspaces {
    fn admit(&self, folder: &Path, _deadline: Instant) -> Option<AdmittedWorkspace> {
        let (actor, binding) = self.admitted(folder)?;
        Some(AdmittedWorkspace {
            source_sets: vec![SourceSetLayout {
                name: "main".to_owned(),
                kind: SourceSetKind::Configuration,
                root: binding.source_root().to_path_buf(),
                profile: PlatformProfile::v8_3_27(),
            }],
            reads: Box::new(DiskReads {
                actor: Arc::downgrade(&actor),
                binding,
            }),
            symbols: self.symbols.clone(),
            actor: actor as Arc<dyn LspActor>,
        })
    }
}

struct DiskReads {
    actor: Weak<Actor>,
    binding: ProviderRootBinding,
}

impl LspModuleReads for DiskReads {
    fn read(
        &self,
        session: &str,
        source_set: &str,
    ) -> Result<Box<dyn ModuleSourceRead>, V5LspFailure> {
        if source_set != "main" {
            return Err(V5LspFailure::DocumentUnknown);
        }
        let actor = self
            .actor
            .upgrade()
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        actor
            .validate_binding(&self.binding)
            .map_err(|_| V5LspFailure::WorkspaceUnavailable)?;
        let documents = actor
            .documents()
            .session_documents(session, source_set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        Ok(Box::new(OverlayRead {
            root: self.binding.source_root().to_path_buf(),
            documents,
        }))
    }
}

/// A buffer of the session, else the file.
struct OverlayRead {
    root: PathBuf,
    documents: SessionDocuments,
}

impl ModuleSourceRead for OverlayRead {
    fn source_set(&self) -> &str {
        "main"
    }

    fn root_path(&self) -> &Path {
        &self.root
    }

    fn module_source(&self, target: &MetadataAddress) -> Result<Option<String>, ViewError> {
        let relative = module_relative(target, SourceSetKind::Configuration).unwrap();
        if let Some(buffer) = self.documents.get(&relative) {
            return Ok(Some(buffer.text.to_string()));
        }
        Ok(fs::read_to_string(self.root.join(relative)).ok())
    }
}

fn workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    write_module(workspace.path(), "src", "Main", DISK);
    workspace
}

fn write_module(folder: &Path, root: &str, name: &str, text: &str) {
    let module = folder
        .join(root)
        .join(format!("CommonModules/{name}/Ext/Module.bsl"));
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(module, text).unwrap();
}

fn folder(path: &Path, name: &str) -> V5LspWorkspaceFolder {
    V5LspWorkspaceFolder {
        uri: url::Url::from_file_path(path.canonicalize().unwrap())
            .unwrap()
            .into(),
        name: name.to_owned(),
    }
}

fn service() -> (UnicaLspHost, Arc<DiskWorkspaces>) {
    let workspaces = Arc::new(DiskWorkspaces::default());
    (UnicaLspHost::new(workspaces.clone()), workspaces)
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn open(host: &UnicaLspHost, path: &Path, name: &str) -> V5LspSessionId {
    let session = V5LspSessionId::new();
    host.open(
        &session,
        &[folder(path, name)],
        LspClientChannel::detached_for_test(),
        soon(),
    )
    .unwrap();
    session
}

fn content(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str) -> V5LspOutcome {
    host.call(
        session,
        V5LspCall::TextDocumentContent {
            uri: uri.to_owned(),
        },
        &LspCallContext::for_test(soon()),
    )
}

fn text(text: &str) -> V5LspOutcome {
    V5LspOutcome::Text {
        text: text.to_owned(),
    }
}

fn open_buffer(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str, text: &str) {
    host.notify(
        session,
        V5LspNotification::DidOpen {
            uri: uri.to_owned(),
            language_id: "bsl".into(),
            version: 1,
            text: text.to_owned(),
        },
        soon(),
    )
    .unwrap();
}

#[test]
fn a_folder_without_sources_or_off_disk_gets_a_failure_and_no_space() {
    let workspace = workspace();
    let empty = tempfile::tempdir().unwrap();
    let (host, _workspaces) = service();
    let session = V5LspSessionId::new();
    let folders = [
        folder(empty.path(), "Work"),
        V5LspWorkspaceFolder {
            uri: "vscode-vfs://github/repo".into(),
            name: "Remote".into(),
        },
        folder(workspace.path(), "Work"),
    ];

    let opening = host
        .open(
            &session,
            &folders,
            LspClientChannel::detached_for_test(),
            soon(),
        )
        .unwrap();

    assert_eq!(
        opening.workspaces,
        vec![
            V5LspWorkspaceSpace::refused(&folders[0].uri, V5LspFailure::WorkspaceUnavailable),
            V5LspWorkspaceSpace::refused(&folders[1].uri, V5LspFailure::WorkspaceUnavailable),
            V5LspWorkspaceSpace::opened(&folders[2].uri, "work"),
        ],
        "a refused folder takes no name"
    );
    assert_eq!(
        content(&host, &session, "unica://other/main/CommonModule.Main.Body"),
        V5LspOutcome::Failed {
            failure: V5LspFailure::DocumentUnknown
        }
    );
}

/// A replaced source root makes the registry give MCP a new actor. The
/// session follows it there with its buffers instead of holding MCP on the
/// old one.
#[test]
fn after_the_source_root_is_replaced_the_session_moves_its_buffers_to_the_mcp_actor() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let editor = open(&host, workspace.path(), "Work");
    open_buffer(
        &host,
        &editor,
        BODY_URI,
        "Procedure Buffer() EndProcedure\n",
    );
    let before = Arc::downgrade(&workspaces.actor(workspace.path()).unwrap());

    // A fresh export replaces the source directory with an equal one.
    fs::rename(
        workspace.path().join("src"),
        workspace.path().join("src.old"),
    )
    .unwrap();
    write_module(workspace.path(), "src", "Main", DISK);
    let mcp = workspaces
        .actor(workspace.path())
        .expect("MCP admits the new root");

    assert!(
        before.upgrade().is_none_or(|old| !Arc::ptr_eq(&old, &mcp)),
        "MCP is not held on the actor of the replaced root"
    );
    assert_eq!(
        content(&host, &editor, BODY_URI),
        text("Procedure Buffer() EndProcedure\n")
    );
    assert!(
        !mcp.documents()
            .session_documents(editor.as_str(), "main")
            .unwrap()
            .is_empty(),
        "the buffer moved into the new actor's overlay"
    );
}

#[test]
fn a_change_the_buffer_cannot_take_is_refused_so_the_editor_drops_it() {
    let workspace = workspace();
    let (host, _workspaces) = service();
    let editor = open(&host, workspace.path(), "Work");
    let change = |version, line| V5LspNotification::DidChange {
        uri: BODY_URI.to_owned(),
        version,
        changes: vec![V5LspContentChange {
            range: Some(V5LspRange {
                start: V5LspPosition { line, character: 0 },
                end: V5LspPosition { line, character: 0 },
            }),
            text: "x".into(),
        }],
    };

    assert_eq!(
        host.notify(&editor, change(2, 0), soon()),
        Err(V5LspFailure::DocumentUnknown),
        "a change of a document that is not open"
    );
    open_buffer(&host, &editor, BODY_URI, DISK);
    assert_eq!(
        host.notify(&editor, change(2, 99), soon()),
        Err(V5LspFailure::DocumentUnknown),
        "a range outside the buffer"
    );
    // The refused document is closed: a later change does not land on a
    // buffer that missed one, and answers come from the file.
    assert_eq!(
        host.notify(&editor, change(3, 0), soon()),
        Err(V5LspFailure::DocumentUnknown)
    );
    assert_eq!(content(&host, &editor, BODY_URI), text(DISK));
    open_buffer(&host, &editor, BODY_URI, "Procedure Again() EndProcedure\n");
    assert_eq!(
        content(&host, &editor, BODY_URI),
        text("Procedure Again() EndProcedure\n")
    );
}

/// Every form an editor may send for one module reaches it: the encoded
/// URI the service gives, its decoded text and an authority in any case.
#[test]
fn a_module_is_reached_by_every_spelling_of_its_unica_uri() {
    let workspace = workspace();
    write_module(workspace.path(), "src", "ОбщегоНазначения", "// УТ\n");
    let (host, _workspaces) = service();
    let editor = open(&host, workspace.path(), "УТ Проект");

    for uri in [
        "unica://ут%20проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://ут проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://УТ%20Проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://%D0%A3%D0%A2%20%D0%9F%D1%80%D0%BE%D0%B5%D0%BA%D1%82/main/CommonModule.ОбщегоНазначения.Body",
    ] {
        assert_eq!(content(&host, &editor, uri), text("// УТ\n"), "{uri}");
    }
}

/// A root gone for a moment drops the document with its buffer: once the root
/// is back the file answers and the editor can open the document again, from
/// any version.
#[test]
fn a_document_refused_while_its_root_is_gone_leaves_no_buffer_behind() {
    let workspace = workspace();
    let (host, _workspaces) = service();
    let editor = open(&host, workspace.path(), "Work");
    let change = V5LspNotification::DidChange {
        uri: BODY_URI.to_owned(),
        version: 6,
        changes: vec![V5LspContentChange {
            range: None,
            text: "Procedure Lost() EndProcedure\n".into(),
        }],
    };
    host.notify(
        &editor,
        V5LspNotification::DidOpen {
            uri: BODY_URI.to_owned(),
            language_id: "bsl".into(),
            version: 5,
            text: "Procedure Buffer() EndProcedure\n".into(),
        },
        soon(),
    )
    .unwrap();
    let source = workspace.path().join("src");
    let aside = workspace.path().join("src.aside");

    fs::rename(&source, &aside).unwrap();
    assert_eq!(
        host.notify(&editor, change, soon()),
        Err(V5LspFailure::WorkspaceUnavailable)
    );
    fs::rename(&aside, &source).unwrap();

    assert_eq!(content(&host, &editor, BODY_URI), text(DISK));
    open_buffer(&host, &editor, BODY_URI, "Procedure Again() EndProcedure\n");
    assert_eq!(
        content(&host, &editor, BODY_URI),
        text("Procedure Again() EndProcedure\n")
    );
}

/// A source set moved to another path is a new workspace identity. The session
/// lets go of the old actor, which would otherwise be retained by its buffer
/// for the life of the daemon, and reads modules under the new root.
#[test]
fn a_moved_source_root_releases_the_old_actor_and_serves_the_new_layout() {
    let workspace = workspace();
    let root = workspace.path();
    write_module(root, "src", "Other", "// other\n");
    let (host, workspaces) = service();
    let editor = open(&host, root, "Work");
    open_buffer(
        &host,
        &editor,
        BODY_URI,
        "Procedure Buffer() EndProcedure\n",
    );
    let before = workspaces.actor(root).unwrap();

    fs::rename(root.join("src"), root.join("moved")).unwrap();
    fs::write(root.join("source-root"), "moved").unwrap();

    assert_eq!(
        content(&host, &editor, BODY_URI),
        text("Procedure Buffer() EndProcedure\n")
    );
    assert!(
        before
            .documents()
            .session_documents(editor.as_str(), "main")
            .unwrap()
            .is_empty(),
        "the old actor keeps no buffer of this session"
    );
    assert_eq!(
        content(&host, &editor, "unica://work/main/CommonModule.Other.Body"),
        text("// other\n"),
        "a module with no buffer is read under the new root"
    );
}
