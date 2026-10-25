use crate::applicability::OperationApplicability;
use serde_json::json;
use unica_operation_contract::{object, text, OperationContract, OperationSkeleton};

macro_rules! interface_operations {
    ($($variant:ident => ($name:literal, $applicability:ident, $skeleton:ident)),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum InterfaceOperation {
            $($variant),+
        }

        impl InterfaceOperation {
            pub fn from_name(name: &str) -> Option<Self> {
                match name { $($name => Some(Self::$variant),)+ _ => None }
            }

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
    PanelsSet => ("panels.set", ConfigurationRoot, Values),
    HomePageSet => ("homePage.set", ConfigurationRoot, Values),
);

/// Names of the standard client application panels; `tools` stands only at
/// the top.
const PANELS: [&str; 6] = [
    "sections",
    "favorites",
    "history",
    "open",
    "functions",
    "tools",
];

/// The closed argument contract of `panels.set`: it replaces the whole
/// root document, while the order operations validate against the stored file.
pub(crate) fn panels_contract() -> OperationContract {
    // Every area names all panels: the planner, not the schema,
    // refuses `tools` outside the top with the reason.
    let area = json!({"type":"array", "items":{"anyOf":[
        {"type":"string", "enum":PANELS},
        {"type":"array", "minItems":1, "items":{"type":"string", "enum":PANELS}},
    ]}});
    let values = json!({"type":"object", "additionalProperties":false,
    "properties":{
        "top": area,
        "left": area,
        "right": area,
        "bottom": area,
    }});
    OperationContract {
            schema: object(&[("at", text(true)), ("values", values)], &["values"]),
            target: "The configuration root `<set>:Configuration`; an extension has no panel layout of its own.",
            effect: "Replace the panel layout of the client application in `Ext/ClientApplicationInterface.xml`.",
            notes: "values lists the areas top, left, right and bottom; an omitted area is left empty, and `{}` removes every panel. Each entry is a panel name or an array of names, a group of panels. A panel is placed once; `tools` only at the top. The layout read from the root's `panels` is accepted back as is, unless it lists `unknown` panels: such a layout is refused rather than replaced, since writing it would drop them. Preview writes nothing; execute only the saved execution token.",
            example_args: json!({"values":{"top":["open"],"left":["sections"]}}),
            help: "skills/cf-edit/SKILL.md",
            format_validators: &[],
        }
}

/// The closed argument contract of `homePage.set`.
pub(crate) fn home_page_contract() -> OperationContract {
    let item = json!({"type":"object", "additionalProperties":false,
    "required":["form"], "properties":{
        "form": {"type":"string"},
        "height": {"type":"integer", "minimum":1, "default":10},
        "common": {"type":"boolean", "default":true},
        "roles": {"type":"array", "items": object(&[
            ("role", json!({"type":"string"})),
            ("visible", json!({"type":"boolean"})),
        ], &["role", "visible"])},
    }});
    let column = json!({"type":"array", "items":item});
    let values = object(
        &[
            (
                "template",
                json!({"type":"string",
                    "enum":["OneColumn","TwoColumnsEqualWidth","TwoColumnsVariableWidth"]}),
            ),
            ("left", column.clone()),
            ("right", column),
        ],
        &["template"],
    );
    OperationContract {
            schema: object(&[("at", text(true)), ("values", values)], &["values"]),
            target: "The configuration root `<set>:Configuration`; an extension has no home page of its own.",
            effect: "Replace the home page in `Ext/HomePageWorkArea.xml`.",
            notes: "values names the template and the forms of the left and right columns; `OneColumn` has only left. A form is `CommonForm.F` or `Kind.Name.Form.F`, a role `Role.R`; a reference the stored page lacks must exist in the set, so a form created in the same apply goes before this operation. common is the visibility for roles not listed. The page read from the root's `homePage` is accepted back as is. Preview writes nothing; execute only the saved execution token.",
            example_args: json!({"values":{"template":"OneColumn","left":[{"form":"CommonForm.Desktop"}]}}),
            help: "skills/cf-edit/SKILL.md",
            format_validators: &[],
        }
}

impl unica_operations::FamilyOperation for InterfaceOperation {
    fn retired_arguments(self) -> &'static [unica_operations::recovery::RetiredArgument] {
        crate::recovery::RETIRED_ARGUMENTS
    }
    fn contract(self) -> Option<unica_operation_contract::OperationContract> {
        Some(self.contract())
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
