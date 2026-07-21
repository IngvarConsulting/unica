use crate::domain::events::{DomainEvent, DomainEventKind};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    pub mode: String,
    pub root: String,
    pub workspace_epoch: u64,
    pub events: Vec<String>,
    pub invalidated: Vec<String>,
    pub refreshed: Vec<String>,
    pub lazy_rebuilt: Vec<String>,
    pub stale: Vec<String>,
    pub fresh: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheAccess {
    pub reads: &'static [&'static str],
    pub writes: &'static [&'static str],
}

#[derive(Debug, Clone, Default)]
pub struct CacheImpact {
    pub invalidated: BTreeSet<String>,
    pub eager_refresh: BTreeSet<String>,
}

impl CacheImpact {
    pub fn from_events(events: &[DomainEvent]) -> Self {
        let mut impact = Self::default();
        for event in events {
            impact.add_event(event.kind);
        }
        impact
    }

    fn add_event(&mut self, event: DomainEventKind) {
        match event {
            DomainEventKind::ConfigXmlChanged
            | DomainEventKind::MetadataChanged
            | DomainEventKind::CfeChanged => {
                self.invalidate(["workspace_graph", "metadata_graph", "bsl_diagnostics"]);
                self.refresh(["workspace_graph", "metadata_graph"]);
            }
            DomainEventKind::FormChanged => {
                self.invalidate(["metadata_graph", "form_graph", "bsl_diagnostics"]);
                self.refresh(["metadata_graph"]);
            }
            DomainEventKind::ModuleChanged => {
                self.invalidate(["bsl_index", "bsl_diagnostics"]);
            }
            DomainEventKind::RoleChanged => {
                self.invalidate(["metadata_graph", "rights_graph", "bsl_diagnostics"]);
                self.refresh(["metadata_graph", "rights_graph"]);
            }
            DomainEventKind::DcsChanged => {
                self.invalidate(["metadata_graph", "dcs_graph", "bsl_diagnostics"]);
                self.refresh(["metadata_graph"]);
            }
            DomainEventKind::MxlChanged => {
                self.invalidate(["metadata_graph", "mxl_graph"]);
                self.refresh(["metadata_graph"]);
            }
            DomainEventKind::SubsystemChanged => {
                self.invalidate([
                    "metadata_graph",
                    "subsystem_graph",
                    "command_interface_graph",
                ]);
                self.refresh(["metadata_graph", "subsystem_graph"]);
            }
            DomainEventKind::TemplateChanged => {
                self.invalidate(["metadata_graph", "template_graph"]);
                self.refresh(["metadata_graph"]);
            }
            DomainEventKind::SourceSetChanged | DomainEventKind::BuildCompleted => {
                self.invalidate([
                    "workspace_graph",
                    "metadata_graph",
                    "form_graph",
                    "bsl_index",
                    "bsl_diagnostics",
                ]);
                self.refresh(["workspace_graph", "metadata_graph"]);
            }
        }
    }

    fn invalidate<const N: usize>(&mut self, names: [&'static str; N]) {
        for name in names {
            self.invalidated.insert(name.to_string());
        }
    }

    fn refresh<const N: usize>(&mut self, names: [&'static str; N]) {
        for name in names {
            self.eager_refresh.insert(name.to_string());
        }
    }
}

pub fn path_for_report(path: &Path) -> String {
    path.display().to_string()
}
