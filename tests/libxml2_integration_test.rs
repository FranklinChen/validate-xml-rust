use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;

use validate_xml::{LibXml2Wrapper, ValidationResult};

const SIMPLE_XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

const VALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>Hello World</root>"#;

const INVALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root><invalid>content</invalid></root>"#;

#[tokio::test]
#[ignore]
async fn test_end_to_end_validation_success() {
    let wrapper = LibXml2Wrapper::new();

    // Parse schema
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = wrapper.parse_schema_from_memory(schema_data).unwrap();

    // Create temporary XML file
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(VALID_XML.as_bytes()).unwrap();
    let temp_path = temp_file.path();

    // Validate file
    let result = wrapper.validate_file(&schema, temp_path).unwrap();

    assert_eq!(result, ValidationResult::Valid);
    assert!(result.is_valid());
}

#[tokio::test]
#[ignore]
async fn test_end_to_end_validation_failure() {
    let wrapper = LibXml2Wrapper::new();

    // Parse schema
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = wrapper.parse_schema_from_memory(schema_data).unwrap();

    // Create temporary XML file with invalid content
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(INVALID_XML.as_bytes()).unwrap();
    let temp_path = temp_file.path();

    // Validate file
    let result = wrapper.validate_file(&schema, temp_path).unwrap();

    assert!(result.is_invalid());
    assert!(!result.is_valid());
}

#[tokio::test]
#[ignore]
async fn test_validation_nonexistent_file() {
    let wrapper = LibXml2Wrapper::new();

    // Parse schema
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = wrapper.parse_schema_from_memory(schema_data).unwrap();

    // Try to validate non-existent file
    let nonexistent_path = PathBuf::from("/nonexistent/file.xml");
    let result = wrapper.validate_file(&schema, &nonexistent_path);

    assert!(result.is_err());
}

#[tokio::test]
#[ignore]
async fn test_concurrent_validations() {
    let wrapper = LibXml2Wrapper::new();

    // Parse schema
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = wrapper.parse_schema_from_memory(schema_data).unwrap();

    // Create multiple temporary files
    let mut temp_files = Vec::new();
    for _ in 0..5 {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(VALID_XML.as_bytes()).unwrap();
        temp_files.push(temp_file);
    }

    // Validate all files concurrently
    let tasks: Vec<_> = temp_files
        .iter()
        .map(|temp_file| {
            let wrapper_ref = &wrapper;
            let schema_ref = &schema;
            let path = temp_file.path();

            async move { wrapper_ref.validate_file(schema_ref, path) }
        })
        .collect();

    // Wait for all validations to complete
    let results: Vec<_> = futures::future::join_all(tasks).await;

    // All should succeed
    for result in results {
        let validation_result = result.unwrap();
        assert!(validation_result.is_valid());
    }
}

#[tokio::test]
#[ignore]
async fn test_schema_reuse() {
    let wrapper = LibXml2Wrapper::new();

    // Parse schema once
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = wrapper.parse_schema_from_memory(schema_data).unwrap();

    // Use the same schema for multiple validations
    for i in 0..3 {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(VALID_XML.as_bytes()).unwrap();

        let result = wrapper
            .validate_file(&schema, temp_file.path())
            .unwrap();
        assert!(result.is_valid(), "Validation {} failed", i);
    }
}
