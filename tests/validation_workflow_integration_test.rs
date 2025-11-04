//! Integration tests for comprehensive validation workflow
//!
//! These tests verify the complete end-to-end validation process including:
//! - Performance metrics collection
//! - Result aggregation and summary generation
//! - Error handling and recovery

use std::process::Command;
use tempfile::TempDir;
use tokio::fs;

/// Helper to create test XML and schema files
async fn create_test_files(temp_dir: &std::path::Path) -> std::io::Result<()> {
    // Create a simple schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="element" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

    let schema_file = temp_dir.join("test.xsd");
    fs::write(&schema_file, schema_content).await?;

    // Create multiple XML files
    for i in 0..3 {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="test.xsd">
    <element>content{}</element>
</root>"#,
            i
        );

        let xml_file = temp_dir.join(format!("test{}.xml", i));
        fs::write(&xml_file, xml_content).await?;
    }

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_comprehensive_validation_workflow() {
    let temp_dir = TempDir::new().unwrap();
    create_test_files(temp_dir.path()).await.unwrap();

    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--release"])
        .output()
        .expect("Failed to build binary");

    assert!(
        build_output.status.success(),
        "Failed to build binary: {}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    // Run validation with verbose output to get performance metrics
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--verbose")
        .output()
        .expect("Failed to run validation");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Print output for debugging
    println!("STDOUT:\n{}", stdout);
    println!("STDERR:\n{}", stderr);

    // Verify successful execution
    assert!(output.status.success(), "Validation failed: {}", stderr);

    // Verify comprehensive validation workflow output
    assert!(stdout.contains("Validation completed"));
    assert!(stdout.contains("Files processed: 3"));
    assert!(stdout.contains("Valid: 3"));
    assert!(stdout.contains("Success rate: 100.0%"));

    // Verify performance metrics are included
    assert!(stdout.contains("Performance Metrics:"));
    assert!(stdout.contains("Discovery time:"));
    assert!(stdout.contains("Validation time:"));
    assert!(stdout.contains("Average time per file:"));
    assert!(stdout.contains("Throughput:"));
    assert!(stdout.contains("Concurrent validations:"));

    // Verify schemas are reported
    assert!(stdout.contains("Schemas used: 1"));
}

#[tokio::test]
#[ignore]
async fn test_validation_with_invalid_files() {
    let temp_dir = TempDir::new().unwrap();

    // Create schema that requires a specific element
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="required" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

    fs::write(temp_dir.path().join("strict.xsd"), schema_content)
        .await
        .unwrap();

    // Create valid XML file
    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="strict.xsd">
    <required>value</required>
</root>"#;
    fs::write(temp_dir.path().join("valid.xml"), valid_xml)
        .await
        .unwrap();

    // Create invalid XML file (missing required element)
    let invalid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="strict.xsd">
    <optional>value</optional>
</root>"#;
    fs::write(temp_dir.path().join("invalid.xml"), invalid_xml)
        .await
        .unwrap();

    // Run validation
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--verbose")
        .output()
        .expect("Failed to run validation");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Print output for debugging
    println!("STDOUT:\n{}", stdout);
    println!("STDERR:\n{}", stderr);
    println!("Exit code: {}", output.status.code().unwrap_or(-1));

    // The application may exit with non-zero code for invalid files
    // but should still produce output showing the validation results

    // Verify results show files were processed
    assert!(stdout.contains("Files processed: 2"));
    // Verify the comprehensive validation workflow completed
    assert!(stdout.contains("Validation completed"));
    assert!(stdout.contains("Performance Metrics:"));
}

#[tokio::test]
#[ignore]
async fn test_validation_with_no_schema_reference() {
    let temp_dir = TempDir::new().unwrap();

    // Create XML file without schema reference
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <element>content</element>
</root>"#;
    fs::write(temp_dir.path().join("test.xml"), xml_content)
        .await
        .unwrap();

    // Run validation
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--verbose")
        .output()
        .expect("Failed to run validation");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should complete successfully
    assert!(output.status.success());

    // Verify results show skipped files
    assert!(stdout.contains("Files processed: 1"));
    assert!(stdout.contains("Skipped: 1"));
    assert!(stdout.contains("Success rate: 0.0%"));
}

#[tokio::test]
#[ignore]
async fn test_empty_directory_validation() {
    let temp_dir = TempDir::new().unwrap();

    // Create empty subdirectory
    let empty_dir = temp_dir.path().join("empty");
    fs::create_dir(&empty_dir).await.unwrap();

    // Run validation on empty directory
    let output = Command::new("./target/release/validate-xml")
        .arg(&empty_dir)
        .arg("--verbose")
        .output()
        .expect("Failed to run validation");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should complete successfully
    assert!(output.status.success());

    // Verify results show no files processed
    assert!(stdout.contains("Files processed: 0"));
    assert!(stdout.contains("Success rate: 0.0%"));

    // Performance metrics should still be present
    assert!(stdout.contains("Performance Metrics:"));
    assert!(stdout.contains("Throughput: 0.0 files/second"));
}
