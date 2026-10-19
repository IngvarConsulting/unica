use super::apply::{OperationApplicability, OperationSkeleton};
use super::operation_contract::OperationContract;

macro_rules! event_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum EventOperation {
            $($variant),+
        }

        impl EventOperation {
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

event_operations!(
    Implement => ("event.implement", Event, Target),
);

impl EventOperation {
    pub(crate) fn contract(self) -> OperationContract {
        OperationContract::event(self)
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
