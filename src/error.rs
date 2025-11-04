use std::path::PathBuf;

use thiserror::Error;

/// Main application error type that encompasses all possible failure modes
#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

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
#[derive(Error, Debug)]
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
#[derive(Error, Debug)]
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
#[derive(Error, Debug)]
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
#[derive(Error, Debug)]
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
        let io_error = ValidationError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "File not found",
        ));
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
        assert!(schema_error.to_string().contains("Invalid XML syntax"));

        let validation_failed = ValidationError::ValidationFailed {
            file: PathBuf::from("/path/to/file.xml"),
            details: "Element 'test' is not valid".to_string(),
        };
        assert!(
            validation_failed
                .to_string()
                .contains("XML validation failed")
        );
        assert!(validation_failed.to_string().contains("file.xml"));
    }

    #[test]
    fn test_config_error_display() {
        let file_not_found = ConfigError::FileNotFound {
            path: PathBuf::from("/path/to/config.toml"),
        };
        assert!(
            file_not_found
                .to_string()
                .contains("Configuration file not found")
        );
        assert!(file_not_found.to_string().contains("config.toml"));

        let invalid_format = ConfigError::InvalidFormat {
            details: "Expected TOML format".to_string(),
        };
        assert!(
            invalid_format
                .to_string()
                .contains("Invalid configuration format")
        );

        let missing_field = ConfigError::MissingField {
            field: "cache_dir".to_string(),
        };
        assert!(
            missing_field
                .to_string()
                .contains("Missing required configuration field")
        );
        assert!(missing_field.to_string().contains("cache_dir"));

        let invalid_value = ConfigError::InvalidValue {
            field: "timeout".to_string(),
            value: "-1".to_string(),
            reason: "must be positive".to_string(),
        };
        assert!(
            invalid_value
                .to_string()
                .contains("Invalid configuration value")
        );
        assert!(invalid_value.to_string().contains("timeout"));
        assert!(invalid_value.to_string().contains("-1"));
        assert!(invalid_value.to_string().contains("must be positive"));
    }

    #[test]
    fn test_cache_error_display() {
        let init_failed = CacheError::InitializationFailed {
            details: "Permission denied".to_string(),
        };
        assert!(
            init_failed
                .to_string()
                .contains("Cache initialization failed")
        );

        let write_error = CacheError::WriteError {
            key: "schema_123".to_string(),
            details: "Disk full".to_string(),
        };
        assert!(write_error.to_string().contains("Cache write error"));
        assert!(write_error.to_string().contains("schema_123"));

        let corruption = CacheError::Corruption {
            key: "schema_456".to_string(),
            details: "Checksum mismatch".to_string(),
        };
        assert!(corruption.to_string().contains("Cache corruption detected"));
        assert!(corruption.to_string().contains("schema_456"));
    }

    #[test]
    fn test_network_error_display() {
        let timeout = NetworkError::Timeout {
            url: "http://example.com/schema.xsd".to_string(),
            timeout_ms: 5000,
        };
        assert!(timeout.to_string().contains("Connection timeout"));
        assert!(timeout.to_string().contains("5000ms"));

        let connection_refused = NetworkError::ConnectionRefused {
            url: "http://localhost:8080/schema.xsd".to_string(),
        };
        assert!(
            connection_refused
                .to_string()
                .contains("Connection refused")
        );

        let http_status = NetworkError::HttpStatus {
            status: 404,
            url: "http://example.com/missing.xsd".to_string(),
        };
        assert!(http_status.to_string().contains("HTTP status error"));
        assert!(http_status.to_string().contains("404"));
    }

    #[test]
    fn test_libxml2_error_display() {
        let parse_failed = LibXml2Error::SchemaParseFailed;
        assert!(parse_failed.to_string().contains("Schema parsing failed"));

        let validation_failed = LibXml2Error::ValidationFailed {
            code: -1,
            file: PathBuf::from("test.xml"),
        };
        assert!(
            validation_failed
                .to_string()
                .contains("File validation failed")
        );
        assert!(validation_failed.to_string().contains("-1"));

        let memory_alloc = LibXml2Error::MemoryAllocation;
        assert!(
            memory_alloc
                .to_string()
                .contains("Memory allocation failed")
        );
    }

    #[test]
    fn test_io_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied");
        let validation_error: ValidationError = io_error.into();

        match validation_error {
            ValidationError::Io(_) => (),
            _ => panic!("Expected ValidationError::Io"),
        }
    }

    #[test]
    fn test_config_error_conversion() {
        let config_error = ConfigError::MissingField {
            field: "test_field".to_string(),
        };
        let validation_error: ValidationError = config_error.into();

        match validation_error {
            ValidationError::Config(_) => (),
            _ => panic!("Expected ValidationError::Config"),
        }
    }

    #[test]
    fn test_cache_error_conversion() {
        let cache_error = CacheError::WriteError {
            key: "test_key".to_string(),
            details: "test details".to_string(),
        };
        let validation_error: ValidationError = cache_error.into();

        match validation_error {
            ValidationError::Cache(_) => (),
            _ => panic!("Expected ValidationError::Cache"),
        }
    }

    #[test]
    fn test_libxml2_error_conversion() {
        let libxml2_error = LibXml2Error::SchemaParseFailed;
        let validation_error: ValidationError = libxml2_error.into();

        match validation_error {
            ValidationError::LibXml2Internal { .. } => (),
            _ => panic!("Expected ValidationError::LibXml2Internal"),
        }
    }

    #[test]
    fn test_result_type_aliases() {
        // Test that Result type alias works
        let success: Result<String> = Ok("success".to_string());
        assert!(success.is_ok());

        let failure: Result<String> = Err(ValidationError::Config("test error".to_string()));
        assert!(failure.is_err());
    }

    #[test]
    fn test_config_result_type() {
        let success: ConfigResult<i32> = Ok(42);
        assert!(success.is_ok());

        let failure: ConfigResult<i32> = Err(ConfigError::MissingField {
            field: "test".to_string(),
        });
        assert!(failure.is_err());
    }

    #[test]
    fn test_error_source_chain() {
        use std::error::Error;

        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let validation_error = ValidationError::Io(io_error);

        // Test that the source chain is preserved
        assert!(validation_error.source().is_some());

        let source = validation_error.source().unwrap();
        assert_eq!(source.to_string(), "File not found");
    }

    #[test]
    fn test_debug_formatting() {
        let error = ValidationError::SchemaNotFound {
            url: "http://example.com/schema.xsd".to_string(),
        };

        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("SchemaNotFound"));
        assert!(debug_str.contains("http://example.com/schema.xsd"));
    }

    #[test]
    fn test_display_formatting() {
        let error = ValidationError::ValidationFailed {
            file: PathBuf::from("test.xml"),
            details: "Element validation failed".to_string(),
        };

        let display_str = error.to_string();
        assert!(display_str.contains("XML validation failed"));
        assert!(display_str.contains("test.xml"));
        assert!(display_str.contains("Element validation failed"));
    }
}
