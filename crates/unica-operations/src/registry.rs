use crate::{ObservationLookup, OperationDescriptor};

use unica_address::address::QualifiedAddress;
use unica_operation_contract::OperationRef;
use unica_source_contract::context::{OperationSite, RegistryContext};

#[derive(Debug, Clone, Copy)]
pub struct OperationRegistry {
    descriptors: &'static [OperationDescriptor],
    context: RegistryContext,
}
impl OperationRegistry {
    pub const fn new(
        descriptors: &'static [OperationDescriptor],
        context: RegistryContext,
    ) -> Self {
        Self {
            descriptors,
            context,
        }
    }
    pub const fn with_context(self, context: RegistryContext) -> Self {
        Self { context, ..self }
    }
    pub const fn context(self) -> RegistryContext {
        self.context
    }
    pub const fn descriptors(self) -> &'static [OperationDescriptor] {
        self.descriptors
    }
    pub fn names(self) -> Vec<&'static str> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.name())
            .collect()
    }
    pub fn named(self, name: &str) -> impl Iterator<Item = OperationDescriptor> + '_ {
        self.descriptors
            .iter()
            .copied()
            .filter(move |descriptor| descriptor.name() == name)
    }
    pub fn lookup(self, name: &str) -> Option<OperationDescriptor> {
        self.named(name).next()
    }
    pub fn lookup_at(self, name: &str, site: &OperationSite<'_>) -> Option<OperationDescriptor> {
        self.named(name)
            .find(|descriptor| descriptor.applies_at(site))
            .or_else(|| self.lookup(name))
    }
    pub fn nearest_ancestor_with_observations(
        self,
        name: &str,
        site: &OperationSite<'_>,
        observations: ObservationLookup<'_>,
    ) -> Option<(OperationDescriptor, QualifiedAddress)> {
        self.named(name)
            .filter_map(|descriptor| {
                descriptor
                    .nearest_ancestor_with_observations(site, self.context, observations)
                    .map(|target| (descriptor, target))
            })
            .max_by_key(|(_, target)| target.segments().len())
    }
    pub fn copyable_skeletons(self, site: &OperationSite<'_>) -> Vec<OperationRef> {
        self.descriptors
            .iter()
            .copied()
            .filter(|descriptor| descriptor.applies_at(site))
            .map(OperationDescriptor::copyable_skeleton)
            .collect()
    }
}
