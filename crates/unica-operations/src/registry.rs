use crate::recovery::{
    no_member_lookup, retired_argument_redirect_in, ArgumentRedirect, BoundRetiredArgument,
    MemberPathLookup, RetiredOperation,
};
use crate::{ObservationLookup, OperationDescriptor};
use serde_json::{Map, Value};
use unica_address::address::QualifiedAddress;
use unica_operation_contract::OperationRef;
use unica_source_contract::context::{OperationSite, RegistryContext};

#[derive(Debug, Clone, Copy)]
pub struct OperationRegistry {
    descriptors: &'static [OperationDescriptor],
    retired: &'static [RetiredOperation],
    context: RegistryContext,
}
impl OperationRegistry {
    pub const fn new(
        descriptors: &'static [OperationDescriptor],
        retired: &'static [RetiredOperation],
        context: RegistryContext,
    ) -> Self {
        Self {
            descriptors,
            retired,
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
    pub fn retired(self, name: &str) -> Option<RetiredOperation> {
        self.retired.iter().copied().find(|entry| entry.op == name)
    }
    pub fn retired_arguments(self) -> Vec<BoundRetiredArgument> {
        self.descriptors
            .iter()
            .copied()
            .flat_map(|descriptor| {
                descriptor
                    .retired_arguments()
                    .iter()
                    .copied()
                    .filter(move |entry| entry.op == descriptor.name())
                    .map(move |entry| entry.bind(descriptor))
            })
            .collect()
    }
    pub fn retired_argument_redirect(
        self,
        op: &str,
        site: &OperationSite<'_>,
        args: &Map<String, Value>,
    ) -> Option<ArgumentRedirect> {
        self.retired_argument_redirect_with(op, site, args, &no_member_lookup)
    }
    pub fn retired_argument_redirect_with(
        self,
        op: &str,
        site: &OperationSite<'_>,
        args: &Map<String, Value>,
        members: MemberPathLookup<'_>,
    ) -> Option<ArgumentRedirect> {
        let descriptor = self.lookup_at(op, site)?;
        retired_argument_redirect_in(
            descriptor.retired_arguments(),
            descriptor,
            site,
            self.context,
            args,
            members,
        )
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
