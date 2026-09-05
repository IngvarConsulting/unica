//! Исследование: корпус XML публичного инструмента для платформы 8.3.27.
//!
//! Не тест: проводится один раз, результат закрепляется корпусом в дереве.
//! Цель собирается только с фичей `research`, в плане конвейера её нет.
//! Запуск — `scripts/research/xml-corpus.sh`.

#[allow(dead_code)]
#[path = "../format_8_3_27_xml_corpus.rs"]
mod corpus;

#[test]
fn generate_platform_xml_corpus() {
    let output = corpus::configured_output_directory().expect("safe UNICA_XML_CORPUS_DIR");

    let manifest =
        corpus::generate_corpus(&output).expect("generate complete public-tool XML corpus");

    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.profile, "1c-8.3.27-export-2.20");
    assert_eq!(manifest.cases.len(), corpus::EXECUTABLE_CASES.len());
}
