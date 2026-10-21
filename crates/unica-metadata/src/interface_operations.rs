use crate::applicability::OperationApplicability;
use unica_operation_contract::OperationSkeleton;

macro_rules! interface_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum InterfaceOperation {
            $($variant),+
        }

        impl InterfaceOperation {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }

            pub const fn applicability(self) -> OperationApplicability {
                match self { $(Self::$variant => OperationApplicability::$applicability),+ }
            }

            pub const fn skeleton(self) -> OperationSkeleton {
                match self { $(Self::$variant => OperationSkeleton::$skeleton),+ }
            }

            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<InterfaceOperation> =
                        unica_operations::TypedOperationBundle::new(InterfaceOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }
        }
    };
}

interface_operations!(
    VisibilitySet => ("commandVisibility.set", Interface, Items),
    PlacementSet => ("commandPlacement.set", Interface, Items),
    CommandOrderSet => ("commandOrder.set", InterfaceGroup, Values),
    GroupOrderSet => ("groupOrder.set", Interface, Values),
    SubsystemOrderSet => ("subsystemOrder.set", Configuration, Values),
);

impl unica_operations::FamilyOperation for InterfaceOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Interface
    }
    fn name(self) -> &'static str {
        self.name()
    }
    fn skeleton(self) -> OperationSkeleton {
        self.skeleton()
    }
    fn address_shape(self) -> &'static str {
        self.applicability().shape()
    }
    fn matches(self, site: &unica_source_contract::context::OperationSite<'_>) -> bool {
        self.applicability().matches(site)
    }
}
