use crate::applicability::OperationApplicability;
use unica_operation_contract::OperationContract;
use unica_operation_contract::OperationSkeleton;

macro_rules! mxl_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum MxlOperation {
            $($variant),+
        }

        impl MxlOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<MxlOperation> = unica_operations::TypedOperationBundle::new(MxlOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub const fn applicability(self) -> OperationApplicability {
                match self {
                    $(Self::$variant => OperationApplicability::$applicability),+
                }
            }

            pub const fn skeleton(self) -> OperationSkeleton {
                match self {
                    $(Self::$variant => OperationSkeleton::$skeleton),+
                }
            }

            #[cfg(any(test, feature = "test-support"))]
            pub fn named(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|operation| operation.name() == name)
            }
        }
    };
}

mxl_operations!(
    Set => ("mxl.set", MxlArea, Values),
    AreaAdd => ("area.add", Mxl, Items),
);

impl MxlOperation {
    pub fn contract(self) -> OperationContract {
        crate::contracts::mxl(self)
    }
}

impl unica_operations::FamilyOperation for MxlOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = crate::TemplateFormat;
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Mxl
    }
    fn name(self) -> &'static str {
        self.name()
    }
    fn skeleton(self) -> unica_operation_contract::OperationSkeleton {
        self.skeleton()
    }
    fn address_shape(self) -> &'static str {
        self.applicability().shape()
    }
    fn matches(self, site: &unica_source_contract::context::OperationSite<'_>) -> bool {
        self.applicability().matches(site)
    }
    fn contract(self) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract())
    }
    fn content_resource(self) -> Option<&'static str> {
        Some("Template.xml")
    }

    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        site.is_none_or(|site| site.terminal().kind() == unica_address::address::NodeKind::Template)
    }
    fn admits_observation(self, observation: Option<&Self::Observation>) -> bool {
        observation == Some(&crate::TemplateFormat::SpreadsheetDocument)
    }

    fn creator(self) -> Option<&'static str> {
        match self {
            Self::Set => Some("area.add"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_metadata_preserves_mxl_operations_and_boundary_names() {
        for (operation, name, applicability, skeleton) in [
            (
                MxlOperation::Set,
                "mxl.set",
                OperationApplicability::MxlArea,
                OperationSkeleton::Values,
            ),
            (
                MxlOperation::AreaAdd,
                "area.add",
                OperationApplicability::Mxl,
                OperationSkeleton::Items,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), skeleton);
            assert_eq!(MxlOperation::named(name), Some(operation));
        }
        assert_eq!(
            MxlOperation::ALL,
            &[MxlOperation::Set, MxlOperation::AreaAdd]
        );
        for name in ["mxl.extra", "area.extra", "MXL.SET", "mxl.set ", ""] {
            assert_eq!(MxlOperation::named(name), None, "{name}");
        }
    }
}
