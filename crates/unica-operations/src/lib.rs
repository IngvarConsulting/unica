//! Ordered operation registration through coherent family-owned typed bundles.
//! This owner knows no family implementations or representation codecs.

pub mod recovery;
mod registry;
pub use registry::OperationRegistry;

use recovery::{MemberPathLookup, PreparedRedirect, RetiredArgument};
use serde_json::{Map, Value};
use std::any::{Any, TypeId};
use std::fmt;
use unica_address::address::QualifiedAddress;
use unica_operation_contract::{OperationContract, OperationSkeleton};
use unica_source_contract::context::RegistryContext;
use unica_source_contract::context::{OperationSite, SourceSetTraits};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationFamily {
    Metadata,
    Properties,
    Form,
    Role,
    Dcs,
    Mxl,
    Xdto,
    Subsystem,
    Interface,
    Support,
    Code,
    Event,
}

/// A request for a fact owned by the bound family; no caller chooses which
/// observation type that family must check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationType(TypeId);

impl ObservationType {
    pub fn is<T: Any>(self) -> bool {
        self.0 == TypeId::of::<T>()
    }
}

pub type ObservationLookup<'a> =
    &'a dyn Fn(&QualifiedAddress, ObservationType) -> Option<Box<dyn Any>>;
pub type ContextObservationLookup<'a> = &'a dyn Fn(ObservationType) -> Option<Box<dyn Any>>;

/// Implemented by closed family operation identities. All descriptor behavior
/// is derived from the same typed value before erasure.
pub trait FamilyOperation: Copy + fmt::Debug + Send + Sync + 'static {
    type Observation: Any;

    fn family(self) -> OperationFamily;
    fn name(self) -> &'static str;
    fn skeleton(self) -> OperationSkeleton;
    fn address_shape(self) -> &'static str;
    fn matches(self, site: &OperationSite<'_>) -> bool;
    fn writes_source(self, source: SourceSetTraits) -> bool {
        source.is_writable()
    }

    fn contract(self) -> Option<OperationContract> {
        None
    }

    fn contract_at(self, _site: &OperationSite<'_>) -> Option<OperationContract> {
        self.contract()
    }

    fn content_resource(self) -> Option<&'static str> {
        None
    }

    fn creator(self) -> Option<&'static str> {
        None
    }
    fn retired_arguments(self) -> &'static [RetiredArgument] {
        &[]
    }
    fn prepare_redirect(
        self,
        template: &str,
        _at: &QualifiedAddress,
        _args: &Map<String, Value>,
        _value: &str,
        _members: MemberPathLookup<'_>,
    ) -> Option<PreparedRedirect> {
        Some(PreparedRedirect::literal(template))
    }
    fn additional_recovery_candidates(self, _site: &OperationSite<'_>) -> Vec<QualifiedAddress> {
        Vec::new()
    }
    fn retired_query_alias_target(
        self,
        _site: &OperationSite<'_>,
        _context: RegistryContext,
        _members: MemberPathLookup<'_>,
    ) -> Option<QualifiedAddress> {
        None
    }

    /// `Some(site)` is the node dictionary/recovery phase. `None` is the
    /// backend's explicit context filter; no additional read is performed here.
    fn needs_observation(self, _site: Option<&OperationSite<'_>>) -> bool {
        false
    }

    fn admits_observation(self, _observation: Option<&Self::Observation>) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct TypedOperationBundle<O: FamilyOperation> {
    operation: O,
}

impl<O: FamilyOperation> TypedOperationBundle<O> {
    pub const fn new(operation: O) -> Self {
        Self { operation }
    }
}

