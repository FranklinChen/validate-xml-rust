use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

/// Main application error type that encompasses all possible failure modes
#[derive(Error, Debug, Clone)]
pub enum ValidationError {
    #[error("IO error: {0}")]
    Io(#[source] Arc<std::io::Error>),

    #[error("HTTP error: {0}")]
    Http(#[source] Arc<reqwest::Error>),

    #[error("HTTP status error: {status} for {url} - {message}")]
    HttpStatus {
        url: String,
        status: u16,
        message: String,
    },

    #[error("Request timeout: {url} after {timeout_seconds} seconds")]
    Timeout { url: String, timeout_seconds: u64 },

    #[error("Schema parsing error: {url} - {details}")]
    SchemaParsing { url: String, details: String },

    #[error("XML validation failed: {file} - {details}")]
    ValidationFailed { file: PathBuf, details: String },

    #[error("Schema not found: {url}")]
    SchemaNotFound { url: String },

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("LibXML2 internal error: {details}")]
    LibXml2Internal { details: String },

    #[error("File system traversal error: {path} - {reason}")]
    FileSystemTraversal { path: PathBuf, reason: String },

    #[error("Schema URL extraction failed: {file} - no schema location found")]
    SchemaUrlNotFound { file: PathBuf },

    #[error("Invalid file extension: expected {expected}, found {actual}")]
    InvalidFileExtension { expected: String, actual: String },

    #[error("Concurrent operation error: {details}")]
    Concurrency { details: String },

    #[error("Resource exhaustion: {resource} - {details}")]
    ResourceExhaustion { resource: String, details: String },
}

/// Configuration-specific error types
#[derive(Error, Debug, Clone)]
pub enum ConfigError {
    #[error("Configuration file not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("Invalid configuration format: {details}")]
    InvalidFormat { details: String },

    #[error("Missing required configuration field: {field}")]
    MissingField { field: String },

    #[error("Invalid configuration value: {field} = {value} - {reason}")]
    InvalidValue {
        field: String,
        value: String,
        reason: String,
    },

    #[error("Configuration merge conflict: {details}")]
    MergeConflict { details: String },
}

/// Cache-specific error types
#[derive(Error, Debug, Clone)]
pub enum CacheError {
    #[error("Cache initialization failed: {details}")]
    InitializationFailed { details: String },

    #[error("Cache write error: {key} - {details}")]
    WriteError { key: String, details: String },

    #[error("Cache read error: {key} - {details}")]
    ReadError { key: String, details: String },

    #[error("Cache corruption detected: {key} - {details}")]
    Corruption { key: String, details: String },

    #[error("Cache cleanup failed: {details}")]
    CleanupFailed { details: String },

    #[error("Cache TTL expired: {key}")]
    Expired { key: String },
}

/// Network-specific error types
#[derive(Error, Debug, Clone)]
pub enum NetworkError {
    #[error("Connection timeout: {url} after {timeout_ms}ms")]
    Timeout { url: String, timeout_ms: u64 },

    #[error("Connection refused: {url}")]
    ConnectionRefused { url: String },

    #[error("DNS resolution failed: {hostname}")]
    DnsResolution { hostname: String },

    #[error("HTTP status error: {status} for {url}")]
    HttpStatus { status: u16, url: String },

    #[error("Network unreachable: {url}")]
    NetworkUnreachable { url: String },

    #[error("SSL/TLS error: {url} - {details}")]
    TlsError { url: String, details: String },
}

/// LibXML2-specific error types
#[derive(Error, Debug, Clone)]
pub enum LibXml2Error {
    #[error("Schema parsing failed: null pointer returned")]
    SchemaParseFailed,

    #[error("Validation context creation failed")]
    ValidationContextFailed,

    #[error("Validation context creation failed")]
    ValidationContextCreationFailed,

    #[error("File validation failed with code {code}: {file}")]
    ValidationFailed { code: i32, file: PathBuf },

    #[error("Memory allocation failed in libxml2")]
    MemoryAllocation,

    #[error("Invalid XML structure: {details}")]
    InvalidXml { details: String },

    #[error("Schema validation internal error: {details}")]
    InternalError { details: String },
}

// Manual From implementations to handle Arc wrapping
impl From<std::io::Error> for ValidationError {
    fn from(err: std::io::Error) -> Self {
        ValidationError::Io(Arc::new(err))
    }
}

impl From<reqwest::Error> for ValidationError {
    fn from(err: reqwest::Error) -> Self {
        ValidationError::Http(Arc::new(err))
    }
}

// Error conversion implementations
impl From<ConfigError> for ValidationError {
    fn from(err: ConfigError) -> Self {
        ValidationError::Config(err.to_string())
    }
}

impl From<CacheError> for ValidationError {
    fn from(err: CacheError) -> Self {
        ValidationError::Cache(err.to_string())
    }
}

impl From<NetworkError> for ValidationError {
    fn from(err: NetworkError) -> Self {
        // Create a generic HTTP error by wrapping the network error details
        ValidationError::Cache(format!("Network error: {}", err))
    }
}

impl From<LibXml2Error> for ValidationError {
    fn from(err: LibXml2Error) -> Self {
        ValidationError::LibXml2Internal {
            details: err.to_string(),
        }
    }
}

/// Result type alias for convenience
pub type Result<T> = std::result::Result<T, ValidationError>;

/// Configuration result type alias
#[allow(dead_code)]
pub type ConfigResult<T> = std::result::Result<T, ConfigError>;

/// Cache result type alias
#[allow(dead_code)]
pub type CacheResult<T> = std::result::Result<T, CacheError>;

/// Network result type alias
#[allow(dead_code)]
pub type NetworkResult<T> = std::result::Result<T, NetworkError>;

/// LibXML2 result type alias
#[allow(dead_code)]
pub type LibXml2Result<T> = std::result::Result<T, LibXml2Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validation_error_display() {
        let io_error = ValidationError::Io(Arc::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "File not found",
        )));
        assert!(io_error.to_string().contains("IO error"));

        let schema_error = ValidationError::SchemaParsing {
            url: "http://example.com/schema.xsd".to_string(),
            details: "Invalid XML syntax".to_string(),
        };
        assert!(schema_error.to_string().contains("Schema parsing error"));
        assert!(
            schema_error
                .to_string()
                .contains("http://example.com/schema.xsd")
        );

        let validation_failed = ValidationError::ValidationFailed {
            file: PathBuf::from("/path/to/file.xml"),
            details: "Element 'test' is not valid".to_string(),
        };
        assert!(validation_failed.to_string().contains("file.xml"));
    }

    #[test]
    fn test_error_conversions() {
        // Test IO conversion
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied");
        let validation_error: ValidationError = io_error.into();
        assert!(matches!(validation_error, ValidationError::Io(_)));

        // Test Config conversion
        let config_error = ConfigError::MissingField {
            field: "test_field".to_string(),
        };
        let validation_error: ValidationError = config_error.into();
        assert!(matches!(validation_error, ValidationError::Config(_)));

        // Test LibXml2 conversion
        let libxml2_error = LibXml2Error::SchemaParseFailed;
        let validation_error: ValidationError = libxml2_error.into();
        assert!(matches!(
            validation_error,
            ValidationError::LibXml2Internal { .. }
        ));
    }

    #[test]
    fn test_error_source_chain() {
        use std::error::Error;

        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let validation_error = ValidationError::from(io_error);

        assert!(validation_error.source().is_some());
        assert_eq!(
            validation_error.source().unwrap().to_string(),
            "File not found"
        );
    }
}
