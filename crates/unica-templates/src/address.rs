//! Владелец содержимого макета: `Owner.Name.Template.Name` в логическом адресе.
//! Общий разбор для семейств, которые пишут тело макета (СКД и табличный
//! документ); узлы ниже макета разбирает семейство.
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};

/// Адрес макета и сегменты под ним. Отказы называет семейство: `shape` —
/// для адреса короче `Owner.Name.Template.Name`, `not_template` — когда
/// второй сегмент не макет.
pub fn template_owner<'a>(
    target: &'a QualifiedAddress,
    op_index: usize,
    shape: &str,
    not_template: &str,
) -> Result<(MetadataAddress, &'a [AddressSegment]), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let bad = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message.to_string())
            .at_path(at_path.clone())
    };
    let [owner, template, rest @ ..] = target.segments() else {
        return Err(bad(shape));
    };
    if template.kind() != NodeKind::Template {
        return Err(bad(not_template));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad("the template owner must be named"))?;
    let template_name = template
        .name()
        .ok_or_else(|| bad("the template must be named"))?;
    let template = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!(
            "{}.{owner_name}.Template.{template_name}",
            owner.kind().as_str()
        ),
    )
    .map_err(|error| bad(&error.to_string()))?;
    Ok((template, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(raw: &str) -> String {
        let target = QualifiedAddress::parse(raw).unwrap();
        template_owner(&target, 2, "shape refusal", "not a template refusal")
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn refusals_name_the_calling_family_shape_and_template_node() {
        assert!(refusal("main:Report.Sales").contains("shape refusal"));
        assert!(refusal("main:Report.Sales.Form.Main").contains("not a template refusal"));
        let target =
            QualifiedAddress::parse("main:Report.Sales.Template.Schema.DataSet.D").unwrap();
        let (template, rest) = template_owner(&target, 0, "shape", "kind").unwrap();
        assert_eq!(template.as_str(), "Report.Sales.Template.Schema");
        assert_eq!(rest.len(), 1);
    }
}