trait ErasedOperation: Sync {
    fn family(&self) -> OperationFamily;
    fn name(&self) -> &'static str;
    fn skeleton(&self) -> OperationSkeleton;
    fn address_shape(&self) -> &'static str;
    fn matches(&self, site: &OperationSite<'_>) -> bool;
    fn writes_source(&self, source: SourceSetTraits) -> bool;
    fn contract(&self) -> Option<OperationContract>;
    fn contract_at(&self, site: &OperationSite<'_>) -> Option<OperationContract>;
    fn content_resource(&self) -> Option<&'static str>;
    fn creator(&self) -> Option<&'static str>;
    fn retired_arguments(&self) -> &'static [RetiredArgument];
    fn prepare_redirect(
        &self,
        template: &str,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        value: &str,
        members: MemberPathLookup<'_>,
    ) -> Option<PreparedRedirect>;
    fn additional_recovery_candidates(&self, site: &OperationSite<'_>) -> Vec<QualifiedAddress>;
    fn retired_query_alias_target(
        &self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        members: MemberPathLookup<'_>,
    ) -> Option<QualifiedAddress>;

    fn operation(&self) -> &dyn Any;
    fn observation_type(&self) -> ObservationType;
    fn admits(
        &self,
        site: Option<&OperationSite<'_>>,
        lookup: ContextObservationLookup<'_>,
    ) -> bool;
}

impl<O: FamilyOperation> ErasedOperation for TypedOperationBundle<O> {
    fn family(&self) -> OperationFamily {
        self.operation.family()
    }
    fn name(&self) -> &'static str {
        self.operation.name()
    }
    fn skeleton(&self) -> OperationSkeleton {
        self.operation.skeleton()
    }
    fn address_shape(&self) -> &'static str {
        self.operation.address_shape()
    }
    fn matches(&self, site: &OperationSite<'_>) -> bool {
        self.operation.matches(site)
    }
    fn writes_source(&self, source: SourceSetTraits) -> bool {
        self.operation.writes_source(source)
    }
    fn contract(&self) -> Option<OperationContract> {
        self.operation.contract()
    }
    fn contract_at(&self, site: &OperationSite<'_>) -> Option<OperationContract> {
        self.operation.contract_at(site)
    }
    fn content_resource(&self) -> Option<&'static str> {
        self.operation.content_resource()
    }
    fn creator(&self) -> Option<&'static str> {
        self.operation.creator()
    }

    fn retired_arguments(&self) -> &'static [RetiredArgument] {
        self.operation.retired_arguments()
    }
    fn prepare_redirect(
        &self,
        template: &str,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        value: &str,
        members: MemberPathLookup<'_>,
    ) -> Option<PreparedRedirect> {
        self.operation
            .prepare_redirect(template, at, args, value, members)
    }
    fn additional_recovery_candidates(&self, site: &OperationSite<'_>) -> Vec<QualifiedAddress> {
        self.operation.additional_recovery_candidates(site)
    }
    fn retired_query_alias_target(
        &self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        members: MemberPathLookup<'_>,
    ) -> Option<QualifiedAddress> {
        self.operation
            .retired_query_alias_target(site, context, members)
    }
    fn operation(&self) -> &dyn Any {
        &self.operation
    }
    fn observation_type(&self) -> ObservationType {
        ObservationType(TypeId::of::<O::Observation>())
    }

    fn admits(
        &self,
        site: Option<&OperationSite<'_>>,
        lookup: ContextObservationLookup<'_>,
    ) -> bool {
        if !self.operation.needs_observation(site) {
            return true;
        }
        match lookup(self.observation_type()) {
            None => self.operation.admits_observation(None),
            Some(observed) => observed
                .downcast_ref::<O::Observation>()
                .is_some_and(|value| self.operation.admits_observation(Some(value))),
        }
    }
}

/// Coherent registration cannot be edited into a mismatched raw descriptor.
///
/// ```compile_fail
/// use unica_operations::OperationDescriptor;
/// fn rename(descriptor: OperationDescriptor) -> OperationDescriptor {
///     OperationDescriptor { name: "different.operation", ..descriptor }
/// }
/// ```
#[derive(Clone, Copy)]
pub struct OperationDescriptor {
    binding: &'static dyn ErasedOperation,
}

impl OperationDescriptor {
    pub const fn bind<O: FamilyOperation>(bundle: &'static TypedOperationBundle<O>) -> Self {
        Self { binding: bundle }
    }

