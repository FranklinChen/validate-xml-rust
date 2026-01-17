//! # validate-xml Library
//!
//! A high-performance, async-first Rust library for validating XML files against XSD schemas
//! with built-in remote schema caching and concurrent processing.

pub mod cache;
pub mod cli;
pub mod error;
pub mod file_discovery;
pub mod http_client;
pub mod libxml2;
pub mod output;
pub mod schema_loader;
pub mod validator;

pub use cache::{
    CacheConfig, CacheMetadata, CacheStats, CachedSchema, CleanupStats, ComprehensiveCacheStats,
    DiskCache, MemoryCache, MemoryCacheStats, ParsedSchemaCache, SchemaCache,
};
pub use cli::{Cli, Config, VerbosityLevel};
pub use error::ValidationError;
pub use file_discovery::{DiscoveryStats, FileDiscovery};
pub use http_client::{AsyncHttpClient, HttpClientConfig};
pub use libxml2::{LibXml2Wrapper, ValidationResult, XmlSchemaPtr};
pub use output::Output;
pub use schema_loader::{
    SchemaExtractor, SchemaLoader, SchemaReference, SchemaSourceType, extract_schema_url_async,
};
pub use validator::{
    FileValidationResult, PerformanceMetrics, ProgressCallback, ValidationConfig, ValidationEngine,
    ValidationPhase, ValidationProgress, ValidationResults, ValidationStatus,
};
