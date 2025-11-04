//! Integration tests for enhanced output and reporting system

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use serde_json;

use validate_xml::cli::OutputFormat;
use validate_xml::error_reporter::VerbosityLevel;
use validate_xml::output::*;
use validate_xml::validator::{ValidationResults, FileValidationResult, ValidationStatus, PerformanceMetrics, SchemaCacheStats};

/// Create a temporary directory with test XML files and schemas
fn create_test_environment() -> (TempDir, Vec<PathBuf>) {
    let temp_dir = TempDir::new().unwrap();
    let xml_dir = temp_dir.path().join("xml");
    let schema_dir = temp_dir.path().join("schemas");
    
    fs::create_dir_all(&xml_dir).unwrap();
    fs::create_dir_all(&schema_dir).unwrap();
    
    // Create a simple XSD schema
    let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="http://example.com/test"
           xmlns:tns="http://example.com/test"
           elementFormDefault="qualified">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="item" type="xs:string" maxOccurs="unbounded"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;
    
    let schema_path = schema_dir.join("test.xsd");
    fs::write(&schema_path, schema_content).unwrap();
    
    // Create valid XML file
    let valid_xml = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="http://example.com/test"
      xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/test {}">
    <item>Test Item 1</item>
    <item>Test Item 2</item>
</root>"#, schema_path.to_string_lossy());
    
    let valid_xml_path = xml_dir.join("valid.xml");
    fs::write(&valid_xml_path, valid_xml).unwrap();
    
    // Create invalid XML file
    let invalid_xml = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns="http://example.com/test"
      xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/test {}">
    <invalid_element>This should not be here</invalid_element>
</root>"#, schema_path.to_string_lossy());
    
    let invalid_xml_path = xml_dir.join("invalid.xml");
    fs::write(&invalid_xml_path, invalid_xml).unwrap();
    
    // Create XML file without schema reference
    let no_schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <item>No schema reference</item>
</root>"#;
    
    let no_schema_xml_path = xml_dir.join("no_schema.xml");
    fs::write(&no_schema_xml_path, no_schema_xml).unwrap();
    
    let xml_files = vec![valid_xml_path, invalid_xml_path, no_schema_xml_path];
    
    (temp_dir, xml_files)
}

/// Create test validation results for integration testing
fn create_integration_test_results() -> ValidationResults {
    let file_results = vec![
        FileValidationResult {
            path: PathBuf::from("valid.xml"),
            status: ValidationStatus::Valid,
            schema_url: Some("file:///tmp/test.xsd".to_string()),
            duration: Duration::from_millis(120),
            error_details: Vec::new(),
        },
        FileValidationResult {
            path: PathBuf::from("invalid.xml"),
            status: ValidationStatus::Invalid { error_count: 1 },
            schema_url: Some("file:///tmp/test.xsd".to_string()),
            duration: Duration::from_millis(180),
            error_details: vec!["Element 'invalid_element' is not allowed".to_string()],
        },
        FileValidationResult {
            path: PathBuf::from("no_schema.xml"),
            status: ValidationStatus::Skipped { reason: "No schema URL found in XML file".to_string() },
            schema_url: None,
            duration: Duration::from_millis(30),
            error_details: vec!["No schema URL found in XML file".to_string()],
        },
    ];
    
    let performance_metrics = PerformanceMetrics {
        total_duration: Duration::from_millis(450),
        discovery_duration: Duration::from_millis(80),
        schema_loading_duration: Duration::from_millis(40),
        validation_duration: Duration::from_millis(330),
        average_time_per_file: Duration::from_millis(110),
        throughput_files_per_second: 6.67,
        peak_memory_mb: 32,
        cache_hit_rate: 66.7,
        concurrent_validations: 2,
        schema_cache_stats: SchemaCacheStats {
            hits: 2,
            misses: 1,
            schemas_loaded: 1,
            cache_size_bytes: 2048,
        },
    };
    
    ValidationResults::with_metrics(file_results, performance_metrics)
}

#[test]
fn test_end_to_end_human_output() {
    let results = create_integration_test_results();
    let mut output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Normal);
    
    // Capture output to a buffer
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    output_writer = output_writer.with_writer(writer);
    
    // Write results
    output_writer.write_results(&results).unwrap();
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Verify human-readable output contains expected elements
    assert!(output.contains("Validation Summary:"));
    assert!(output.contains("Total files: 3"));
    assert!(output.contains("Valid: 1"));
    assert!(output.contains("Invalid: 1"));
    assert!(output.contains("Skipped: 1"));
    assert!(output.contains("Success rate: 33.3%"));
    assert!(output.contains("Duration:"));
    assert!(output.contains("Performance Metrics:"));
    assert!(output.contains("Discovery time:"));
    assert!(output.contains("Validation time:"));
    assert!(output.contains("Throughput:"));
}

