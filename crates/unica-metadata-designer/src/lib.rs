//! Designer codecs for metadata and its supplier-support state.
//!
//! The modules are the `*.rs` files of this directory. `build.rs` lists them, so
//! a new module adds a file and does not edit a shared list.
include!(concat!(env!("OUT_DIR"), "/metadata_designer_modules.rs"));
declare_modules!();
