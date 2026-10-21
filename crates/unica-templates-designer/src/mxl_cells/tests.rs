use super::{add_area, set_area_cells, CellText, MxlEditError};

const EXPORT: &[u8] =
    include_bytes!("../../../../tests/fixtures/platform_8_3_27/mxl_edit/Template.xml");
const EMPTY: &[u8] =
    include_bytes!("../../../../tests/fixtures/platform_8_3_27/mxl_edit/Empty.xml");

fn body(bytes: &[u8]) -> String {
    let body = bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .expect("platform exports start with a BOM");
    String::from_utf8(body.to_vec()).unwrap()
}

fn cells(list: &[(&str, &str)]) -> Vec<CellText> {
    list.iter()
        .map(|(key, text)| {
            let (row, col) = unica_templates::contracts::parse_mxl_cell_address(key).unwrap();
            CellText {
                key: key.to_string(),
                row,
                col,
                text: text.to_string(),
            }
        })
        .collect()
}

fn set(
    xml: &str,
    area: &str,
    columns: Option<i64>,
    list: &[(&str, &str)],
) -> Result<String, MxlEditError> {
    let mut xml = xml.to_string();
    set_area_cells(&mut xml, area, columns, &cells(list))?;
    Ok(xml)
}

fn add(
    xml: &str,
    area: &str,
    columns: Option<i64>,
    list: &[(&str, &str)],
) -> Result<String, MxlEditError> {
    let mut xml = xml.to_string();
    add_area(&mut xml, area, columns, &cells(list))?;
    Ok(xml)
}

/// The changed window of `after` against `before`: the common prefix and
/// suffix are the bytes the edit left alone.
fn changed_window<'a>(before: &str, after: &'a str) -> (usize, &'a str) {
    let prefix: usize = before
        .chars()
        .zip(after.chars())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    let suffix: usize = before[prefix..]
        .chars()
        .rev()
        .zip(after[prefix..].chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    (prefix, &after[prefix..after.len() - suffix])
}

/// Every expected file is the writer's output that an 8.3.27 import and
/// export reproduced byte for byte (see the fixture README).
#[test]
fn edits_of_a_platform_export_are_what_the_platform_exports_again() {
    let export = body(EXPORT);
    let cases: [(&[u8], String); 6] = [
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-set-text.xml"
            ),
            set(&export, "Шапка", None, &[("R1C1", "Отчёт за год")]).unwrap(),
        ),
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-set-parameter.xml"
            ),
            set(&export, "Шапка", None, &[("R1C4", "Период: год")]).unwrap(),
        ),
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-set-adjacent.xml"
            ),
            set(&export, "Шапка", None, &[("R1C3", "впритык")]).unwrap(),
        ),
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-add-area.xml"
            ),
            add(&export, "Итог", None, &[("R1C1", "Итого"), ("R3C2", "x")]).unwrap(),
        ),
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-empty-add-area.xml"
            ),
            add(
                &body(EMPTY),
                "Header",
                Some(3),
                &[("R1C1", "Номенклатура"), ("R1C2", "Сумма"), ("R2C1", "")],
            )
            .unwrap(),
        ),
        (
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/mxl_edit/expected-grow-last-area.xml"
            ),
            set(
                &add(&export, "Итог", None, &[("R1C1", "Итого")]).unwrap(),
                "Итог",
                None,
                &[("R4C2", "ниже")],
            )
            .unwrap(),
        ),
    ];
    for (index, (expected, actual)) in cases.iter().enumerate() {
        assert_eq!(body(expected), *actual, "case {index}");
    }
}

#[test]
fn a_text_change_rewrites_only_the_content_of_that_cell() {
    let export = body(EXPORT);
    let after = set(&export, "Шапка", None, &[("R1C1", "Отчёт за год")]).unwrap();
    let (_, window) = changed_window(&export, &after);
    // Only the differing characters of the old and new text.
    assert!(!window.contains('<'), "{window:?}");
    assert!("Отчёт за год".contains(window), "{window:?}");
}