#[test]
fn test_end_to_end_json_output() {
    let results = create_integration_test_results();
    let mut output_writer = OutputWriter::new(OutputFormat::Json, VerbosityLevel::Normal);
    
    // Capture output to a buffer
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    output_writer = output_writer.with_writer(writer);
    
    // Write results
    output_writer.write_results(&results).unwrap();
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Parse and verify JSON structure
    let json_value: serde_json::Value = serde_json::from_str(&output).unwrap();
    
    // Verify top-level structure
    assert!(json_value["summary"].is_object());
    assert!(json_value["files"].is_array());
    assert!(json_value["schemas"].is_array());
    assert!(json_value["performance"].is_object());
    assert!(json_value["timestamp"].is_string());
    
    // Verify summary
    let summary = &json_value["summary"];
    assert_eq!(summary["total_files"], 3);
    assert_eq!(summary["valid_files"], 1);
    assert_eq!(summary["invalid_files"], 1);
    assert_eq!(summary["skipped_files"], 1);
    assert!((summary["success_rate"].as_f64().unwrap() - 33.333333333333336).abs() < 0.001);
    
    // Verify files array
    let files = json_value["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    
    // Check valid file
    assert_eq!(files[0]["status"], "valid");
    assert_eq!(files[0]["path"], "valid.xml");
    assert_eq!(files[0]["duration_ms"], 120);
    
    // Check invalid file
    assert_eq!(files[1]["status"], "invalid");
    assert_eq!(files[1]["error_count"], 1);
    assert_eq!(files[1]["error_details"].as_array().unwrap().len(), 1);
    
    // Check skipped file
    assert_eq!(files[2]["status"], "skipped");
    assert!(files[2]["error_count"].is_null());
    
    // Verify performance metrics
    let performance = &json_value["performance"];
    assert_eq!(performance["total_duration_ms"], 450);
    assert_eq!(performance["concurrent_validations"], 2);
    assert!((performance["throughput_files_per_second"].as_f64().unwrap() - 6.67).abs() < 0.01);
    
    // Verify cache stats
    let cache_stats = &performance["cache_stats"];
    assert_eq!(cache_stats["hits"], 2);
    assert_eq!(cache_stats["misses"], 1);
    assert_eq!(cache_stats["schemas_loaded"], 1);
}

#[test]
fn test_end_to_end_summary_output() {
    let results = create_integration_test_results();
    let mut output_writer = OutputWriter::new(OutputFormat::Summary, VerbosityLevel::Normal);
    
    // Capture output to a buffer
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    output_writer = output_writer.with_writer(writer);
    
    // Write results
    output_writer.write_results(&results).unwrap();
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Verify compact summary format
    assert!(output.contains("1/3 valid"));
    assert!(output.contains("33.3%"));
    assert!(output.contains("0.45s"));
}

#[test]
fn test_progress_indicator_integration() {
    let formatter = Box::new(HumanFormatter::new(VerbosityLevel::Normal));
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    
    let mut progress_indicator = ProgressIndicator::new(formatter)
        .with_writer(writer)
        .with_update_interval(Duration::from_millis(0)); // No throttling for tests
    
    // Simulate progress updates during validation
    let files = vec![
        PathBuf::from("file1.xml"),
        PathBuf::from("file2.xml"),
        PathBuf::from("file3.xml"),
        PathBuf::from("file4.xml"),
        PathBuf::from("file5.xml"),
    ];
    
    for (i, file) in files.iter().enumerate() {
        progress_indicator.update(i, files.len(), Some(file)).unwrap();
    }
    
    // Final update
    progress_indicator.update(files.len(), files.len(), None).unwrap();
    progress_indicator.finish().unwrap();
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Verify progress updates were written
    assert!(output.contains("0/5"));
    assert!(output.contains("1/5"));
    assert!(output.contains("5/5"));
    assert!(output.contains("100%"));
    assert!(output.contains("file1.xml"));
    assert!(output.contains("file5.xml"));
}

#[test]
fn test_verbosity_level_integration() {
    let results = create_integration_test_results();
    
    // Test quiet mode
    let mut quiet_buffer = Vec::new();
    let quiet_writer = Box::new(std::io::Cursor::new(&mut quiet_buffer));
    let mut quiet_output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Quiet)
        .with_writer(quiet_writer);
    
    quiet_output_writer.write_results(&results).unwrap();
    let quiet_output = String::from_utf8(quiet_buffer).unwrap();
    
    // Quiet mode should only show errors
    assert!(quiet_output.contains("Errors: 0 Invalid: 1"));
    assert!(!quiet_output.contains("Performance Metrics"));
    
    // Test verbose mode
    let mut verbose_buffer = Vec::new();
    let verbose_writer = Box::new(std::io::Cursor::new(&mut verbose_buffer));
    let mut verbose_output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Verbose)
        .with_writer(verbose_writer);
    
    verbose_output_writer.write_results(&results).unwrap();
    let verbose_output = String::from_utf8(verbose_buffer).unwrap();
    
    // Verbose mode should show detailed information
    assert!(verbose_output.contains("Validation Summary:"));
    assert!(verbose_output.contains("Performance Metrics:"));
    assert!(verbose_output.contains("Discovery time:"));
    assert!(verbose_output.contains("Validation time:"));
    assert!(verbose_output.contains("Throughput:"));
    
    // Test debug mode
    let mut debug_buffer = Vec::new();
    let debug_writer = Box::new(std::io::Cursor::new(&mut debug_buffer));
    let mut debug_output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Debug)
        .with_writer(debug_writer);
    
    debug_output_writer.write_results(&results).unwrap();
    let debug_output = String::from_utf8(debug_buffer).unwrap();
    
    // Debug mode should show everything including debug info
    assert!(debug_output.contains("Validation Summary:"));
    assert!(debug_output.contains("Performance Metrics:"));
    assert!(debug_output.contains("Peak memory:"));
    assert!(debug_output.contains("Cache hit rate:"));
    assert!(debug_output.contains("Debug Information:"));
}

