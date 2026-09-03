use crate::domain::events::{DomainEvent, DomainEventKind};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EagerCacheKey {
    Workspace,
    Metadata,
    Rights,
    Subsystem,
}

impl EagerCacheKey {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace_graph",
            Self::Metadata => "metadata_graph",
            Self::Rights => "rights_graph",
            Self::Subsystem => "subsystem_graph",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "workspace_graph" => Some(Self::Workspace),
            "metadata_graph" => Some(Self::Metadata),
            "rights_graph" => Some(Self::Rights),
            "subsystem_graph" => Some(Self::Subsystem),
            _ => None,
        }
    }
}

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
    #[serde(skip)]
    pub(crate) publication_warnings: Vec<String>,
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
            DomainEventKind::ModuleChanged | DomainEventKind::SourceResourcesReplaced => {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every event kind, listed once. `index_of` below is what keeps the list
    /// honest: it has no wildcard arm, so adding a variant stops compilation
    /// until the new event is given an invalidation rule and a place here.
    const ALL_KINDS: [DomainEventKind; 13] = [
        DomainEventKind::ConfigXmlChanged,
        DomainEventKind::CfeChanged,
        DomainEventKind::MetadataChanged,
        DomainEventKind::FormChanged,
        DomainEventKind::ModuleChanged,
        DomainEventKind::RoleChanged,
        DomainEventKind::DcsChanged,
        DomainEventKind::MxlChanged,
        DomainEventKind::SubsystemChanged,
        DomainEventKind::TemplateChanged,
        DomainEventKind::SourceSetChanged,
        DomainEventKind::BuildCompleted,
        DomainEventKind::SourceResourcesReplaced,
    ];

    fn index_of(kind: DomainEventKind) -> usize {
        match kind {
            DomainEventKind::ConfigXmlChanged => 0,
            DomainEventKind::CfeChanged => 1,
            DomainEventKind::MetadataChanged => 2,
            DomainEventKind::FormChanged => 3,
            DomainEventKind::ModuleChanged => 4,
            DomainEventKind::RoleChanged => 5,
            DomainEventKind::DcsChanged => 6,
            DomainEventKind::MxlChanged => 7,
            DomainEventKind::SubsystemChanged => 8,
            DomainEventKind::TemplateChanged => 9,
            DomainEventKind::SourceSetChanged => 10,
            DomainEventKind::BuildCompleted => 11,
            DomainEventKind::SourceResourcesReplaced => 12,
        }
    }

    fn event(kind: DomainEventKind) -> DomainEvent {
        DomainEvent {
            kind,
            artifact: "fixture".to_string(),
            details: None,
        }
    }

    fn names(set: &BTreeSet<String>) -> Vec<&str> {
        set.iter().map(String::as_str).collect()
    }

    /// How many variants the enum has. Deriving this from `ALL_KINDS.len()`
    /// would make the check below a tautology — both sides would shrink
    /// together and a forgotten kind would pass. Adding a variant forces a new
    /// arm in `index_of`, and this constant is what then fails until the kind
    /// reaches `ALL_KINDS` too.
    const EXPECTED_KIND_COUNT: usize = 13;

    #[test]
    fn the_kind_list_covers_the_whole_enum() {
        assert_eq!(ALL_KINDS.len(), EXPECTED_KIND_COUNT);
        let mut seen = ALL_KINDS.map(index_of).to_vec();
        seen.sort_unstable();
        assert_eq!(seen, (0..EXPECTED_KIND_COUNT).collect::<Vec<_>>());
    }

    /// ADR-0022: the bounded resource writer replaces one proven BSL module, so
    /// the BSL caches are the only ones that can have gone stale — and nothing
    /// is rebuilt eagerly, because the caller asked to write, not to warm.
    #[test]
    fn replacing_a_source_resource_invalidates_only_the_two_bsl_caches() {
        let impact = CacheImpact::from_events(&[event(DomainEventKind::SourceResourcesReplaced)]);

        assert_eq!(names(&impact.invalidated), ["bsl_diagnostics", "bsl_index"]);
        assert!(
            impact.eager_refresh.is_empty(),
            "a resource replacement rebuilds nothing eagerly: {:?}",
            impact.eager_refresh
        );
    }

    /// A module edit reaches the same two caches by a different route, so the
    /// claim above is about the pair, not about one event that happens to
    /// match it today.
    #[test]
    fn a_module_change_invalidates_the_same_two_caches() {
        let impact = CacheImpact::from_events(&[event(DomainEventKind::ModuleChanged)]);

        assert_eq!(names(&impact.invalidated), ["bsl_diagnostics", "bsl_index"]);
        assert!(
            impact.eager_refresh.is_empty(),
            "{:?}",
            impact.eager_refresh
        );
    }

    /// Refreshing a cache that was never invalidated would rebuild something
    /// already fresh; the report would then name work that did not need doing.
    #[test]
    fn no_event_refreshes_a_cache_it_did_not_invalidate() {
        for kind in ALL_KINDS {
            let impact = CacheImpact::from_events(&[event(kind)]);
            let dangling = impact
                .eager_refresh
                .difference(&impact.invalidated)
                .cloned()
                .collect::<Vec<_>>();
            assert!(
                dangling.is_empty(),
                "{kind:?} refreshes caches it never invalidated: {dangling:?}"
            );
        }
    }

    /// An event that invalidates nothing would be a change no consumer hears
    /// about, which is the failure this whole model exists to prevent.
    #[test]
    fn every_event_invalidates_at_least_one_cache() {
        for kind in ALL_KINDS {
            let impact = CacheImpact::from_events(&[event(kind)]);
            assert!(
                !impact.invalidated.is_empty(),
                "{kind:?} invalidates nothing"
            );
        }
    }

    /// One call reporting several events must answer for all of them: a
    /// mutation that emits two events cannot leave one of their caches warm.
    #[test]
    fn from_events_unions_the_impact_of_every_event() {
        let combined = CacheImpact::from_events(&[
            event(DomainEventKind::SourceResourcesReplaced),
            event(DomainEventKind::RoleChanged),
        ]);

        assert_eq!(
            names(&combined.invalidated),
            [
                "bsl_diagnostics",
                "bsl_index",
                "metadata_graph",
                "rights_graph"
            ]
        );
        assert_eq!(
            names(&combined.eager_refresh),
            ["metadata_graph", "rights_graph"]
        );
    }

    #[test]
    fn no_events_leave_the_impact_empty() {
        let impact = CacheImpact::from_events(&[]);

        assert!(impact.invalidated.is_empty());
        assert!(impact.eager_refresh.is_empty());
    }

    #[test]
    fn typed_event_cache_impact_catalog_is_closed() {
        the_kind_list_covers_the_whole_enum();
        every_event_invalidates_at_least_one_cache();
        no_event_refreshes_a_cache_it_did_not_invalidate();
        from_events_unions_the_impact_of_every_event();
        no_events_leave_the_impact_empty();
    }
}
