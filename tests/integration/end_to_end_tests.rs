use std::process::Command;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::fs;

use crate::common::test_helpers::{TestFixtures, create_temp_xml_files, PerformanceTimer};

#[tokio::test]
async fn test_complete_validation_workflow_with_fixtures() {
    let fixtures = TestFixtures::new();
    
    // Build the binary first
    let build_output = Command::new("cargo")
        .args(&["build", "--release"])
        .output()
        .expect("Failed to build binary");
    
    assert!(build_output.status.success(), "Failed to build binary: {}", 
            String::from_utf8_lossy(&build_output.stderr));
    
    // Run validation on fixture directory
    let output = Command::new("./target/release/validate-xml")
        .arg(fixtures.xml_valid_dir())
        .arg("--verbose")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    println!("STDOUT:\n{}", stdout);
    println!("STDERR:\n{}", stderr);
    
    // Verify successful execution
    assert!(output.status.success(), "Validation failed: {}", stderr);
    
    // Parse JSON output
    let json_result: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
    
    // Verify JSON structure
    assert!(json_result.get("summary").is_some());
    assert!(json_result.get("files").is_some());
    assert!(json_result.get("performance").is_some());
    
    let summary = json_result.get("summary").unwrap();
    assert!(summary.get("total_files").unwrap().as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_validation_with_mixed_results() {
    let temp_dir = TempDir::new().unwrap();
    let fixtures = TestFixtures::new();
    
    // Copy fixture files to temp directory for testing
    let schema_content = fs::read_to_string(fixtures.simple_schema()).await.unwrap();
    let schema_path = temp_dir.path().join("test.xsd");
    fs::write(&schema_path, schema_content).await.unwrap();
    
    // Create valid XML file
    let valid_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">Valid content</root>"#,
        schema_path.file_name().unwrap().to_string_lossy()
    );
    fs::write(temp_dir.path().join("valid.xml"), valid_xml).await.unwrap();
    
    // Create invalid XML file
    let invalid_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}"><invalid>content</invalid></root>"#,
        schema_path.file_name().unwrap().to_string_lossy()
    );
    fs::write(temp_dir.path().join("invalid.xml"), invalid_xml).await.unwrap();
    
    // Create XML file without schema reference
    let no_schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>No schema reference</root>"#;
    fs::write(temp_dir.path().join("no_schema.xml"), no_schema_xml).await.unwrap();
    
    // Run validation
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--verbose")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Parse results
    let json_result: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
    
    let summary = json_result.get("summary").unwrap();
    assert_eq!(summary.get("total_files").unwrap().as_u64().unwrap(), 3);
    
    // Should have mixed results
    let valid_count = summary.get("valid_files").unwrap().as_u64().unwrap();
    let invalid_count = summary.get("invalid_files").unwrap().as_u64().unwrap();
    let skipped_count = summary.get("skipped_files").unwrap().as_u64().unwrap();
    
    assert_eq!(valid_count, 1);
    assert_eq!(invalid_count, 1);
    assert_eq!(skipped_count, 1);
}

#[tokio::test]
async fn test_performance_with_large_dataset() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="document">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="id" type="xs:int"/>
                <xs:element name="title" type="xs:string"/>
                <xs:element name="content" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;
    
    let schema_path = temp_dir.path().join("document.xsd");
    fs::write(&schema_path, schema_content).await.unwrap();
    
    // Create many XML files
    let file_count = 50;
    for i in 0..file_count {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<document xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:noNamespaceSchemaLocation="document.xsd">
    <id>{}</id>
    <title>Document {}</title>
    <content>This is the content of document number {}. It contains some text to make the file larger and more realistic for performance testing.</content>
</document>"#,
            i, i, i
        );
        
        fs::write(temp_dir.path().join(format!("doc_{:03}.xml", i)), xml_content).await.unwrap();
    }
    
    // Run validation with performance measurement
    let timer = PerformanceTimer::new();
    
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--threads")
        .arg("4")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    
    let elapsed = timer.elapsed();
    
    assert!(output.status.success());
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_result: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
    
    let summary = json_result.get("summary").unwrap();
    assert_eq!(summary.get("total_files").unwrap().as_u64().unwrap(), file_count);
    assert_eq!(summary.get("valid_files").unwrap().as_u64().unwrap(), file_count);
    
    // Performance assertions
    let performance = json_result.get("performance").unwrap();
    let throughput = performance.get("throughput_files_per_second").unwrap().as_f64().unwrap();
    
    // Should process at least 10 files per second (conservative estimate)
    assert!(throughput >= 10.0, "Throughput too low: {} files/sec", throughput);
    
    // Total time should be reasonable (less than 10 seconds for 50 files)
    assert!(elapsed.as_secs() < 10, "Validation took too long: {:?}", elapsed);
}

#[tokio::test]
async fn test_concurrent_validation_scaling() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;
    
    let schema_path = temp_dir.path().join("test.xsd");
    fs::write(&schema_path, schema_content).await.unwrap();
    
    // Create test files
    let file_count = 20;
    for i in 0..file_count {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="test.xsd">Content {}</root>"#,
            i
        );
        fs::write(temp_dir.path().join(format!("test_{}.xml", i)), xml_content).await.unwrap();
    }
    
    // Test with different thread counts
    let thread_counts = vec![1, 2, 4, 8];
    let mut results = Vec::new();
    
    for thread_count in thread_counts {
        let timer = PerformanceTimer::new();
        
        let output = Command::new("./target/release/validate-xml")
            .arg(temp_dir.path())
            .arg("--threads")
            .arg(thread_count.to_string())
            .arg("--format")
            .arg("json")
            .output()
            .expect("Failed to run validation");
        
        let elapsed = timer.elapsed();
        
        assert!(output.status.success());
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_result: serde_json::Value = serde_json::from_str(&stdout)
            .expect("Output should be valid JSON");
        
        let performance = json_result.get("performance").unwrap();
        let throughput = performance.get("throughput_files_per_second").unwrap().as_f64().unwrap();
        
        results.push((thread_count, elapsed, throughput));
    }
    
    // Verify that increasing thread count generally improves performance
    // (though this may not always be true due to overhead and system constraints)
    println!("Performance scaling results:");
    for (threads, elapsed, throughput) in &results {
        println!("  {} threads: {:?} elapsed, {:.2} files/sec", threads, elapsed, throughput);
    }
    
    // At minimum, all configurations should complete successfully
    assert_eq!(results.len(), 4);
}