#[test]
fn test_individual_file_result_output() {
    let results = create_integration_test_results();
    
    // Test writing individual file results
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    let mut output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Normal)
        .with_writer(writer);
    
    // Write each file result individually
    for file_result in &results.file_results {
        output_writer.write_file_result(file_result).unwrap();
    }
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Verify individual file results
    assert!(output.contains("VALID"));
    assert!(output.contains("valid.xml"));
    assert!(output.contains("INVALID"));
    assert!(output.contains("invalid.xml"));
    assert!(output.contains("SKIPPED"));
    assert!(output.contains("no_schema.xml"));
}

#[test]
fn test_summary_only_output() {
    let results = create_integration_test_results();
    
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    let mut output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Normal)
        .with_writer(writer);
    
    // Write only the summary
    output_writer.write_summary(&results).unwrap();
    
    let output = String::from_utf8(buffer).unwrap();
    
    // Verify summary output
    assert!(output.contains("Validation Summary:"));
    assert!(output.contains("Total files: 3"));
    assert!(output.contains("Success rate: 33.3%"));
    assert!(output.contains("Performance Metrics:"));
}

#[test]
fn test_json_progress_output() {
    let formatter = JsonFormatter::new(false);
    
    let progress_output = formatter.format_progress(
        3, 
        10, 
        Some(&PathBuf::from("current_file.xml"))
    ).unwrap();
    
    // Parse JSON progress
    let json_value: serde_json::Value = serde_json::from_str(&progress_output).unwrap();
    
    assert_eq!(json_value["current"], 3);
    assert_eq!(json_value["total"], 10);
    assert_eq!(json_value["percentage"], 30.0);
    assert_eq!(json_value["current_file"], "current_file.xml");
    assert!(json_value["timestamp"].is_string());
}

#[test]
fn test_output_error_handling() {
    // Test write error handling by using a writer that always fails
    struct FailingWriter;
    
    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Write failed"))
        }
        
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Flush failed"))
        }
    }
    
    let results = create_integration_test_results();
    let failing_writer = Box::new(FailingWriter);
    let mut output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Normal)
        .with_writer(failing_writer);
    
    // This should return an error
    let result = output_writer.write_results(&results);
    assert!(result.is_err());
    
    match result.unwrap_err() {
        OutputError::WriteError(_) => {}, // Expected
        _ => panic!("Expected WriteError"),
    }
}

#[test]
fn test_factory_integration() {
    // Test that factory methods create working components
    let results = create_integration_test_results();
    
    // Test formatter factory
    let formatter = OutputFormatterFactory::create_formatter(
        OutputFormat::Json, 
        VerbosityLevel::Normal
    );
    
    let json_output = formatter.format_results(&results).unwrap();
    let _: serde_json::Value = serde_json::from_str(&json_output).unwrap(); // Should parse
    
    // Test output writer factory
    let mut buffer = Vec::new();
    let writer = Box::new(std::io::Cursor::new(&mut buffer));
    let mut output_writer = OutputFormatterFactory::create_output_writer(
        OutputFormat::Summary, 
        VerbosityLevel::Normal
    ).with_writer(writer);
    
    output_writer.write_results(&results).unwrap();
    let output = String::from_utf8(buffer).unwrap();
    assert!(output.contains("1/3 valid"));
}

