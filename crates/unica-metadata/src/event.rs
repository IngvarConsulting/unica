use crate::applicability::OperationApplicability;
use unica_operation_contract::OperationContract;
use unica_operation_contract::OperationSkeleton;

macro_rules! event_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum EventOperation {
            $($variant),+
        }

        impl EventOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<EventOperation> = unica_operations::TypedOperationBundle::new(EventOperation::$variant);
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

event_operations!(
    Implement => ("event.implement", Event, Target),
);

impl EventOperation {
    pub fn contract(self) -> OperationContract {
        crate::event_contract::event(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_metadata_preserves_event_identity_and_boundary_names() {
        let operation = EventOperation::Implement;
        assert_eq!(EventOperation::ALL, &[operation]);
        assert_eq!(operation.name(), "event.implement");
        assert_eq!(operation.applicability(), OperationApplicability::Event);
        assert_eq!(operation.skeleton(), OperationSkeleton::Target);
        assert_eq!(EventOperation::named("event.implement"), Some(operation));
        for name in [
            "event.bind",
            "event.extra",
            "EVENT.IMPLEMENT",
            "event.implement ",
            "",
        ] {
            assert_eq!(EventOperation::named(name), None, "{name}");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormCallType {
    Before,
    After,
    Override,
}

impl FormCallType {
    pub const ALL: [Self; 3] = [Self::Before, Self::After, Self::Override];

    pub fn from_xml(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|call_type| call_type.as_str() == value)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::After => "After",
            Self::Override => "Override",
        }
    }
}

impl unica_operations::FamilyOperation for EventOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    type Observation = ();
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Event
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
        Some("Form.xml")
    }
}

#[cfg(test)]
mod vocabulary_tests {
    use super::FormCallType;
    #[test]
    fn form_call_type_vocabulary_preserves_xml_spelling_and_order() {
        assert_eq!(
            FormCallType::ALL.map(FormCallType::as_str),
            ["Before", "After", "Override"]
        );
        for call_type in FormCallType::ALL {
            assert_eq!(FormCallType::from_xml(call_type.as_str()), Some(call_type));
        }
        for value in ["Instead", "before", " Before", "After ", ""] {
            assert_eq!(FormCallType::from_xml(value), None, "{value}");
        }
    }
}

impl EventOperation {
    pub fn implementation_skeleton(
        at: impl Into<String>,
        call_type: Option<&str>,
    ) -> unica_operation_contract::OperationRef {
        let descriptor = Self::Implement.descriptor();
        let mut args = descriptor.skeleton().args();
        args.insert("at".to_string(), serde_json::Value::String(at.into()));
        if let Some(call_type) = call_type {
            args.insert(
                "callType".to_string(),
                serde_json::Value::String(call_type.to_string()),
            );
        }
        unica_operation_contract::OperationRef::new(descriptor.name(), args)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingFact {
    Platform,
    Property,
    Code,
    Subscription,
}