#[test]
fn a_new_cell_and_area_add_only_their_own_nodes() {
    let export = body(EXPORT);
    // A cell after the last one: one new slot, no other byte changes.
    let after = set(&export, "Шапка", None, &[("R2C5", "Новая")]).unwrap();
    // The column count grows from 4 to 5 in its own element.
    let resized = after.replacen("<size>5</size>", "<size>4</size>", 1);
    assert_ne!(resized, after);
    let row = export.find("<index>1</index>\r\n\t\t<row>").unwrap();
    let end = row + export[row..].find("\r\n\t\t</row>").unwrap();
    let slot = "\r\n\t\t\t<c>\r\n\t\t\t\t<c>\r\n\t\t\t\t\t<f>0</f>\r\n\t\t\t\t\t<tl>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>Новая</v8:content>\r\n\t\t\t\t\t\t</v8:item>\r\n\t\t\t\t\t</tl>\r\n\t\t\t\t</c>\r\n\t\t\t</c>";
    assert_eq!(
        resized,
        format!("{}{slot}{}", &export[..end], &export[end..])
    );

    // A new area: its rows, its named item, the height; nothing else.
    let after = add(&export, "Итог", None, &[("R1C1", "Итого")]).unwrap();
    let removed: String = after
        .replace("<height>12</height>", "<height>11</height>")
        .replace("<vgRows>12</vgRows>", "<vgRows>11</vgRows>");
    let original_lines: Vec<&str> = export.split("\r\n").collect();
    let lines: Vec<&str> = removed.split("\r\n").collect();
    // Every original line is kept in order; the additions are new lines only.
    let mut cursor = 0;
    for line in &lines {
        if cursor < original_lines.len() && *line == original_lines[cursor] {
            cursor += 1;
        }
    }
    assert_eq!(cursor, original_lines.len(), "an original line changed");
    assert!(
        !after.contains("\n\t<format>\r\n\t\t<font>1</font>\r\n\t</format>"),
        "no format was added"
    );
}

#[test]
fn a_repeated_edit_writes_nothing() {
    let export = body(EXPORT);
    for (area, list) in [
        ("Шапка", vec![("R1C1", "Отчёт за год"), ("R2C5", "Новая")]),
        (
            "Строка",
            vec![("R1C2", "Кол-во"), ("R1C3", "Итого: 5 руб.")],
        ),
        ("Подвал", vec![("R2C1", ""), ("R2C3", "a\r\nb")]),
    ] {
        let once = set(&export, area, Some(6), &list).unwrap();
        let twice = set(&once, area, Some(6), &list).unwrap();
        assert_eq!(once, twice, "{area}");
    }
    let once = add(&export, "Итог", None, &[("R1C1", "Итого")]).unwrap();
    let again = set(&once, "Итог", None, &[("R1C1", "Итого")]).unwrap();
    assert_eq!(once, again);
    assert_eq!(
        set(&export, "Шапка", None, &[("R1C1", "Отчёт о продажах")]).unwrap(),
        export
    );
}

#[test]
fn a_shared_parameter_format_is_copied_once_and_then_reused() {
    let export = body(EXPORT);
    let formats =
        |xml: &str| xml.matches("\t<format>").count() + xml.matches("\t<format/>").count();
    // Format 7 (font 1, Parameter) serves two cells of `Строка`.
    let one = set(&export, "Строка", None, &[("R1C2", "Кол-во")]).unwrap();
    assert_eq!(formats(&one), formats(&export) + 1);
    assert!(one.ends_with("\t<format>\r\n\t\t<font>1</font>\r\n\t</format>\r\n</document>"));
    assert!(!one.contains("<parameter>Количество</parameter>"));
    assert!(
        one.contains("<parameter>Номенклатура</parameter>"),
        "the neighbour keeps its parameter"
    );
    // The second cell is then the only user of format 7: it changes in place,
    // and a third conversion elsewhere reuses the copy.
    let two = set(&one, "Строка", None, &[("R1C1", "Товар")]).unwrap();
    assert_eq!(formats(&two), formats(&one));
    assert!(
        two.contains("<detailParameter>Ссылка</detailParameter>"),
        "the detail parameter stays"
    );
}

