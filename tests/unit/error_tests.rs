//! Error type tests
//!
//! Tests for error types and error reporting.
//! Note: Tests simplified due to architectural changes.

use std::path::PathBuf;
use validate_xml::ValidationError;

#[test]
fn test_io_error_conversion() {
    // Test that IO errors can be converted to ValidationError
    let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
    let validation_error: ValidationError = io_error.into();

    // Verify the error was created
    let error_msg = validation_error.to_string();
    assert!(!error_msg.is_empty());
}

#[test]
fn test_validation_error_types() {
    // Test creating various error types
    let timeout_error = ValidationError::Timeout {
        url: "http://example.com/schema.xsd".to_string(),
        timeout_seconds: 30,
    };

    let schema_error = ValidationError::SchemaNotFound {
        url: "http://example.com/missing.xsd".to_string(),
    };

    let config_error = ValidationError::Config("Invalid configuration".to_string());

    // Verify errors have messages
    assert!(!timeout_error.to_string().is_empty());
    assert!(!schema_error.to_string().is_empty());
    assert!(!config_error.to_string().is_empty());
}

#[test]
fn test_validation_failed_error() {
    let validation_error = ValidationError::ValidationFailed {
        file: PathBuf::from("test.xml"),
        details: "Missing required element".to_string(),
    };

    let message = validation_error.to_string();
    assert!(message.contains("test.xml"));
    assert!(message.contains("Missing required element"));
}

#[test]
fn test_cache_error() {
    let cache_error = ValidationError::Cache("Cache initialization failed".to_string());

    let message = cache_error.to_string();
    assert!(message.contains("Cache"));
    assert!(message.contains("initialization"));
}

#[test]
fn test_http_status_error() {
    let http_error = ValidationError::HttpStatus {
        url: "http://example.com/schema.xsd".to_string(),
        status: 404,
        message: "Not Found".to_string(),
    };

    let message = http_error.to_string();
    assert!(message.contains("404"));
    assert!(message.contains("http://example.com"));
}
