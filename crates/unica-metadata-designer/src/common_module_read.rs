//! Чтение свойств общего модуля из его дескриптора для порта
//! [`unica_metadata::read_codec::MetadataReadCodec`].
use roxmltree::{Document, Node};
use unica_metadata::read_codec::{MetaPropertySource, ReadFinding};
use unica_metadata::{MetaInfoPropertyValue, MetadataKind};

const NS: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Разобранный дескриптор общего модуля. Документ разбирается один раз,
/// свойства наблюдаются по имени.
pub struct CommonModuleProperties<'a> {
    document: Document<'a>,
}

impl<'a> CommonModuleProperties<'a> {
    /// Разбирает дескриптор и проверяет, что в нём один `CommonModule` с
    /// одним прямым `Properties` из пространства имён метаданных.
    /// Повреждение структуры не становится отсутствием локального свойства.
    pub fn parse(text: &'a str) -> Result<Self, String> {
        let document = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|error| error.to_string())?;
        container(&document)?;
        Ok(Self { document })
    }

    fn properties(&self) -> Result<Node<'_, 'a>, ReadFinding> {
        container(&self.document).map_err(|message| ReadFinding {
            field: "properties".to_string(),
            message,
        })
    }
}

impl MetaPropertySource for CommonModuleProperties<'_> {
    fn info(&self, name: &str) -> Result<Option<MetaInfoPropertyValue>, ReadFinding> {
        crate::info_projection::observe_meta_info_property(
            self.properties()?,
            MetadataKind::CommonModule,
            name,
        )
        .map_err(|error| ReadFinding {
            field: error.field,
            message: error.message,
        })
    }

    fn borrowing(&self, name: &str) -> Result<Option<String>, ReadFinding> {
        crate::info_projection::observe_meta_borrowing_property(
            self.properties()?,
            MetadataKind::CommonModule,
            name,
        )
        .map_err(|error| ReadFinding {
            field: error.field,
            message: error.message,
        })
    }
}

fn container<'d, 'i>(document: &'d Document<'i>) -> Result<Node<'d, 'i>, String> {
    let objects = document
        .root_element()
        .children()
        .filter(Node::is_element)
        .collect::<Vec<_>>();
    let [object] = objects.as_slice() else {
        return Err("common module descriptor must contain one object".to_string());
    };
    if !object.has_tag_name((NS, "CommonModule")) {
        return Err("common module descriptor has the wrong object kind".to_string());
    }
    let containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(NS) => Ok(*node),
        _ => {
            Err("common module descriptor must contain one direct Properties container".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(object: &str) -> String {
        format!("<MetaDataObject xmlns=\"{NS}\" version=\"2.20\">{object}</MetaDataObject>")
    }

    #[test]
    fn damaged_structure_is_refused_before_any_property_is_read() {
        for (object, message) in [
            ("", "one object"),
            ("<Catalog><Properties/></Catalog>", "wrong object kind"),
            ("<CommonModule/>", "one direct Properties container"),
            (
                "<CommonModule><Properties/><Properties/></CommonModule>",
                "one direct Properties container",
            ),
        ] {
            let error = CommonModuleProperties::parse(&descriptor(object))
                .err()
                .expect("damaged structure");
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn absent_properties_are_none_and_present_ones_are_observed() {
        let text = descriptor(
            "<CommonModule><Properties><Name>Shared</Name><ObjectBelonging>Adopted</ObjectBelonging></Properties></CommonModule>",
        );
        let properties = CommonModuleProperties::parse(&text).unwrap();
        assert_eq!(
            properties.borrowing("ObjectBelonging"),
            Ok(Some("Adopted".to_string()))
        );
        assert_eq!(
            properties.borrowing("ExtendedConfigurationObject"),
            Ok(None)
        );
        assert_eq!(properties.info("Global"), Ok(None));
    }
}