#[test]
fn line_endings_and_a_missing_bom_are_kept() {
    let lf = body(EXPORT).replace("\r\n", "\n");
    let after = add(&lf, "Итог", Some(5), &[("R1C1", "Итого"), ("R2C3", "x")]).unwrap();
    assert!(!after.contains('\r'));
    let crlf = body(EXPORT);
    let after = add(&crlf, "Итог", Some(5), &[("R1C1", "Итого"), ("R2C3", "x")]).unwrap();
    assert_eq!(after.matches('\n').count(), after.matches("\r\n").count());
}

#[test]
fn other_translations_of_a_cell_stay() {
    let export = body(EXPORT).replace(
        "<v8:content>Номенклатура</v8:content>\r\n\t\t\t\t\t\t</v8:item>",
        "<v8:content>Номенклатура</v8:content>\r\n\t\t\t\t\t\t</v8:item>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>en</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>Item</v8:content>\r\n\t\t\t\t\t\t</v8:item>",
    );
    let after = set(&export, "Шапка", None, &[("R2C1", "Товар")]).unwrap();
    assert!(after.contains("<v8:content>Товар</v8:content>\r\n\t\t\t\t\t\t</v8:item>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>en</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>Item</v8:content>"));
}

#[test]
fn text_is_stored_the_way_the_platform_stores_it() {
    let after = set(
        &body(EXPORT),
        "Подвал",
        None,
        &[("R2C1", "a & b < c > \"q\"\r\nx\ry")],
    )
    .unwrap();
    assert!(after.contains("<v8:content>a &amp; b &lt; c &gt; \"q\"\nx\ny</v8:content>"));
    let empty = set(&body(EXPORT), "Подвал", None, &[("R2C1", "")]).unwrap();
    assert!(empty.contains("<f>0</f>\r\n\t\t\t\t\t<tl/>"));
    // Empty text in a missing cell writes no cell.
    assert_eq!(
        set(&body(EXPORT), "Подвал", None, &[("R2C3", "")]).unwrap(),
        body(EXPORT)
    );
    assert!(matches!(
        set(&body(EXPORT), "Подвал", None, &[("R2C1", "a\u{1}b")]),
        Err(MxlEditError::Cell { key, .. }) if key == "R2C1"
    ));
    // A cell with a format and no text already holds empty text.
    let formatted = body(EXPORT).replace(
        "<f>0</f>\r\n\t\t\t\t\t<tl>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>Подпись</v8:content>\r\n\t\t\t\t\t\t</v8:item>\r\n\t\t\t\t\t</tl>",
        "<f>3</f>",
    );
    assert_ne!(formatted, body(EXPORT));
    assert_eq!(
        set(&formatted, "Подвал", None, &[("R2C1", "")]).unwrap(),
        formatted
    );
}