    pub fn name(self) -> &'static str {
        self.binding.name()
    }
    pub fn family(self) -> OperationFamily {
        self.binding.family()
    }
    pub fn skeleton(self) -> OperationSkeleton {
        self.binding.skeleton()
    }
    pub fn skeleton_key(self) -> &'static str {
        self.skeleton().key()
    }
    pub fn address_shape(self) -> &'static str {
        self.binding.address_shape()
    }
    pub fn operation<O: FamilyOperation>(self) -> Option<O> {
        self.binding.operation().downcast_ref::<O>().copied()
    }
    pub fn content_resource(self) -> Option<&'static str> {
        self.binding.content_resource()
    }
    pub fn creator(self) -> Option<&'static str> {
        self.binding.creator()
    }
    pub fn argument_contract(self) -> Option<OperationContract> {
        self.binding.contract()
    }
    pub fn argument_contract_at(self, site: &OperationSite<'_>) -> Option<OperationContract> {
        self.binding.contract_at(site)
    }

    pub fn writes_source(self, source: SourceSetTraits) -> bool {
        self.binding.writes_source(source)
    }

    pub fn applies_at(self, site: &OperationSite<'_>) -> bool {
        site.at().line_range().is_none()
            && self.writes_source(site.source())
            && self.binding.matches(site)
    }

    pub fn admits_node(self, site: &OperationSite<'_>, lookup: ObservationLookup<'_>) -> bool {
        self.binding
            .admits(Some(site), &|requested| lookup(site.at(), requested))
    }

    pub fn admits_context(self, lookup: ContextObservationLookup<'_>) -> bool {
        self.binding.admits(None, lookup)
    }

    pub fn observation_type(self) -> ObservationType {
        self.binding.observation_type()
    }

    pub fn applies_with_observations(
        self,
        site: &OperationSite<'_>,
        lookup: ObservationLookup<'_>,
    ) -> bool {
        self.applies_at(site) && self.admits_node(site, lookup)
    }
}

impl fmt::Debug for OperationDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationDescriptor")
            .field("family", &self.family())
            .field("name", &self.name())
            .finish()
    }
}

impl PartialEq for OperationDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.family() == other.family() && self.name() == other.name()
    }
}
impl Eq for OperationDescriptor {}

impl OperationDescriptor {
    pub fn copyable_skeleton(self) -> unica_operation_contract::OperationRef {
        unica_operation_contract::OperationRef::new(self.name(), self.skeleton().args())
    }
    pub fn copyable_skeleton_at(
        self,
        at: impl Into<String>,
    ) -> unica_operation_contract::OperationRef {
        let mut args = self.skeleton().args();
        args.insert("at".to_string(), serde_json::Value::String(at.into()));
        unica_operation_contract::OperationRef::new(self.name(), args)
    }
}