#[test]
fn test_real_world_scenario_simulation() {
    // Simulate a real-world validation scenario with mixed results
    let file_results = vec![
        // Batch of valid files
        FileValidationResult {
            path: PathBuf::from("documents/doc1.xml"),
            status: ValidationStatus::Valid,
            schema_url: Some("https://example.com/schemas/document.xsd".to_string()),
            duration: Duration::from_millis(95),
            error_details: Vec::new(),
        },
        FileValidationResult {
            path: PathBuf::from("documents/doc2.xml"),
            status: ValidationStatus::Valid,
            schema_url: Some("https://example.com/schemas/document.xsd".to_string()),
            duration: Duration::from_millis(87),
            error_details: Vec::new(),
        },
        // Invalid file with multiple errors
        FileValidationResult {
            path: PathBuf::from("documents/doc3.xml"),
            status: ValidationStatus::Invalid { error_count: 3 },
            schema_url: Some("https://example.com/schemas/document.xsd".to_string()),
            duration: Duration::from_millis(145),
            error_details: vec![
                "Element 'title' is missing".to_string(),
                "Attribute 'id' is required".to_string(),
                "Invalid date format in 'created' element".to_string(),
            ],
        },
        // Network error for remote schema
        FileValidationResult {
            path: PathBuf::from("remote/remote_doc.xml"),
            status: ValidationStatus::Error { 
                message: "Failed to download schema: Connection timeout".to_string() 
            },
            schema_url: Some("https://remote.example.com/schema.xsd".to_string()),
            duration: Duration::from_millis(5000), // Long timeout
            error_details: vec!["Failed to download schema: Connection timeout".to_string()],
        },
        // Files without schema references
        FileValidationResult {
            path: PathBuf::from("legacy/old_format.xml"),
            status: ValidationStatus::Skipped { 
                reason: "No schema URL found in XML file".to_string() 
            },
            schema_url: None,
            duration: Duration::from_millis(15),
            error_details: vec!["No schema URL found in XML file".to_string()],
        },
    ];
    
    let performance_metrics = PerformanceMetrics {
        total_duration: Duration::from_millis(5342),
        discovery_duration: Duration::from_millis(120),
        schema_loading_duration: Duration::from_millis(890),
        validation_duration: Duration::from_millis(4332),
        average_time_per_file: Duration::from_millis(1068),
        throughput_files_per_second: 0.94,
        peak_memory_mb: 128,
        cache_hit_rate: 40.0, // Some cache hits, some misses
        concurrent_validations: 4,
        schema_cache_stats: SchemaCacheStats {
            hits: 2,
            misses: 3,
            schemas_loaded: 2,
            cache_size_bytes: 8192,
        },
    };
    
    let results = ValidationResults::with_metrics(file_results, performance_metrics);
    
    // Test human output for this scenario
    let mut human_buffer = Vec::new();
    let human_writer = Box::new(std::io::Cursor::new(&mut human_buffer));
    let mut human_output_writer = OutputWriter::new(OutputFormat::Human, VerbosityLevel::Verbose)
        .with_writer(human_writer);
    
    human_output_writer.write_results(&results).unwrap();
    let human_output = String::from_utf8(human_buffer).unwrap();
    
    // Verify comprehensive output
    assert!(human_output.contains("Total files: 5"));
    assert!(human_output.contains("Valid: 2"));
    assert!(human_output.contains("Invalid: 1"));
    assert!(human_output.contains("Errors: 1"));
    assert!(human_output.contains("Skipped: 1"));
    assert!(human_output.contains("Success rate: 40.0%"));
    assert!(human_output.contains("5.34s") || human_output.contains("5342ms"));
    assert!(human_output.contains("Peak memory: 128 MB"));
    assert!(human_output.contains("Cache hit rate: 40.0%"));
    
    // Test JSON output for machine processing
    let mut json_buffer = Vec::new();
    let json_writer = Box::new(std::io::Cursor::new(&mut json_buffer));
    let mut json_output_writer = OutputWriter::new(OutputFormat::Json, VerbosityLevel::Normal)
        .with_writer(json_writer);
    
    json_output_writer.write_results(&results).unwrap();
    let json_output = String::from_utf8(json_buffer).unwrap();
    
    let json_value: serde_json::Value = serde_json::from_str(&json_output).unwrap();
    assert_eq!(json_value["summary"]["total_files"], 5);
    assert_eq!(json_value["summary"]["success_rate"], 40.0);
    assert_eq!(json_value["files"].as_array().unwrap().len(), 5);
    
    // Verify error details are preserved in JSON
    let files = json_value["files"].as_array().unwrap();
    let invalid_file = &files[2]; // doc3.xml
    assert_eq!(invalid_file["status"], "invalid");
    assert_eq!(invalid_file["error_count"], 3);
    assert_eq!(invalid_file["error_details"].as_array().unwrap().len(), 3);
}