#[test]
fn addressed_nodes_the_writer_cannot_change_refuse_without_writing() {
    let export = body(EXPORT);
    let refused = |result: Result<String, MxlEditError>| match result {
        Err(MxlEditError::Source(message)) => message,
        other => panic!("expected a source refusal, got {other:?}"),
    };
    // A cell refusal names the key of the cell as the caller spelled it.
    let cell = |result: Result<String, MxlEditError>, expected: &str| match result {
        Err(MxlEditError::Cell { key, message }) => {
            assert_eq!(key, expected);
            message
        }
        other => panic!("expected a cell refusal, got {other:?}"),
    };
    // R5C2 is covered by the merge that starts at R5C1.
    assert!(cell(set(&export, "Подвал", None, &[("R01C2", "x")]), "R01C2").contains("merged cell"));
    assert!(refused(set(&export, "Угол", None, &[("R1C1", "x")])).contains("`Rectangle`"));
    let rowless = export.replace(
        "<beginRow>3</beginRow>\r\n\t\t\t<endRow>3</endRow>",
        "<beginRow>3</beginRow>\r\n\t\t\t<endRow>2</endRow>",
    );
    assert!(refused(set(&rowless, "Строка", None, &[("R1C1", "x")])).contains("has no rows"));
    let value = export.replace(
        "<parameter>Итого</parameter>",
        "<parameter>Итого</parameter>\r\n\t\t\t\t\t<control/>",
    );
    assert!(cell(set(&value, "Подвал", None, &[("R1C3", "x")]), "R1C3").contains("`control`"));
    // Row 2 has format 6; give it a parameter fill: a new cell would inherit it.
    let filled = export.replace("\t<format>\r\n\t\t<height>20</height>\r\n\t</format>", "\t<format>\r\n\t\t<height>20</height>\r\n\t\t<fillType>Parameter</fillType>\r\n\t</format>");
    let outside = filled.replace("<name>Строка</name>\r\n\t\t<area>\r\n\t\t\t<type>Rows</type>\r\n\t\t\t<beginRow>3</beginRow>", "<name>Строка</name>\r\n\t\t<area>\r\n\t\t\t<type>Rows</type>\r\n\t\t\t<beginRow>2</beginRow>");
    assert!(
        cell(set(&outside, "Строка", None, &[("R1C1", "x")]), "R1C1").contains("`Parameter` fill")
    );
    // So would a cell of a column set whose own format fills from a parameter.
    let set_fill = export.replace(
        "\t<columns>\r\n\t\t<size>4</size>",
        "\t<columns>\r\n\t\t<size>4</size>\r\n\t\t<formatIndex>4</formatIndex>",
    );
    assert!(
        cell(set(&set_fill, "Подвал", None, &[("R2C3", "x")]), "R2C3").contains("`Parameter` fill")
    );
    // `Шапка` is followed by other rows: it does not grow.
    assert!(matches!(
        set(&export, "Шапка", None, &[("R3C1", "x")]),
        Err(MxlEditError::RowOutsideArea { key, rows: 2 }) if key == "R3C1"
    ));
    assert_eq!(
        set(&export, "Нет", None, &[("R1C1", "x")]),
        Err(MxlEditError::AreaMissing)
    );
    assert_eq!(
        set(&export, "шапка", None, &[("R1C1", "x")]),
        Err(MxlEditError::AreaSpelling("Шапка".to_string()))
    );
    assert_eq!(
        add(&export, "ШАПКА", None, &[("R1C1", "x")]),
        Err(MxlEditError::AreaExists("Шапка".to_string()))
    );
}

#[test]
fn outside_rows_drawings_and_other_column_sets_stay_untouched() {
    // The fixture has an outside row, compressed rows, a second column set,
    // Columns and Rectangle areas and a merge; writing area `Строка` keeps
    // all of them byte for byte.
    let export = body(EXPORT);
    let after = set(&export, "Строка", Some(4), &[("R1C4", "Резерв")]).unwrap();
    let (_, window) = changed_window(&export, &after);
    assert_eq!(window, "Резерв");
}

