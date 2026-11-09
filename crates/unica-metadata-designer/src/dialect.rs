// module: private
//! The Designer export dialect of the metadata codec: what this codec knows
//! about the versions of the files it reads and writes. A new export format
//! version edits this module, not the readers and writers that ask it.
pub(crate) mod root_version;
