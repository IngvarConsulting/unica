//! Platform XML source-family adapter.
//!
//! The public boundary is a factory and registration composed from
//! format-neutral core ports.
//!
//! ```
//! use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
//! use unica_format_core::source::SourceFamily;
//!
//! let registration = PlatformXmlAdapterFactory::new().registration();
//! assert_eq!(registration.manifest.source_family, SourceFamily::PlatformXml);
//! ```
//!
//! Version modules and native parser types are deliberately private.
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::v2_20;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::native_model::PlatformXmlNativeSnapshot;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::projector::project;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::versions::v2_20::schema::MetadataClassProfile;
//! ```
//!
//! ```compile_fail
//! use unica_adapter_platform_xml::owner::Owner;
//! ```

mod factory;
mod guards;
mod owner;
mod publication;
mod validation;
mod versions;

pub use factory::PlatformXmlAdapterFactory;

mod domain {
    pub(crate) use unica_format_core::limits as navigation_limits;
    pub(crate) use unica_format_core::navigation;
    pub(crate) use unica_format_core::source as source_adapters;

    pub(crate) mod identifiers {
        pub(crate) fn is_1c_identifier(value: &str) -> bool {
            let mut chars = value.chars();
            let Some(first) = chars.next() else {
                return false;
            };
            is_1c_identifier_start(first) && chars.all(is_1c_identifier_part)
        }

        fn is_1c_identifier_start(ch: char) -> bool {
            ch == '_'
                || ch.is_ascii_alphabetic()
                || ('А'..='Я').contains(&ch)
                || ('а'..='я').contains(&ch)
                || ch == 'Ё'
                || ch == 'ё'
        }

        fn is_1c_identifier_part(ch: char) -> bool {
            is_1c_identifier_start(ch) || ch.is_ascii_digit()
        }
    }
}

mod infrastructure {
    #[cfg(test)]
    pub(crate) mod source_adapters {
        pub(crate) mod platform_xml {
            pub(crate) use crate::versions::v2_20::*;
        }
    }

    #[cfg(test)]
    pub(crate) mod platform {
        pub(crate) mod filesystem {
            use std::{io, path::Path};

            #[cfg(unix)]
            pub(crate) fn create_file_symlink_for_test(
                target: impl AsRef<Path>,
                link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                Some(std::os::unix::fs::symlink(target, link))
            }

            #[cfg(unix)]
            pub(crate) fn create_dir_symlink_for_test(
                target: impl AsRef<Path>,
                link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                Some(std::os::unix::fs::symlink(target, link))
            }

            #[cfg(windows)]
            pub(crate) fn create_file_symlink_for_test(
                target: impl AsRef<Path>,
                link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                Some(std::os::windows::fs::symlink_file(target, link))
            }

            #[cfg(windows)]
            pub(crate) fn create_dir_symlink_for_test(
                target: impl AsRef<Path>,
                link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                Some(std::os::windows::fs::symlink_dir(target, link))
            }

            #[cfg(not(any(unix, windows)))]
            pub(crate) fn create_file_symlink_for_test(
                _target: impl AsRef<Path>,
                _link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                None
            }

            #[cfg(not(any(unix, windows)))]
            pub(crate) fn create_dir_symlink_for_test(
                _target: impl AsRef<Path>,
                _link: impl AsRef<Path>,
            ) -> Option<io::Result<()>> {
                None
            }
        }
    }
}

#[cfg(test)]
mod certification;