/// The hand-made templates the former whole-document writer refused (#1311):
/// the cell changes and every other byte stays.
#[test]
fn templates_the_former_writer_refused_change_only_the_cell() {
    macro_rules! fixture {
        ($name:literal) => {
            (
                $name,
                include_str!(concat!(
                    "../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/",
                    $name,
                    "/Ext/Template.xml"
                )),
            )
        };
    }
    for (name, xml) in [
        fixture!("UnsafeOutsideRow"),
        fixture!("UnsafeOverlappingAreas"),
        fixture!("UnsafePhysicalAreaOrder"),
        fixture!("UnsafeCellTranslations"),
        fixture!("UnsafeMultipleColumnSets"),
        fixture!("UnsafeColumnsId"),
        fixture!("UnsafeLanguageSettings"),
        fixture!("UnsafeDrawing"),
        fixture!("UnsafeColumnArea"),
        fixture!("UnsafeFormatTranslations"),
        fixture!("UnsafeCompressedOutsideRow"),
    ] {
        let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
        match (name, set(xml, "A", None, &[("R1C1", "Edited")])) {
            ("UnsafeColumnArea", Err(MxlEditError::Source(message))) => {
                assert!(message.contains("`Columns` area"), "{message}");
            }
            ("UnsafeCompressedOutsideRow", Err(MxlEditError::Source(message))) => {
                assert!(message.contains("overlap other rows"), "{message}");
            }
            (_, Ok(after)) => {
                // Only the text of the cell differs.
                let (_, window) = changed_window(xml, &after);
                assert!(
                    "Edited".contains(window) && !window.is_empty(),
                    "{name}: {window:?}"
                );
                assert_eq!(after.matches("Edited").count(), 1, "{name}");
            }
            (_, other) => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn a_converted_cell_reuses_an_identical_format() {
    // Format 9 (font 0, Parameter) serves two cells; without the fill it
    // equals format 3 (font 0), which the converted cell then uses.
    let export = body(EXPORT).replacen(
        "<f>7</f>\r\n\t\t\t\t\t<parameter>Количество</parameter>",
        "<f>9</f>\r\n\t\t\t\t\t<parameter>Количество</parameter>",
        1,
    );
    let after = set(&export, "Строка", None, &[("R1C2", "Кол-во")]).unwrap();
    assert_eq!(
        after.matches("\t<format>").count(),
        export.matches("\t<format>").count()
    );
    assert!(after.contains("<f>3</f>\r\n\t\t\t\t\t<tl>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>Кол-во</v8:content>"));
    assert!(
        after.contains("<f>9</f>\r\n\t\t\t\t\t<parameter>Итого</parameter>"),
        "the other user keeps format 9"
    );
}

#[test]
fn a_compressed_formatted_range_splits_into_copies() {
    let export = body(EXPORT)
        .replace(
            "<index>6</index>\r\n\t\t<indexTo>9</indexTo>\r\n\t\t<row>\r\n\t\t\t<empty>true</empty>",
            "<index>6</index>\r\n\t\t<indexTo>9</indexTo>\r\n\t\t<row>\r\n\t\t\t<formatIndex>6</formatIndex>\r\n\t\t\t<empty>true</empty>",
        )
        .replace(
            "<beginRow>4</beginRow>\r\n\t\t\t<endRow>5</endRow>",
            "<beginRow>4</beginRow>\r\n\t\t\t<endRow>9</endRow>",
        );
    let after = set(&export, "Подвал", None, &[("R4C1", "x")]).unwrap();
    let piece = |first: i64, to: Option<i64>, content: &str| {
        let to = to.map_or(String::new(), |to| {
            format!("\r\n\t\t<indexTo>{to}</indexTo>")
        });
        format!("\t<rowsItem>\r\n\t\t<index>{first}</index>{to}\r\n\t\t<row>\r\n\t\t\t<formatIndex>6</formatIndex>\r\n\t\t\t{content}\r\n\t\t</row>\r\n\t</rowsItem>\r\n")
    };
    let cell = "<c>\r\n\t\t\t\t<c>\r\n\t\t\t\t\t<f>0</f>\r\n\t\t\t\t\t<tl>\r\n\t\t\t\t\t\t<v8:item>\r\n\t\t\t\t\t\t\t<v8:lang>ru</v8:lang>\r\n\t\t\t\t\t\t\t<v8:content>x</v8:content>\r\n\t\t\t\t\t\t</v8:item>\r\n\t\t\t\t\t</tl>\r\n\t\t\t\t</c>\r\n\t\t\t</c>";
    let expected = format!(
        "{}{}{}",
        piece(6, None, "<empty>true</empty>"),
        piece(7, None, cell),
        piece(8, Some(9), "<empty>true</empty>")
    );
    assert!(after.contains(&expected), "{after}");
}
