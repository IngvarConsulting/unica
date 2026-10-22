//! Pure source facts and an address classified by a semantic owner.
//! These records describe an operation site; they do not authorize publication.

use crate::route::RouteClass;
use crate::{SourceProfile, SourceSetKind};
use unica_address::address::{AddressSegment, QualifiedAddress};

/// Exact writable identity and the identity used for read classification are
/// separate facts. A classification fallback never supplies write admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSetTraits {
    kind: SourceSetKind,
    exact_writable_profile: Option<SourceProfile>,
    classification_profile: SourceProfile,
}

impl SourceSetTraits {
    /// Records facts already admitted by the source/profile semantic owner.
    pub const fn from_profiles(
        kind: SourceSetKind,
        exact_writable_profile: Option<SourceProfile>,
        classification_profile: SourceProfile,
    ) -> Self {
        Self {
            kind,
            exact_writable_profile,
            classification_profile,
        }
    }

    pub const fn kind(self) -> SourceSetKind {
        self.kind
    }

    pub const fn exact_writable_profile(self) -> Option<SourceProfile> {
        self.exact_writable_profile
    }

    pub const fn classification_profile(self) -> SourceProfile {
        self.classification_profile
    }

    /// Source-level admission fact. Family policy and final write authority
    /// remain with their owners.
    pub const fn is_writable(self) -> bool {
        self.exact_writable_profile.is_some()
            && matches!(
                self.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
    }
}

/// Address, observed source facts and the route assigned by the sole semantic
/// classifier. Node existence, state and family applicability are separate.
#[derive(Debug, Clone, Copy)]
pub struct OperationSite<'a> {
    at: &'a QualifiedAddress,
    source: SourceSetTraits,
    route: RouteClass,
}

impl<'a> OperationSite<'a> {
    pub const fn from_route(
        at: &'a QualifiedAddress,
        source: SourceSetTraits,
        route: RouteClass,
    ) -> Self {
        Self { at, source, route }
    }

    pub const fn source(&self) -> SourceSetTraits {
        self.source
    }

    pub const fn route(&self) -> RouteClass {
        self.route
    }

    pub const fn at(&self) -> &'a QualifiedAddress {
        self.at
    }

    pub fn segments(&self) -> &[AddressSegment] {
        self.at.segments()
    }

    pub fn terminal(&self) -> &AddressSegment {
        self.segments()
            .last()
            .expect("a parsed logical address has a terminal segment")
    }
}

/// A classifier binds each new candidate address to the same source facts.
/// Recovery must call it again instead of copying the original site's route.
pub type SiteClassifier =
    for<'a> fn(&'a QualifiedAddress, SourceSetTraits) -> Option<OperationSite<'a>>;

#[derive(Debug, Clone, Copy)]
pub struct RegistryContext {
    classifier: SiteClassifier,
}

impl RegistryContext {
    pub const fn new(classifier: SiteClassifier) -> Self {
        Self { classifier }
    }

    pub fn site<'a>(
        self,
        at: &'a QualifiedAddress,
        source: SourceSetTraits,
    ) -> Option<OperationSite<'a>> {
        (self.classifier)(at, source)
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationSite, RegistryContext, SourceSetTraits};
    use crate::route::RouteClass;
    use crate::{SourceProfile, SourceSetKind};
    use unica_address::address::{NodeKind, QualifiedAddress};

    #[test]
    fn classification_identity_does_not_supply_writable_identity() {
        let active = SourceProfile::platform_xml_8_3_27_format_2_20();
        let read_only = SourceSetTraits::from_profiles(SourceSetKind::Configuration, None, active);
        assert_eq!(read_only.classification_profile(), active);
        assert_eq!(read_only.exact_writable_profile(), None);
        assert!(!read_only.is_writable());
        for kind in [
            SourceSetKind::ExternalProcessor,
            SourceSetKind::ExternalReport,
        ] {
            let external = SourceSetTraits::from_profiles(kind, Some(active), active);
            assert!(!external.is_writable());
        }
    }

    #[test]
    fn context_reclassifies_each_address_and_preserves_source_facts() {
        // This immutable fake demonstrates the port's data flow, not product grammar.
        fn classify(at: &QualifiedAddress, source: SourceSetTraits) -> Option<OperationSite<'_>> {
            let route = if at.segments()[0].kind() == NodeKind::Configuration {
                RouteClass::Configuration
            } else {
                RouteClass::Metadata
            };
            Some(OperationSite::from_route(at, source, route))
        }
        let profile = SourceProfile::platform_xml_8_3_27_format_2_20();
        let source = SourceSetTraits::from_profiles(SourceSetKind::Extension, None, profile);
        let context = RegistryContext::new(classify);
        let owner = QualifiedAddress::parse("extension:Document.Order").unwrap();
        let root = QualifiedAddress::parse("extension:Configuration").unwrap();
        let owner_site = context.site(&owner, source).unwrap();
        let root_site = context.site(&root, source).unwrap();
        assert_eq!(owner_site.route(), RouteClass::Metadata);
        assert_eq!(root_site.route(), RouteClass::Configuration);
        assert_eq!(owner_site.source(), source);
        assert_eq!(root_site.source(), source);
        assert_eq!(owner_site.at(), &owner);
        assert_eq!(root_site.at(), &root);
    }
}
