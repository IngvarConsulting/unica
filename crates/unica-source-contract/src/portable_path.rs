//! A source path as an agent reads it: one spelling on every host.
//!
//! Diagnostics name workspace-relative files. `Path::display` joins components
//! with the host separator, so the same file read `Catalogs\Products.xml` on
//! Windows and `Catalogs/Products.xml` elsewhere; an agent cannot reuse such a
//! name across hosts. `portable()` joins the components with `/` instead.

use std::fmt;
use std::path::{Component, Path};

pub struct PortablePath<'a>(&'a Path);

pub trait PortablePathExt {
    fn portable(&self) -> PortablePath<'_>;
}

impl PortablePathExt for Path {
    fn portable(&self) -> PortablePath<'_> {
        PortablePath(self)
    }
}

impl fmt::Display for PortablePath<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for component in self.0.components() {
            match component {
                // A drive or UNC prefix is spelled with `/` as well.
                Component::Prefix(prefix) => {
                    formatter
                        .write_str(&prefix.as_os_str().to_string_lossy().replace('\\', "/"))?;
                    continue;
                }
                Component::RootDir => {
                    formatter.write_str("/")?;
                    separator = "";
                    continue;
                }
                Component::CurDir => {
                    formatter.write_str(separator)?;
                    formatter.write_str(".")?;
                }
                Component::ParentDir => {
                    formatter.write_str(separator)?;
                    formatter.write_str("..")?;
                }
                Component::Normal(name) => {
                    formatter.write_str(separator)?;
                    formatter.write_str(&name.to_string_lossy())?;
                }
            }
            separator = "/";
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PortablePathExt;
    use std::path::{Path, PathBuf};

    #[test]
    fn components_are_joined_with_a_forward_slash_on_every_host() {
        // `push` joins with the host separator: on Windows this path holds
        // backslashes, and the portable spelling must not.
        let mut path = PathBuf::from("DefinedTypes");
        path.push("Inner.xml");
        assert_eq!(path.portable().to_string(), "DefinedTypes/Inner.xml");
        let nested = Path::new("Catalogs")
            .join("Products")
            .join("Ext")
            .join("Form.xml");
        assert_eq!(
            nested.portable().to_string(),
            "Catalogs/Products/Ext/Form.xml"
        );
        assert_eq!(
            Path::new("Configuration.xml").portable().to_string(),
            "Configuration.xml"
        );
        assert_eq!(Path::new("").portable().to_string(), "");
        assert_eq!(Path::new("a/./b/../c").portable().to_string(), "a/b/../c");
    }

    #[cfg(windows)]
    #[test]
    fn windows_separators_are_spelled_as_forward_slashes() {
        assert_eq!(
            Path::new(r"Catalogs\Products.xml").portable().to_string(),
            "Catalogs/Products.xml"
        );
        assert_eq!(
            Path::new(r"C:\work\src").portable().to_string(),
            "C:/work/src"
        );
        assert_eq!(
            Path::new(r"\\srv\share\src").portable().to_string(),
            "//srv/share/src"
        );
    }
}
