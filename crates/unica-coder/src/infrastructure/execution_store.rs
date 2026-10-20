//! Physical prototype for the execution model; no productive runtime cutover.
#![allow(dead_code)]
pub(crate) mod codec;
pub(crate) mod file;
pub(crate) mod namespace;
pub(crate) mod owner;

pub(crate) use file::{CommitStep, FileAggregateStore, ReadStep};

use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    ForeignEpoch,
    UnownedRequest,
    Frozen,
    IdentityCollision,
}

#[derive(Debug)]
pub(crate) enum OpenError {
    Io(io::Error),
    InvalidInventory,
    IdentityCollision,
    Busy,
    IncompleteBootstrap,
}
impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "execution store I/O: {error}"),
            Self::InvalidInventory => f.write_str("invalid execution inventory"),
            Self::IdentityCollision => f.write_str("execution identity collision"),
            Self::Busy => f.write_str("execution authority is busy or frozen"),
            Self::IncompleteBootstrap => f.write_str("incomplete execution bootstrap"),
        }
    }
}
impl std::error::Error for OpenError {}
