//! Existing registered subsystem topology facts and role/membership semantics.
use crate::subsystem::{EffectiveSubsystemRole, SubsystemAddress};
use std::path::PathBuf;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemTopology {
    pub roots: Vec<SubsystemTopologyNode>,
    pub dependency_paths: Vec<PathBuf>,
    pub dependency_documents: Vec<SubsystemTopologyDependencyDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemTopologyDependencyDocument {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemTopologyNode {
    pub address: SubsystemAddress,
    pub name: String,
    pub role: EffectiveSubsystemRole,
    pub content: Vec<ContentReference>,
    pub children: Vec<SubsystemTopologyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentReference {
    MetadataAddress(MetadataAddress),
    Uuid(Uuid),
    /// A nested subsystem in the content (`Subsystem.A.Subsystem.B`): the
    /// platform allows it, and it names no object outside the subsystem tree.
    Subsystem(SubsystemAddress),
}

impl ContentReference {
    fn matches(&self, identity: &MetadataObjectIdentity) -> bool {
        match self {
            Self::MetadataAddress(address) => address == &identity.address,
            Self::Uuid(uuid) => uuid == &identity.uuid,
            Self::Subsystem(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataObjectIdentity {
    pub address: MetadataAddress,
    pub uuid: Uuid,
}

impl SubsystemTopology {
    pub fn dependency_paths(&self) -> &[PathBuf] {
        &self.dependency_paths
    }

    pub fn dependency_documents(&self) -> &[SubsystemTopologyDependencyDocument] {
        &self.dependency_documents
    }

    pub fn functional_addresses(&self) -> Vec<SubsystemAddress> {
        functional_addresses_in(&self.roots)
    }

    pub fn interface_addresses(&self) -> Vec<SubsystemAddress> {
        interface_addresses_in(&self.roots)
    }

    pub fn interface_memberships(&self, object_ref: &str) -> Vec<SubsystemAddress> {
        self.memberships_by_metadata_address(object_ref, EffectiveSubsystemRole::Interface)
    }

    pub fn functional_memberships(&self, object_ref: &str) -> Vec<SubsystemAddress> {
        self.memberships_by_metadata_address(object_ref, EffectiveSubsystemRole::Functional)
    }

    pub fn interface_memberships_for(
        &self,
        identity: &MetadataObjectIdentity,
    ) -> Vec<SubsystemAddress> {
        collect_addresses(
            &self.roots,
            EffectiveSubsystemRole::Interface,
            Some(MembershipSelector::Identity(identity)),
        )
    }

    pub fn functional_memberships_for(
        &self,
        identity: &MetadataObjectIdentity,
    ) -> Vec<SubsystemAddress> {
        collect_addresses(
            &self.roots,
            EffectiveSubsystemRole::Functional,
            Some(MembershipSelector::Identity(identity)),
        )
    }

    fn memberships_by_metadata_address(
        &self,
        object_ref: &str,
        role: EffectiveSubsystemRole,
    ) -> Vec<SubsystemAddress> {
        let Ok(address) = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, object_ref)
        else {
            return Vec::new();
        };
        collect_addresses(
            &self.roots,
            role,
            Some(MembershipSelector::MetadataAddress(&address)),
        )
    }
}

/// Prefix of the library description module (`ОбновлениеИнформационнойБазыБСП`
/// and its siblings). A root that holds such a module is a library root by
/// the 1C development standards convention, not by a platform property.
pub const STANDARD_LIBRARY_MODULE_PREFIX: &str = "CommonModule.ОбновлениеИнформационнойБазы";

/// One subsystem that has an object in its content: `direct` for the
/// subsystem that lists the object, not for its ancestors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemMembership {
    pub address: SubsystemAddress,
    pub direct: bool,
}

/// A library root by the standards convention and the module that marks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub address: SubsystemAddress,
    pub module: MetadataAddress,
}

impl SubsystemTopology {
    /// Every subsystem that holds `address` (or `uuid`) in its content, and
    /// the ancestors of each, in topology order. A subsystem that lists the
    /// object itself is direct even when it is also an ancestor of another.
    pub fn memberships_of(
        &self,
        address: &MetadataAddress,
        uuid: Option<Uuid>,
    ) -> Vec<SubsystemMembership> {
        fn visit(
            nodes: &[SubsystemTopologyNode],
            holds: &dyn Fn(&ContentReference) -> bool,
            ancestors: &mut Vec<SubsystemAddress>,
            output: &mut Vec<SubsystemMembership>,
        ) {
            for node in nodes {
                if node.content.iter().any(holds) {
                    for ancestor in ancestors.iter() {
                        if !output.iter().any(|known| &known.address == ancestor) {
                            output.push(SubsystemMembership {
                                address: ancestor.clone(),
                                direct: false,
                            });
                        }
                    }
                    match output
                        .iter_mut()
                        .find(|known| known.address == node.address)
                    {
                        Some(known) => known.direct = true,
                        None => output.push(SubsystemMembership {
                            address: node.address.clone(),
                            direct: true,
                        }),
                    }
                }
                ancestors.push(node.address.clone());
                visit(&node.children, holds, ancestors, output);
                ancestors.pop();
            }
        }
        let holds = |reference: &ContentReference| match reference {
            ContentReference::MetadataAddress(actual) => actual == address,
            ContentReference::Uuid(actual) => Some(*actual) == uuid,
            ContentReference::Subsystem(_) => false,
        };
        let mut output = Vec::new();
        visit(&self.roots, &holds, &mut Vec::new(), &mut output);
        output
    }

    /// Library roots by the standards convention: top-level subsystems outside the
    /// command interface whose tree holds a `ОбновлениеИнформационнойБазы…`
    /// common module. Without such a module there are no roots.
    pub fn library_roots(&self) -> Vec<LibraryRoot> {
        fn module_in(node: &SubsystemTopologyNode) -> Option<MetadataAddress> {
            node.content
                .iter()
                .find_map(|reference| match reference {
                    ContentReference::MetadataAddress(address)
                        if address.as_str().starts_with(STANDARD_LIBRARY_MODULE_PREFIX) =>
                    {
                        Some(address.clone())
                    }
                    _ => None,
                })
                .or_else(|| node.children.iter().find_map(module_in))
        }
        self.roots
            .iter()
            .filter(|root| root.role == EffectiveSubsystemRole::Functional)
            .filter_map(|root| {
                module_in(root).map(|module| LibraryRoot {
                    address: root.address.clone(),
                    module,
                })
            })
            .collect()
    }
}

pub fn functional_addresses_in(nodes: &[SubsystemTopologyNode]) -> Vec<SubsystemAddress> {
    collect_addresses(nodes, EffectiveSubsystemRole::Functional, None)
}

pub fn interface_addresses_in(nodes: &[SubsystemTopologyNode]) -> Vec<SubsystemAddress> {
    collect_addresses(nodes, EffectiveSubsystemRole::Interface, None)
}

fn collect_addresses(
    nodes: &[SubsystemTopologyNode],
    role: EffectiveSubsystemRole,
    content_item: Option<MembershipSelector<'_>>,
) -> Vec<SubsystemAddress> {
    fn visit(
        nodes: &[SubsystemTopologyNode],
        role: EffectiveSubsystemRole,
        content_item: Option<MembershipSelector<'_>>,
        output: &mut Vec<SubsystemAddress>,
    ) {
        for node in nodes {
            if node.role == role
                && content_item
                    .is_none_or(|item| node.content.iter().any(|value| item.matches(value)))
            {
                output.push(node.address.clone());
            }
            visit(&node.children, role, content_item, output);
        }
    }

    let mut output = Vec::new();
    visit(nodes, role, content_item, &mut output);
    output
}

#[derive(Clone, Copy)]
enum MembershipSelector<'a> {
    Identity(&'a MetadataObjectIdentity),
    MetadataAddress(&'a MetadataAddress),
}

impl MembershipSelector<'_> {
    fn matches(self, reference: &ContentReference) -> bool {
        match self {
            Self::Identity(identity) => reference.matches(identity),
            Self::MetadataAddress(expected) => {
                matches!(reference, ContentReference::MetadataAddress(actual) if actual == expected)
            }
        }
    }
}
