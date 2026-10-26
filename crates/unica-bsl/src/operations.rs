use crate::applicability::OperationApplicability;
use unica_operation_contract::OperationSkeleton;

macro_rules! code_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum CodeOperation {
            $($variant),+
        }

        impl CodeOperation {
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
                    static BUNDLE: unica_operations::TypedOperationBundle<CodeOperation> =
                        unica_operations::TypedOperationBundle::new(CodeOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }
        }
    };
}

code_operations!(
    Insert => ("code.insert", CodeInsert, Text),
    Replace => ("code.replace", Code, Text),
);

impl unica_operations::FamilyOperation for CodeOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    fn contract(self) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract(None))
    }
    fn contract_at(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract(Some(site.at())))
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Code
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
    fn content_resource(self) -> Option<&'static str> {
        Some("Form.xml")
    }
}
