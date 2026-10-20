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
}

impl ContentReference {
    fn matches(&self, identity: &MetadataObjectIdentity) -> bool {
        match self {
            Self::MetadataAddress(address) => address == &identity.address,
            Self::Uuid(uuid) => uuid == &identity.uuid,
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
