use crate::applicability::OperationApplicability;
use crate::FormItemRole;
use unica_operation_contract::OperationContract;
use unica_operation_contract::OperationSkeleton;

macro_rules! form_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum FormOperation {
            $($variant),+
        }

        impl FormOperation {
            pub const fn descriptor(self) -> unica_operations::OperationDescriptor {
                match self { $(Self::$variant => {
                    static BUNDLE: unica_operations::TypedOperationBundle<FormOperation> = unica_operations::TypedOperationBundle::new(FormOperation::$variant);
                    unica_operations::OperationDescriptor::bind(&BUNDLE)
                }),+ }
            }

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
            pub const fn is_named(name: &str) -> bool {
                $(if operation_names_equal(name, $name) {
                    return true;
                })+
                false
            }
        }
    };
}

form_operations!(
    Add => ("form.add", MetadataForms, Items),
    Set => ("form.set", FormDefaults, Values),
    Remove => ("form.remove", Form, Target),
    ItemAdd => ("item.add", FormItemContainer, Items),
    ItemRemove => ("item.remove", FormItem, Target),
    ItemMove => ("item.move", FormItem, Values),
    AttributeAdd => ("attribute.add", Form, Items),
    ColumnAdd => ("column.add", FormAttribute, Items),
    CommandAdd => ("command.add", Form, Items),
    EventBind => ("event.bind", FormEvent, Values),
    PropsSet => ("props.set", FormProperties, Values),
);

impl FormOperation {
    pub fn contract(self) -> OperationContract {
        crate::contracts::form(self)
    }

    pub fn contract_for_address(
        self,
        at: &unica_address::address::QualifiedAddress,
    ) -> OperationContract {
        if self == Self::PropsSet && crate::applicability::is_form_column(at.segments()) {
            crate::contracts::form_column_type_set()
        } else {
            self.contract()
        }
    }

    pub const fn content_resource(self) -> Option<&'static str> {
        match self {
            Self::Add | Self::Set | Self::Remove => None,
            Self::ItemAdd
            | Self::ItemRemove
            | Self::ItemMove
            | Self::AttributeAdd
            | Self::ColumnAdd
            | Self::CommandAdd
            | Self::EventBind
            | Self::PropsSet => Some("Form.xml"),
        }
    }

    pub fn admits_form_item(self, role: Option<FormItemRole>) -> bool {
        match self {
            Self::ItemAdd => role != Some(FormItemRole::Leaf),
            Self::ItemRemove | Self::ItemMove => role != Some(FormItemRole::AutoCommandBar),
            Self::Add
            | Self::Set
            | Self::Remove
            | Self::AttributeAdd
            | Self::ColumnAdd
            | Self::CommandAdd
            | Self::EventBind
            | Self::PropsSet => true,
        }
    }
}

/// Const equality for the test-only Form name lookup.
#[cfg(any(test, feature = "test-support"))]
const fn operation_names_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

