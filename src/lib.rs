//! # validate-xml Library
//!
//! A high-performance, async-first Rust library for validating XML files against XSD schemas
//! with built-in remote schema caching and concurrent processing.
//!
//! ## Features
//!
//! - **Async I/O**: Built on `tokio` for efficient concurrent operations
//! - **Schema Caching**: Two-tier caching (memory + disk) to minimize redundant downloads
//! - **Concurrent Validation**: Process thousands of XML files in parallel
//! - **Error Handling**: Comprehensive error types with context and recovery suggestions
//! - **JSON Output**: Machine-readable validation results for CI/CD integration
//! - **CLI Tool**: Full-featured command-line interface via `validate-xml` binary
//!
//! ## Architecture Overview
//!
//! The library is organized into specialized modules:
//!
//! - **`validator`**: Core XML validation logic against XSD schemas
//! - **`cache`**: Two-tier caching system (memory + disk-backed)
//! - **`schema_loader`**: Async HTTP schema downloading with error recovery
//! - **`file_discovery`**: Recursive file discovery with extension filtering
//! - **`output`**: Text and JSON formatting for validation results
//! - **`cli`**: Command-line argument parsing (via `clap` derive API)
//! - **`config`**: Configuration management for all subsystems
//! - **`error`**: Error types and handling strategies
//! - **`http_client`**: Async HTTP client with timeout and retry logic
//! - **`libxml2`**: Safe FFI wrappers for libxml2 library
//!
//! ## Quick Start
//!
//! ```ignore
//! use validate_xml::{Config, FileDiscovery, SchemaLoader, Validator};
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. Configure the validation system
//!     let config = Config::from_file("config.toml")?;
//!
//!     // 2. Discover XML files to validate
//!     let discovery = FileDiscovery::new(&config.file_config);
//!     let files = discovery.discover(&PathBuf::from("/path/to/xml/files")).await?;
//!
//!     // 3. Initialize schema loader with caching
//!     let schema_loader = SchemaLoader::new(config.cache_config);
//!
//!     // 4. Validate files concurrently
//!     let validator = Validator::new(schema_loader, config.validation_config);
//!     let results = validator.validate_files(files).await?;
//!
//!     // 5. Format and output results
//!     println!("{}", results.to_human_readable());
//!     Ok(())
//! }
//! ```
//!
//! ## Performance Characteristics
//!
//! - **Single File**: ~1-10ms depending on schema complexity
//! - **20,000 Files**: <30 seconds on modern hardware (8 cores) with schema caching
//! - **Memory Usage**: Bounded by cache configuration (default 100MB)
//! - **Concurrency**: Automatic utilization of all available CPU cores
//!
//! ## Configuration
//!
//! Configure behavior via `config.toml` or environment variables:
//!
//! ```toml
//! [cache]
//! directory = "./cache"
//! ttl_hours = 24
//! max_size_mb = 100
//! max_memory_entries = 1000
//! memory_ttl_seconds = 3600
//!
//! [validation]
//! require_schema = true
//! fail_on_warning = false
//!
//! [network]
//! timeout_seconds = 30
//! retry_attempts = 3
//! ```
//!
//! ## Testing
//!
//! Run the test suite:
//!
//! ```bash
//! cargo test --lib          # Library unit tests
//! cargo test --test '*'     # All integration tests
//! cargo test --doc          # Documentation tests
//! cargo bench               # Performance benchmarks
//! ```
//!
//! ## Error Handling
//!
//! The library uses a comprehensive error hierarchy:
//!
//! - `ValidationError`: XML/schema validation failures
//! - `ConfigError`: Configuration loading/parsing issues
//! - `CacheError`: Caching system failures
//! - `NetworkError`: HTTP/remote schema access failures
//! - `LibXml2Error`: libxml2 FFI errors
//!
//! All errors include context and suggestions for recovery.
//!
//! ## Constraints and Limitations
//!
//! - **Memory**: Schema cache is memory-resident; very large schemas may require tuning
//! - **Concurrency**: Bounded by available system resources; configure `max_memory_entries`
//! - **Schemas**: Only HTTP and local file URLs supported in MVP
//! - **XML Features**: Standard XML 1.0 with XSD schema validation
//!
//! ## CONSTITUTION COMPLIANCE
//!
//! This library adheres to the validate-xml constitution (v1.0.0):
//! - ✅ Async-First: All I/O uses `tokio` async/await
//! - ✅ Efficient Caching: Remote schemas cached once per URL per run
//! - ✅ Test-First: Comprehensive test coverage for all modules
//! - ✅ CLI-Driven: Primary interface is the `validate-xml` command-line tool
//! - ✅ Performance Excellence: Measurable performance targets and benchmarks
//!
//! See `.specify/memory/constitution.md` for the full governance document.

// Core modules
pub mod cache;
pub mod cli;
pub mod config;
pub mod error;
pub mod error_reporter;
pub mod file_discovery;
pub mod http_client;
pub mod libxml2;
pub mod output;
pub mod schema_loader;
pub mod validator;

// Re-export commonly used types for convenient access
// This forms the public API surface
pub use cache::*;
pub use cli::*;
pub use config::{CacheConfig, Config, ConfigManager};
pub use error::{CacheError, ConfigError, LibXml2Error, NetworkError, ValidationError};
pub use error_reporter::*;
pub use file_discovery::{DiscoveryStats, FileDiscovery};
pub use http_client::{AsyncHttpClient, HttpClientConfig};
pub use libxml2::{LibXml2Wrapper, ValidationResult, XmlSchemaPtr};
pub use output::*;
pub use schema_loader::{
    SchemaExtractor, SchemaLoader, SchemaReference, SchemaSourceType, extract_schema_url_async,
};
pub use validator::*;
