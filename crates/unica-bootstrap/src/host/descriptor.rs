/// Directory a host publishes for the private data of an installed plugin.
#[derive(Clone, Copy, Debug)]
pub(super) struct HostDataDir {
    /// Environment variable that carries the directory.
    pub(super) env: &'static str,
    /// Runtime cache location relative to that directory.
    pub(super) runtime_subdir: &'static [&'static str],
}

/// Directory a host keeps its own state in when it publishes no per-plugin data
/// directory.
#[derive(Clone, Copy, Debug)]
pub(super) struct HostHomeRoot {
    /// Environment variable that overrides the host home directory.
    pub(super) env: &'static str,
    /// Directory under the user home that holds the host state by default.
    pub(super) user_home_segment: &'static str,
    /// Runtime cache location relative to the host home directory.
    pub(super) runtime_subdir: &'static [&'static str],
}

/// A coding host that can install the Unica plugin.
///
/// Everything the bootstrap knows about a particular host lives in one of these
/// descriptors, so supporting one more host is adding an entry to [`KNOWN`]
/// rather than editing branches at the call sites.
#[derive(Clone, Copy, Debug)]
pub(super) struct PluginHost {
    /// Package directory that carries this host's `plugin.json`.
    pub(super) manifest_dir: &'static str,
    /// Whether the manifest must point at `skills/` explicitly. A host that
    /// scans the directory on its own would load it twice if it did.
    pub(super) expects_skills_pointer: bool,
    /// Whether the manifest names the MCP servers file. A host that reads the
    /// root `.mcp.json` on its own would start the same server twice if it did.
    pub(super) expects_manifest_servers: bool,
    /// Data directory the host publishes for its plugins, if any.
    pub(super) data_dir: Option<HostDataDir>,
    /// Home directory the host keeps its own state in, if any.
    pub(super) home_root: Option<HostHomeRoot>,
}

/// Known hosts, in the order the bootstrap consults them.
pub(super) const KNOWN: &[PluginHost] = &[CODEX, CLAUDE];

/// Codex does not scan `skills/`, so its manifest has to name the directory. It
/// publishes no per-plugin data directory, so the runtime cache is derived from
/// the Codex home directory instead.
const CODEX: PluginHost = PluginHost {
    manifest_dir: ".codex-plugin",
    expects_skills_pointer: true,
    expects_manifest_servers: true,
    data_dir: None,
    home_root: Some(HostHomeRoot {
        env: "CODEX_HOME",
        user_home_segment: ".codex",
        runtime_subdir: &["unica", "runtimes"],
    }),
};

/// Claude Code always scans `skills/` and always reads the root `.mcp.json`, so
/// naming either in the manifest would load it twice (ADR-0012). It hands every
/// plugin its own data directory, which is where the runtime cache belongs.
const CLAUDE: PluginHost = PluginHost {
    manifest_dir: ".claude-plugin",
    expects_skills_pointer: false,
    expects_manifest_servers: false,
    data_dir: Some(HostDataDir {
        env: "CLAUDE_PLUGIN_DATA",
        runtime_subdir: &["runtimes"],
    }),
    home_root: None,
};