impl unica_operations::FamilyOperation for FormOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    fn prepare_redirect(
        self,
        template: &str,
        at: &unica_address::address::QualifiedAddress,
        args: &serde_json::Map<String, serde_json::Value>,
        value: &str,
        members: unica_operations::recovery::MemberPathLookup<'_>,
    ) -> Option<unica_operations::recovery::PreparedRedirect> {
        crate::recovery::prepare(template, at, args, value, members)
    }
    type Observation = crate::FormObservation;
    fn family(self) -> unica_operations::OperationFamily {
        unica_operations::OperationFamily::Form
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
        self.content_resource()
    }
    fn contract_at(
        self,
        site: &unica_source_contract::context::OperationSite<'_>,
    ) -> Option<unica_operation_contract::OperationContract> {
        Some(match self {
            Self::Set => crate::contracts::form_set(
                site.segments()
                    .first()
                    .map(|segment| segment.kind().as_str()),
            ),
            _ => self.contract_for_address(site.at()),
        })
    }

    fn needs_observation(
        self,
        site: Option<&unica_source_contract::context::OperationSite<'_>>,
    ) -> bool {
        match self {
            Self::ColumnAdd => true,
            // Only a data column needs the observed ValueTable parent; without
            // a site props.set keeps its role-independent context admission.
            Self::PropsSet => {
                site.is_some_and(|site| crate::applicability::is_form_column(site.segments()))
            }
            _ => site.is_none_or(|site| {
                site.terminal().kind() == unica_address::address::NodeKind::Item
            }),
        }
    }
    fn admits_observation(self, observation: Option<&Self::Observation>) -> bool {
        if self == Self::ColumnAdd {
            observation == Some(&crate::FormObservation::ValueTableAttribute)
        } else if self == Self::PropsSet {
            observation == Some(&crate::FormObservation::ValueTableColumn)
        } else {
            self.admits_form_item(
                observation
                    .copied()
                    .and_then(crate::FormObservation::item_role),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn existing_form_attribute_exposes_a_column_creation_operation() {
        assert!(FormOperation::is_named("column.add"));
    }

    #[test]
    fn registration_metadata_preserves_every_form_operation() {
        for (operation, name, applicability, skeleton) in [
            (
                FormOperation::Add,
                "form.add",
                OperationApplicability::MetadataForms,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::Set,
                "form.set",
                OperationApplicability::FormDefaults,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::Remove,
                "form.remove",
                OperationApplicability::Form,
                OperationSkeleton::Target,
            ),
            (
                FormOperation::ItemAdd,
                "item.add",
                OperationApplicability::FormItemContainer,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::ItemRemove,
                "item.remove",
                OperationApplicability::FormItem,
                OperationSkeleton::Target,
            ),
            (
                FormOperation::ItemMove,
                "item.move",
                OperationApplicability::FormItem,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::AttributeAdd,
                "attribute.add",
                OperationApplicability::Form,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::ColumnAdd,
                "column.add",
                OperationApplicability::FormAttribute,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::CommandAdd,
                "command.add",
                OperationApplicability::Form,
                OperationSkeleton::Items,
            ),
            (
                FormOperation::EventBind,
                "event.bind",
                OperationApplicability::FormEvent,
                OperationSkeleton::Values,
            ),
            (
                FormOperation::PropsSet,
                "props.set",
                OperationApplicability::FormProperties,
                OperationSkeleton::Values,
            ),
        ] {
            assert_eq!(operation.name(), name);
            assert_eq!(operation.applicability(), applicability);
            assert_eq!(operation.skeleton(), skeleton);
            assert!(FormOperation::is_named(name));
        }
        for name in [
            "form.extra",
            "item.extra",
            "command.add.extra",
            "COMMAND.ADD",
            "",
        ] {
            assert!(!FormOperation::is_named(name), "{name}");
        }
    }

    #[test]
    fn form_contracts_keep_examples_and_distinct_writer_fields() {
        for operation in [FormOperation::AttributeAdd, FormOperation::CommandAdd] {
            let contract = operation.contract();
            assert_eq!(contract.help, "skills/form-edit/SKILL.md");
            assert!(contract.target.starts_with("The form itself:"));
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
            assert_eq!(contract.schema["additionalProperties"], false);
        }
        let attribute = FormOperation::AttributeAdd.contract();
        assert_eq!(
            attribute.example_args,
            json!({"items":[{"name":"Sample","type":"String"}]})
        );
        assert!(
            attribute.schema["properties"]["items"]["items"]["properties"]
                .get("savedData")
                .is_some()
        );
        let command = FormOperation::CommandAdd.contract();
        let fields = &command.schema["properties"]["items"]["items"]["properties"];
        assert!(fields.get("action").is_some());
        assert!(fields.get("savedData").is_none());
        assert_eq!(
            command
                .normalize(&json!({"items":[{"name":"A","type":"String"}]}), "args")
                .unwrap_err()
                .path,
            "args.items[0].type"
        );
    }
    #[test]
    fn column_creation_contract_reuses_the_attribute_column_shape() {
        let contract = FormOperation::ColumnAdd.contract();
        let nested = FormOperation::AttributeAdd.contract();
        assert_eq!(
            contract.schema["properties"]["items"]["items"],
            nested.schema["properties"]["items"]["items"]["properties"]["columns"]["items"]
        );
        for input in [
            json!({"items":[{"name":"Value","type":"CatalogRef.Items"}]}),
            json!({"items":[{"name":"Value","type":"string(40)","title":"Value"}]}),
        ] {
            assert!(contract.normalize(&input, "args").is_ok());
        }
        for input in [
            json!({"items":[]}),
            json!({"items":[{"name":"Value"}]}),
            json!({"items":[{"name":"Value","type":true}]}),
            json!({"items":[{"name":"Value","type":"Boolean","id":3}]}),
            json!({"items":[{"name":"Value","type":"Boolean","dataPath":"Rows.Value"}]}),
            json!({"items":[{"name":"Value","type":"Boolean"}],"attribute":"Rows"}),
        ] {
            assert!(contract.normalize(&input, "args").is_err(), "{input}");
        }
    }
    #[test]
    fn column_creation_requires_observed_valuetable_facts() {
        use crate::FormObservation;
        use unica_operations::FamilyOperation;
        assert!(FormOperation::ColumnAdd.needs_observation(None));
        assert!(FormOperation::ColumnAdd
            .admits_observation(Some(&FormObservation::ValueTableAttribute)));
        for observed in [
            None,
            Some(FormObservation::Item(FormItemRole::Container)),
            FormObservation::from_attribute_type("ValueTree"),
            FormObservation::from_attribute_type("boolean"),
        ] {
            assert!(!FormOperation::ColumnAdd.admits_observation(observed.as_ref()));
        }
        assert!(FormOperation::ItemAdd
            .admits_observation(Some(&FormObservation::Item(FormItemRole::Container))));
        assert!(!FormOperation::ItemAdd
            .admits_observation(Some(&FormObservation::Item(FormItemRole::Leaf))));
    }

    #[test]
    fn context_admission_without_a_site_keeps_props_set_and_item_role_filters() {
        use crate::FormObservation;
        let item = |role| {
            move |_: unica_operations::ObservationType| {
                Some(Box::new(FormObservation::Item(role)) as Box<dyn std::any::Any>)
            }
        };
        for role in [
            FormItemRole::Container,
            FormItemRole::Leaf,
            FormItemRole::AutoCommandBar,
        ] {
            assert!(
                FormOperation::PropsSet
                    .descriptor()
                    .admits_context(&item(role)),
                "{role:?}"
            );
        }
        assert!(FormOperation::PropsSet
            .descriptor()
            .admits_context(&|_| None));
        assert!(!FormOperation::ItemAdd
            .descriptor()
            .admits_context(&item(FormItemRole::Leaf)));
        assert!(!FormOperation::ItemRemove
            .descriptor()
            .admits_context(&item(FormItemRole::AutoCommandBar)));
        assert!(!FormOperation::ColumnAdd
            .descriptor()
            .admits_context(&|_| None));
    }
}
