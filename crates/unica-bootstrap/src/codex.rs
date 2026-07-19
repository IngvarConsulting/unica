use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BootstrapError, Result};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceList {
    #[serde(default)]
    pub marketplaces: Vec<MarketplaceRecord>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceRecord {
    pub name: String,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub marketplace_source: MarketplaceSource,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceSource {
    pub source_type: String,
    pub source: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginList {
    #[serde(default)]
    pub installed: Vec<PluginRecord>,
    #[serde(default)]
    pub available: Vec<PluginRecord>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub plugin_id: String,
    pub name: String,
    pub marketplace_name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source: Option<Value>,
    #[serde(default)]
    pub marketplace_source: Option<MarketplaceSource>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDiscovery {
    pub marketplaces: MarketplaceList,
    pub plugins: PluginList,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub codex_home: Option<PathBuf>,
}

impl CommandSpec {
    pub fn codex(codex_home: &Path, args: &[&str]) -> Self {
        Self {
            program: "codex".to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            codex_home: Some(codex_home.to_path_buf()),
        }
    }

    pub fn git(args: &[&str]) -> Self {
        Self {
            program: "git".to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            codex_home: None,
        }
    }
}

pub trait CommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<String> {
        let mut process = Command::new(&command.program);
        process.args(&command.args);
        if let Some(codex_home) = &command.codex_home {
            process.env("CODEX_HOME", codex_home);
        }
        let output = process.output().map_err(|error| {
            BootstrapError::new(format!("failed to run {}: {error}", command.program))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BootstrapError::new(format!(
                "{} failed with exit code {}: {}",
                command.program,
                output.status.code().unwrap_or(1),
                redact(&stderr)
            )));
        }
        String::from_utf8(output.stdout).map_err(|error| {
            BootstrapError::new(format!("{} output is not UTF-8: {error}", command.program))
        })
    }
}

pub fn discover(runner: &dyn CommandRunner, codex_home: &Path) -> Result<CodexDiscovery> {
    let marketplaces = runner.run(&CommandSpec::codex(
        codex_home,
        &["plugin", "marketplace", "list", "--json"],
    ))?;
    let plugins = runner.run(&CommandSpec::codex(
        codex_home,
        &["plugin", "list", "--available", "--json"],
    ))?;
    Ok(CodexDiscovery {
        marketplaces: serde_json::from_str(&marketplaces).map_err(|error| {
            BootstrapError::new(format!("invalid codex marketplace JSON: {error}"))
        })?,
        plugins: serde_json::from_str(&plugins)
            .map_err(|error| BootstrapError::new(format!("invalid codex plugin JSON: {error}")))?,
    })
}

fn redact(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("token")
                || lower.contains("authorization")
                || lower.contains("secret")
            {
                "[redacted]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
