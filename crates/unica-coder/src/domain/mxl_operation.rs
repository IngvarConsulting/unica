use super::apply::{OperationApplicability, OperationSkeleton};
use super::operation_contract::OperationContract;

macro_rules! mxl_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum MxlOperation {
            $($variant),+
        }

        impl MxlOperation {
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub(super) const fn applicability(self) -> OperationApplicability {
                match self {
                    $(Self::$variant => OperationApplicability::$applicability),+
                }
            }

            pub(super) const fn skeleton(self) -> OperationSkeleton {
                match self {
                    $(Self::$variant => OperationSkeleton::$skeleton),+
                }
            }

            #[cfg(test)]
            pub(crate) fn named(name: &str) -> Option<Self> {
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
    pub(crate) fn contract(self) -> OperationContract {
        OperationContract::mxl(self)
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