impl OperationDescriptor {
    pub fn retired_arguments(self) -> &'static [RetiredArgument] {
        self.binding.retired_arguments()
    }
    pub fn prepare_redirect(
        self,
        template: &str,
        at: &QualifiedAddress,
        args: &Map<String, Value>,
        value: &str,
        members: MemberPathLookup<'_>,
    ) -> Option<PreparedRedirect> {
        self.binding
            .prepare_redirect(template, at, args, value, members)
    }
    pub fn retired_query_alias_target(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        members: MemberPathLookup<'_>,
    ) -> Option<QualifiedAddress> {
        self.binding
            .retired_query_alias_target(site, context, members)
    }
    pub fn nearest_ancestor_with_observations(
        self,
        site: &OperationSite<'_>,
        context: RegistryContext,
        observations: ObservationLookup<'_>,
    ) -> Option<QualifiedAddress> {
        let segments = site.segments();
        let mut candidates = self.binding.additional_recovery_candidates(site);
        candidates.extend((1..segments.len()).rev().map(|length| QualifiedAddress {
            line_range: None,
            source_set: site.at().source_set().to_string(),
            segments: segments[..length].to_vec(),
        }));
        if site.route() != unica_source_contract::route::RouteClass::Configuration {
            candidates.push(
                QualifiedAddress::parse(&format!("{}:Configuration", site.at().source_set()))
                    .expect("an admitted source set name forms the configuration-root address"),
            );
        }
        candidates.into_iter().find(|candidate| {
            context
                .site(candidate, site.source())
                .is_some_and(|candidate| self.applies_with_observations(&candidate, observations))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEMPLATE: TypedOperationBundle<TemplateOperation> =
        TypedOperationBundle::new(TemplateOperation::Set);
    static FORM_ADD: TypedOperationBundle<FormOperation> =
        TypedOperationBundle::new(FormOperation::Add);
    static FORM_REMOVE: TypedOperationBundle<FormOperation> =
        TypedOperationBundle::new(FormOperation::Remove);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TemplateOperation {
        Set,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FormOperation {
        Add,
        Remove,
    }
    #[derive(Debug)]
    struct TemplateFormat;
    #[derive(Debug)]
    struct FormItemRole;

    impl FamilyOperation for TemplateOperation {
        type Observation = TemplateFormat;
        fn family(self) -> OperationFamily {
            OperationFamily::Dcs
        }
        fn name(self) -> &'static str {
            "template.test"
        }
        fn skeleton(self) -> OperationSkeleton {
            OperationSkeleton::Values
        }
        fn address_shape(self) -> &'static str {
            "a template"
        }
        fn matches(self, _: &OperationSite<'_>) -> bool {
            true
        }
        fn needs_observation(self, _: Option<&OperationSite<'_>>) -> bool {
            true
        }
        fn admits_observation(self, value: Option<&TemplateFormat>) -> bool {
            value.is_some()
        }
    }
    impl FamilyOperation for FormOperation {
        type Observation = FormItemRole;
        fn family(self) -> OperationFamily {
            OperationFamily::Form
        }
        fn name(self) -> &'static str {
            match self {
                Self::Add => "item.add",
                Self::Remove => "item.remove",
            }
        }
        fn skeleton(self) -> OperationSkeleton {
            OperationSkeleton::Items
        }
        fn address_shape(self) -> &'static str {
            "a form item"
        }
        fn matches(self, _: &OperationSite<'_>) -> bool {
            true
        }
        fn needs_observation(self, _: Option<&OperationSite<'_>>) -> bool {
            true
        }
        fn admits_observation(self, _: Option<&FormItemRole>) -> bool {
            true
        }
    }

    #[test]
    fn a_bundle_recovers_only_its_own_closed_operation_type() {
        let descriptor = OperationDescriptor::bind(&TEMPLATE);
        assert_eq!(
            descriptor.operation::<TemplateOperation>(),
            Some(TemplateOperation::Set)
        );
        assert_eq!(descriptor.operation::<FormOperation>(), None);
        assert_eq!(descriptor.family(), OperationFamily::Dcs);
        assert_eq!(descriptor.name(), "template.test");
        assert_eq!(descriptor.skeleton(), OperationSkeleton::Values);
    }

    #[test]
    fn stable_identity_does_not_depend_on_bundle_addresses() {
        static FIRST: TypedOperationBundle<FormOperation> =
            TypedOperationBundle::new(FormOperation::Add);
        static SECOND: TypedOperationBundle<FormOperation> =
            TypedOperationBundle::new(FormOperation::Add);
        assert_eq!(
            OperationDescriptor::bind(&FIRST),
            OperationDescriptor::bind(&SECOND)
        );
        assert_ne!(
            OperationDescriptor::bind(&FIRST),
            OperationDescriptor::bind(&FORM_REMOVE)
        );
    }

    #[test]
    fn missing_template_observation_and_supplied_wrong_type_both_refuse() {
        let descriptor = OperationDescriptor::bind(&TEMPLATE);
        assert!(!descriptor.admits_context(&|_| None));
        assert!(!descriptor.admits_context(&|requested| {
            assert!(requested.is::<TemplateFormat>());
            Some(Box::new(FormItemRole))
        }));
        assert!(descriptor.admits_context(&|requested| {
            assert!(requested.is::<TemplateFormat>());
            Some(Box::new(TemplateFormat))
        }));
    }

    #[test]
    fn wrong_form_observation_never_becomes_the_historically_permitted_missing_fact() {
        for operation in [FormOperation::Add, FormOperation::Remove] {
            let descriptor = match operation {
                FormOperation::Add => OperationDescriptor::bind(&FORM_ADD),
                FormOperation::Remove => OperationDescriptor::bind(&FORM_REMOVE),
            };
            assert!(descriptor.admits_context(&|_| None));
            assert!(!descriptor.admits_context(&|requested| {
                assert!(requested.is::<FormItemRole>());
                Some(Box::new(TemplateFormat))
            }));
            assert!(descriptor.admits_context(&|_| Some(Box::new(FormItemRole))));
        }
    }
}