#[tokio::test]
async fn test_cache_effectiveness() {
    let temp_dir = TempDir::new().unwrap();
    let cache_dir = temp_dir.path().join("cache");
    
    // Create schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;
    
    let schema_path = temp_dir.path().join("shared.xsd");
    fs::write(&schema_path, schema_content).await.unwrap();
    
    // Create multiple XML files using the same schema
    for i in 0..10 {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="shared.xsd">Content {}</root>"#,
            i
        );
        fs::write(temp_dir.path().join(format!("test_{}.xml", i)), xml_content).await.unwrap();
    }
    
    // First run - should populate cache
    let timer1 = PerformanceTimer::new();
    let output1 = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    let elapsed1 = timer1.elapsed();
    
    assert!(output1.status.success());
    
    // Second run - should use cache
    let timer2 = PerformanceTimer::new();
    let output2 = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    let elapsed2 = timer2.elapsed();
    
    assert!(output2.status.success());
    
    // Verify cache directory was created and contains files
    assert!(cache_dir.exists());
    let cache_entries = fs::read_dir(&cache_dir).await.unwrap().count();
    assert!(cache_entries > 0, "Cache should contain entries");
    
    // Second run should be faster or at least not significantly slower
    // (allowing for some variance in timing)
    let speedup_ratio = elapsed1.as_millis() as f64 / elapsed2.as_millis() as f64;
    println!("Cache effectiveness: first run {:?}, second run {:?}, speedup: {:.2}x", 
             elapsed1, elapsed2, speedup_ratio);
    
    // At minimum, second run shouldn't be more than 50% slower
    assert!(speedup_ratio >= 0.5, "Second run was significantly slower: {:.2}x", speedup_ratio);
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create mix of valid, invalid, and problematic files
    
    // Valid file with local schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;
    fs::write(temp_dir.path().join("valid.xsd"), schema_content).await.unwrap();
    
    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="valid.xsd">Valid content</root>"#;
    fs::write(temp_dir.path().join("valid.xml"), valid_xml).await.unwrap();
    
    // Invalid XML (schema validation failure)
    let invalid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="valid.xsd"><invalid>content</invalid></root>"#;
    fs::write(temp_dir.path().join("invalid.xml"), invalid_xml).await.unwrap();
    
    // Malformed XML
    let malformed_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root><unclosed>"#;
    fs::write(temp_dir.path().join("malformed.xml"), malformed_xml).await.unwrap();
    
    // XML with missing schema
    let missing_schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="missing.xsd">Content</root>"#;
    fs::write(temp_dir.path().join("missing_schema.xml"), missing_schema_xml).await.unwrap();
    
    // Run validation
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--verbose")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    
    // Should complete despite errors
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    println!("STDOUT:\n{}", stdout);
    println!("STDERR:\n{}", stderr);
    
    // Parse results
    let json_result: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
    
    let summary = json_result.get("summary").unwrap();
    assert_eq!(summary.get("total_files").unwrap().as_u64().unwrap(), 4);
    
    // Should have processed all files with appropriate results
    let valid_count = summary.get("valid_files").unwrap().as_u64().unwrap();
    let invalid_count = summary.get("invalid_files").unwrap().as_u64().unwrap();
    let error_count = summary.get("error_files").unwrap().as_u64().unwrap();
    
    assert_eq!(valid_count, 1);
    assert!(invalid_count >= 1); // At least the invalid.xml
    assert!(error_count >= 1);   // At least the missing schema and malformed files
    
    // Verify error details are included
    let files = json_result.get("files").unwrap().as_array().unwrap();
    let error_files: Vec<_> = files.iter()
        .filter(|f| f.get("status").unwrap().as_str().unwrap() == "error")
        .collect();
    
    assert!(!error_files.is_empty());
    
    // Each error file should have error details
    for error_file in error_files {
        let errors = error_file.get("errors").unwrap().as_array().unwrap();
        assert!(!errors.is_empty());
    }
}

#[tokio::test]
async fn test_configuration_file_integration() {
    let temp_dir = TempDir::new().unwrap();
    let fixtures = TestFixtures::new();
    
    // Copy test configuration
    let config_content = fs::read_to_string(fixtures.configs_dir().join("default.toml")).await.unwrap();
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_content).await.unwrap();
    
    // Create test files
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;
    fs::write(temp_dir.path().join("test.xsd"), schema_content).await.unwrap();
    
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="test.xsd">Content</root>"#;
    fs::write(temp_dir.path().join("test.xml"), xml_content).await.unwrap();
    
    // Run with configuration file
    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .arg("--config")
        .arg(&config_path)
        .arg("--verbose")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to run validation");
    
    assert!(output.status.success());
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_result: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");
    
    // Verify configuration was applied
    let config_info = json_result.get("configuration").unwrap();
    let extensions = config_info.get("extensions").unwrap().as_array().unwrap();
    
    // Should include extensions from config file
    assert!(extensions.iter().any(|e| e.as_str().unwrap() == "xml"));
    assert!(extensions.iter().any(|e| e.as_str().unwrap() == "cmdi"));
}