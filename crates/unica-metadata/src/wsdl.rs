//! Observed declarations of a local WSDL companion, independent of SOAP execution.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QName {
    pub namespace: String,
    pub local_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub rows: Vec<Row>,
    pub limits: Vec<String>,
}

impl Definition {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            rows: Vec::new(),
            limits: vec![message.into()],
        }
    }
}

/// Positions refer to declarations in this document, never to logical addresses.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub source_order: usize,
    pub parent_order: Option<usize>,
    #[serde(flatten)]
    pub fact: Fact,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Fact {
    Definitions {
        name: Option<String>,
        target_namespace: Option<String>,
    },
    Types {},
    Message {
        name: Option<String>,
        namespace: String,
    },
    Part {
        name: Option<String>,
        element: Option<QName>,
        r#type: Option<QName>,
    },
    PortType {
        name: Option<String>,
        namespace: String,
    },
    Operation {
        name: Option<String>,
        parameter_order: Option<String>,
    },
    Input {
        name: Option<String>,
        message: Option<QName>,
    },
    Output {
        name: Option<String>,
        message: Option<QName>,
    },
    Fault {
        name: Option<String>,
        message: Option<QName>,
    },
    Binding {
        name: Option<String>,
        namespace: String,
        r#type: Option<QName>,
    },
    SoapBinding {
        style: Option<String>,
        transport: Option<String>,
    },
    SoapOperation {
        soap_action: Option<String>,
        style: Option<String>,
    },
    SoapBody {
        r#use: Option<String>,
        namespace: Option<String>,
        encoding_style: Option<String>,
        parts: Option<String>,
    },
    SoapFault {
        name: Option<String>,
        r#use: Option<String>,
        namespace: Option<String>,
        encoding_style: Option<String>,
    },
    Service {
        name: Option<String>,
        namespace: String,
    },
    Port {
        name: Option<String>,
        binding: Option<QName>,
    },
    SoapAddress {
        location: Option<String>,
    },
    Schema {
        target_namespace: Option<String>,
        element_form_default: Option<String>,
        attribute_form_default: Option<String>,
    },
    ComplexType {
        name: Option<String>,
        mixed: Option<bool>,
        r#abstract: Option<bool>,
    },
    Sequence {
        min_occurs: Option<String>,
        max_occurs: Option<String>,
    },
    Element {
        name: Option<String>,
        r#ref: Option<QName>,
        r#type: Option<QName>,
        min_occurs: Option<String>,
        max_occurs: Option<String>,
        nillable: Option<bool>,
        form: Option<String>,
    },
    Attribute {
        name: Option<String>,
        r#ref: Option<QName>,
        r#type: Option<QName>,
        r#use: Option<String>,
        form: Option<String>,
    },
    WsdlImport {
        namespace: Option<String>,
        location: Option<String>,
    },
    SchemaImport {
        namespace: Option<String>,
        schema_location: Option<String>,
    },
    SchemaInclude {
        schema_location: Option<String>,
    },
}
