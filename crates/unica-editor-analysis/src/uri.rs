//! `unica://<space>/<source set>/<address>` document identities.
//!
//! The server names each workspace folder of one LSP connection a space: the
//! folder name in lower case. Long or shared names get a short hash of the
//! folder path and are shortened to fit the protocol. Editors lower-case a URI
//! authority (VS Code does), so a space is named and matched in lower case.
//! A URI of a space this connection was not given is refused, so
//! two workspaces that both have a source set `main` never mix. The address
//! follows the Unica logical address grammar; a document is a module body,
//! addressed as MCP addresses it (`CommonModule.X.Body`,
//! `Catalog.X.Module.Object.Body`).

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;
use unica_lsp_contract::protocol::MAX_LSP_SPACE_BYTES;

pub(crate) const SCHEME: &str = "unica";

/// Everything a URI component keeps literally is RFC 3986 unreserved.
const COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// One workspace folder the client opened, as `initialize` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFolder {
    pub name: String,
    pub root: PathBuf,
}

/// The spaces of one LSP connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceRegistry {
    spaces: BTreeMap<String, PathBuf>,
}

/// A document of one space: a source set and a logical address in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentUri {
    pub space: String,
    pub address: String,
}

impl DocumentUri {
    /// The module whose body this document is, if it is one.
    pub fn module(&self) -> Option<crate::ModuleDocument> {
        crate::ModuleDocument::of_document(&self.address)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    /// Not a `unica://` URI with a space, a source set and an address.
    Malformed(&'static str),
    /// The space was not given to this connection.
    ForeignSpace(String),
    /// The address is not a Unica logical address.
    Address(String),
}

impl SpaceRegistry {
    /// Name a space for each folder. Folders are taken in the order given;
    /// Long or shared names get a short hash of each path.
    pub fn from_folders(folders: &[WorkspaceFolder]) -> Self {
        let mut uses = BTreeMap::<String, usize>::new();
        for folder in folders {
            *uses.entry(space_name(folder)).or_default() += 1;
        }
        let mut spaces = BTreeMap::new();
        for folder in folders {
            let name = space_name(folder);
            let space = if name.len() > MAX_LSP_SPACE_BYTES || uses[&name] > 1 {
                let hash = short_path_hash(&folder.root);
                let mut suffix = format!("-{hash}");
                let mut index = 0;
                loop {
                    let budget = MAX_LSP_SPACE_BYTES - suffix.len();
                    let prefix = &name[..name.floor_char_boundary(budget)];
                    let candidate = format!("{prefix}{suffix}");
                    if !uses.contains_key(&candidate)
                        && spaces
                            .get(&candidate)
                            .is_none_or(|root| root == &folder.root)
                    {
                        break candidate;
                    }
                    index += 1;
                    suffix = format!("-{hash}-{index}");
                }
            } else {
                name
            };
            spaces.entry(space).or_insert_with(|| folder.root.clone());
        }
        Self { spaces }
    }

    pub(crate) fn root(&self, space: &str) -> Option<&Path> {
        self.spaces.get(space).map(PathBuf::as_path)
    }

    pub(crate) fn spaces(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.spaces
            .iter()
            .map(|(space, root)| (space.as_str(), root.as_path()))
    }

    /// The space whose root is `root`.
    pub fn space_of_root(&self, root: &Path) -> Option<&str> {
        self.spaces
            .iter()
            .find(|(_, known)| known.as_path() == root)
            .map(|(space, _)| space.as_str())
    }

    /// The `unica://` URI of `address` in `space`.
    pub fn format(&self, document: &DocumentUri) -> Result<String, UriError> {
        if !self.spaces.contains_key(&document.space) {
            return Err(UriError::ForeignSpace(document.space.clone()));
        }
        let address = QualifiedAddress::parse(&document.address)
            .map_err(|error| UriError::Address(error.to_string()))?;
        let rendered = address.to_string();
        let (source_set, logical) = rendered
            .split_once(':')
            .ok_or(UriError::Malformed("address has no source set"))?;
        Ok(format!(
            "{SCHEME}://{}/{}/{}",
            utf8_percent_encode(&document.space, COMPONENT),
            utf8_percent_encode(source_set, COMPONENT),
            utf8_percent_encode(logical, COMPONENT),
        ))
    }

    /// The space and address a `unica://` URI names.
    pub fn parse(&self, uri: &str) -> Result<DocumentUri, UriError> {
        let rest = uri
            .strip_prefix(SCHEME)
            .and_then(|rest| rest.strip_prefix("://"))
            .ok_or(UriError::Malformed("not a unica:// URI"))?;
        if rest.contains(['?', '#']) {
            return Err(UriError::Malformed(
                "a document URI has no query or fragment",
            ));
        }
        let mut parts = rest.split('/');
        let (Some(space), Some(source_set), Some(logical), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(UriError::Malformed(
                "expected unica://<space>/<source set>/<address>",
            ));
        };
        let decode = |part: &str| {
            percent_decode_str(part)
                .decode_utf8()
                .map(|text| text.into_owned())
                .map_err(|_| UriError::Malformed("a URI component is not UTF-8"))
        };
        let (space, source_set, logical) = (decode(space)?, decode(source_set)?, decode(logical)?);
        if space.is_empty() || source_set.is_empty() || logical.is_empty() {
            return Err(UriError::Malformed(
                "expected unica://<space>/<source set>/<address>",
            ));
        }
        let space = space.to_lowercase();
        if !self.spaces.contains_key(&space) {
            return Err(UriError::ForeignSpace(space));
        }
        let address = QualifiedAddress::parse(&format!("{source_set}:{logical}"))
            .map_err(|error| UriError::Address(error.to_string()))?;
        Ok(DocumentUri {
            space,
            address: address.to_string(),
        })
    }
}

fn space_name(folder: &WorkspaceFolder) -> String {
    let name = folder.name.trim();
    if name.is_empty() {
        folder
            .root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("workspace")
            .to_lowercase()
    } else {
        name.to_lowercase()
    }
}

fn short_path_hash(root: &Path) -> String {
    let digest = Sha256::digest(root.to_string_lossy().as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, root: &str) -> WorkspaceFolder {
        WorkspaceFolder {
            name: name.to_owned(),
            root: PathBuf::from(root),
        }
    }

    #[test]
    fn two_spaces_with_source_set_main_do_not_mix() {
        let registry =
            SpaceRegistry::from_folders(&[folder("УТ", "/work/ut"), folder("БП", "/work/bp")]);
        let ut = DocumentUri {
            space: "ут".to_owned(),
            address: "main:CommonModule.ОбщегоНазначения".to_owned(),
        };
        let bp = DocumentUri {
            space: "бп".to_owned(),
            ..ut.clone()
        };
        let ut_uri = registry.format(&ut).unwrap();
        let bp_uri = registry.format(&bp).unwrap();
        assert_ne!(ut_uri, bp_uri);
        assert!(ut_uri.starts_with("unica://%D1%83%D1%82/main/CommonModule."));
        assert_eq!(registry.parse(&ut_uri).unwrap(), ut);
        // An editor that upper-cases the authority still names the space.
        assert_eq!(
            registry
                .parse(&ut_uri.replace("%D1%83%D1%82", "%D0%A3%D0%A2"))
                .unwrap(),
            ut
        );
        assert_eq!(registry.parse(&bp_uri).unwrap(), bp);

        // A space of another connection is refused, never resolved here.
        let other = SpaceRegistry::from_folders(&[folder("УТ", "/work/ut")]);
        assert_eq!(
            other.parse(&bp_uri),
            Err(UriError::ForeignSpace("бп".to_owned()))
        );
        assert_eq!(
            other.format(&bp),
            Err(UriError::ForeignSpace("бп".to_owned()))
        );
    }

    #[test]
    fn a_shared_folder_name_gets_a_short_hash_of_its_path() {
        let registry =
            SpaceRegistry::from_folders(&[folder("src", "/a/src"), folder("SRC", "/b/src")]);
        let spaces = registry
            .spaces()
            .map(|(space, _)| space.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(spaces.len(), 2);
        assert!(spaces
            .iter()
            .all(|space| space.starts_with("src-") && space.len() == 12));
        assert_ne!(spaces[0], spaces[1]);
        let space = registry.space_of_root(Path::new("/b/src")).unwrap();
        assert_eq!(registry.root(space), Some(Path::new("/b/src")));
    }

    #[test]
    fn long_names_and_shared_names_fit_the_protocol() {
        use unica_lsp_contract::protocol::V5LspWorkspaceSpace;

        let long = "я".repeat(MAX_LSP_SPACE_BYTES);
        let at_limit = "я".repeat(MAX_LSP_SPACE_BYTES / 2);
        let reserved = format!("{}-{}", "я".repeat(27), short_path_hash(Path::new("/a")));
        let registry = SpaceRegistry::from_folders(&[
            folder(&long, "/a"),
            folder(&long, "/b"),
            folder(&at_limit, "/c"),
            folder(&at_limit, "/d"),
            folder(&reserved, "/e"),
        ]);
        assert_eq!(registry.spaces().count(), 5);
        assert_eq!(registry.root(&reserved), Some(Path::new("/e")));
        for (space, root) in registry.spaces() {
            V5LspWorkspaceSpace::opened("file:///work", space)
                .validate()
                .unwrap();
            let document = DocumentUri {
                space: space.to_owned(),
                address: "main:CommonModule.А.Body".to_owned(),
            };
            assert_eq!(
                registry
                    .parse(&registry.format(&document).unwrap())
                    .unwrap(),
                document
            );
            assert_eq!(registry.space_of_root(root), Some(space));
        }
    }

    #[test]
    fn module_addresses_round_trip_and_malformed_uris_are_refused() {
        let registry = SpaceRegistry::from_folders(&[folder("ws", "/ws")]);
        for address in [
            "main:Catalog.Валюты.Module.Object",
            "ext:Document.Заказ.Form.ФормаДокумента.Module.Form",
            r"main:CommonTemplate.S.Parameter.A\.B",
            "main:CommonModule.ОбщегоНазначения.Body",
        ] {
            let document = DocumentUri {
                space: "ws".to_owned(),
                address: address.to_owned(),
            };
            let uri = registry.format(&document).unwrap();
            assert!(!uri.contains('\\'), "{uri}");
            assert_eq!(registry.parse(&uri).unwrap(), document, "{uri}");
        }
        for uri in [
            "file:///ws/main/CommonModule.X",
            "unica://ws/main",
            "unica://ws/main/CommonModule.X/extra",
            "unica://ws/main/CommonModule.X?x=1",
            "unica://ws//CommonModule.X",
        ] {
            assert!(
                matches!(registry.parse(uri), Err(UriError::Malformed(_))),
                "{uri}"
            );
        }
        assert!(matches!(
            registry.parse("unica://ws/main/..%2F"),
            Err(UriError::Address(_))
        ));
    }
}
