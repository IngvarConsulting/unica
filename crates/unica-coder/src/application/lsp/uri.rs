//! `unica://<space>/<source set>/<address>` document identities.
//!
//! The server names each workspace folder of one LSP connection a space: the
//! folder name in lower case, or that with a short hash of its path when two
//! folders share it. Editors lower-case a URI authority (VS Code does), so a
//! space is named and matched in lower case. A URI of a space this connection was not given is refused, so
//! two workspaces that both have a source set `main` never mix. The address
//! follows the Unica logical address grammar (`CommonModule.X`,
//! `Catalog.X.Module.Object`, ...).

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use unica_address::address::QualifiedAddress;

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
pub(crate) struct WorkspaceFolder {
    pub(crate) name: String,
    pub(crate) root: PathBuf,
}

/// The spaces of one LSP connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpaceRegistry {
    spaces: BTreeMap<String, PathBuf>,
}

/// A document of one space: a source set and a logical address in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DocumentUri {
    pub(crate) space: String,
    pub(crate) address: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UriError {
    /// Not a `unica://` URI with a space, a source set and an address.
    Malformed(&'static str),
    /// The space was not given to this connection.
    ForeignSpace(String),
    /// The address is not a Unica logical address.
    Address(String),
}

impl SpaceRegistry {
    /// Name a space for each folder. Folders are taken in the order given;
    /// a name shared by several folders gets a short hash of each path.
    pub(crate) fn from_folders(folders: &[WorkspaceFolder]) -> Self {
        let mut uses = BTreeMap::<String, usize>::new();
        for folder in folders {
            *uses.entry(space_name(folder)).or_default() += 1;
        }
        let mut spaces = BTreeMap::new();
        for folder in folders {
            let name = space_name(folder);
            let space = if uses[&name] > 1 {
                format!("{name}-{}", short_path_hash(&folder.root))
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
    pub(crate) fn space_of_root(&self, root: &Path) -> Option<&str> {
        self.spaces
            .iter()
            .find(|(_, known)| known.as_path() == root)
            .map(|(space, _)| space.as_str())
    }

    /// The `unica://` URI of `address` in `space`.
    pub(crate) fn format(&self, document: &DocumentUri) -> Result<String, UriError> {
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
    pub(crate) fn parse(&self, uri: &str) -> Result<DocumentUri, UriError> {
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
    fn module_addresses_round_trip_and_malformed_uris_are_refused() {
        let registry = SpaceRegistry::from_folders(&[folder("ws", "/ws")]);
        for address in [
            "main:Catalog.Валюты.Module.Object",
            "ext:Document.Заказ.Form.ФормаДокумента.Module.Form",
            r"main:CommonTemplate.S.Parameter.A\.B",
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
