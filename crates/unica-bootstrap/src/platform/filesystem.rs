/// Whether a `file://` URI with a host names a native directory: on Windows
/// the host is the server of a UNC share (`\\server\share`).
pub(crate) const fn file_uri_host_names_share() -> bool {
    cfg!(windows)
}
