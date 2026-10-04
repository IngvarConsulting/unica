#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceStateScope {
    /// v0.12 compatibility namespace: only canonical workspace/source paths
    /// participate in persisted state identity.
    LegacyPhysical,
    /// v0.13 actor namespace: the digest covers the complete structural actor
    /// identity and is bounded to lowercase SHA-256 text.
    Scoped(ScopedStateDigest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedStateDigest(String);

impl WorkspaceStateScope {
    pub(crate) fn scoped_digest(&self) -> Option<&str> {
        match self {
            Self::LegacyPhysical => None,
            Self::Scoped(digest) => Some(&digest.0),
        }
    }

    pub(crate) fn scoped_sha256(digest: String) -> Result<Self, String> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "workspace actor state scope must be a lowercase SHA-256 digest".to_string(),
            );
        }
        Ok(Self::Scoped(ScopedStateDigest(digest)))
    }
}
