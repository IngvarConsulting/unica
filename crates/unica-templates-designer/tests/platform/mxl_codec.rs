use super::test_support::platform_mxl_fixture;
use super::*;
use crate::mxl_root::empty_spreadsheet_document_xml;
use std::fs;
use unica_metadata::support_state::{ObjectSupportData, ObjectSupportState};
#[test]
fn mxl_info_shared_core_parses_admitted_bytes_without_workspace_resolution() {
    let bytes = fs::read(platform_mxl_fixture()).unwrap();
    let support = ObjectSupportData {
        state: ObjectSupportState::EditableWithSupport,
        direct_edit_safe: Some(true),
    };

    let data = parse_mxl_info_xml(&bytes, "Печать", support.clone(), false).unwrap();
    let value = serde_json::to_value(data).unwrap();

    assert_eq!(value["name"], "Печать");
    assert_eq!(value["support"], serde_json::to_value(support).unwrap());
    assert_eq!(value["rows"], 0);
    assert!(value["outside"]["content"].is_null());
}

/// `area.add` and `mxl.set` stage a real template through decompile and
/// compile. On real BSP print forms the round trip must yield well-formed
/// spreadsheet XML that keeps the named areas. The round trip is neither
/// byte- nor row-exact (the power-of-attorney form comes back with 119 of
/// its 159 logical rows); #1448 part B replaces it with node edits.
#[test]
fn mxl_round_trip_of_real_bsp_templates_keeps_named_areas() {
    let bsp = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/unica_mcp_script_parity/bsp/mxl");
    for template in [
        "Catalogs__МашиночитаемыеДоверенности__ПФ_MXL_Квитанция",
        "Catalogs__МашиночитаемыеДоверенности__ПФ_MXL_Доверенность",
    ] {
        let source = fs::read_to_string(bsp.join(template).join("Template.xml")).unwrap();
        let decompiled = mxl_decompile_document(&source, template).unwrap();
        let definition: Value = serde_json::from_str(&decompiled.json_text).unwrap();

        let compiled = mxl_compile_document(&definition).unwrap();
        let document = Document::parse(&compiled.xml)
            .unwrap_or_else(|error| panic!("{template}: compiled MXL is not XML: {error}"));
        assert_eq!(document.root_element().tag_name().name(), "document");

        let again = mxl_decompile_document(&compiled.xml, template).unwrap();
        assert_eq!(again.area_count, decompiled.area_count, "{template}");
    }
}

#[test]
fn mxl_decompile_counts_only_exact_platform_sentinel_as_zero_rows() {
    let decompiled_height = |xml: &str| {
        mxl_decompile_document(xml, "Template.xml")
            .unwrap()
            .logical_height
    };
    let exact = fs::read_to_string(platform_mxl_fixture()).unwrap();
    assert_eq!(decompiled_height(&exact), 0);

    let lookalike_xml = empty_spreadsheet_document_xml().replace(
        "\t<vgRows>0</vgRows>",
        "\t<height>0</height>\n\t<vgRows>0</vgRows>",
    );
    assert_eq!(decompiled_height(&lookalike_xml), 1);

    let mut missing_row_xml = empty_spreadsheet_document_xml();
    let row_range = {
        let document = Document::parse(&missing_row_xml).unwrap();
        let rows = document
            .root_element()
            .children()
            .filter(|node| node.has_tag_name((MXL_DOCUMENT_NS, "rowsItem")))
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1, "the sentinel must have exactly one rowsItem");
        rows[0].range()
    };
    missing_row_xml.replace_range(row_range, "");
    assert_eq!(decompiled_height(&missing_row_xml), 1);
}
