use crate::domain::discovery::{
    BslFact, DefinitionFact, DiscoveryQuery, FactBatch, FormFact, MetadataFact, ProviderOutcome,
    RuntimeFlowFact, SourceInventory, SupportFact,
};

pub(crate) trait SourceInventoryPort {
    fn inventory(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<SourceInventory>;
}

pub(crate) trait MetadataCatalogPort {
    fn metadata(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<MetadataFact>>;
}

pub(crate) trait ManagedFormPort {
    fn forms(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<FormFact>>;
}

pub(crate) trait BslSearchPort {
    fn search(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<BslFact>>;
}

pub(crate) trait DefinitionPort {
    fn definitions(&self, query: &DiscoveryQuery<'_>)
        -> ProviderOutcome<FactBatch<DefinitionFact>>;
}

pub(crate) trait RuntimeFlowPort {
    fn runtime_flow(
        &self,
        query: &DiscoveryQuery<'_>,
    ) -> ProviderOutcome<FactBatch<RuntimeFlowFact>>;
}

pub(crate) trait SupportStatePort {
    fn support(
        &self,
        query: &DiscoveryQuery<'_>,
        files: &SourceInventory,
    ) -> ProviderOutcome<FactBatch<SupportFact>>;
}

pub(crate) struct DiscoveryPorts<'a> {
    pub source_inventory: &'a dyn SourceInventoryPort,
    pub metadata_catalog: &'a dyn MetadataCatalogPort,
    pub managed_forms: &'a dyn ManagedFormPort,
    pub bsl_search: &'a dyn BslSearchPort,
    pub definitions: &'a dyn DefinitionPort,
    pub runtime_flow: &'a dyn RuntimeFlowPort,
    pub support_state: &'a dyn SupportStatePort,
}
