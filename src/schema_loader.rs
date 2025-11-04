use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::cache::{CachedSchema, SchemaCache};
use crate::error::{Result, ValidationError};
use crate::http_client::AsyncHttpClient;

/// Cached regex for xsi:schemaLocation extraction
static SCHEMA_LOCATION_REGEX: OnceLock<Regex> = OnceLock::new();

/// Cached regex for xsi:noNamespaceSchemaLocation extraction
static NO_NAMESPACE_REGEX: OnceLock<Regex> = OnceLock::new();

/// Get or initialize the schema location regex
fn get_schema_location_regex() -> &'static Regex {
    SCHEMA_LOCATION_REGEX.get_or_init(|| {
        Regex::new(r#"xsi:schemaLocation="\S+\s+(.+?)""#)
            .expect("Failed to compile schemaLocation regex")
    })
}

/// Get or initialize the no namespace schema location regex
fn get_no_namespace_regex() -> &'static Regex {
    NO_NAMESPACE_REGEX.get_or_init(|| {
        Regex::new(r#"xsi:noNamespaceSchemaLocation="(.+?)""#)
            .expect("Failed to compile noNamespaceSchemaLocation regex")
    })
}

/// Schema URL extraction result
#[derive(Debug, Clone)]
pub struct SchemaReference {
    pub url: String,
    pub source_type: SchemaSourceType,
}

/// Type of schema source
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaSourceType {
    Local(PathBuf),
    Remote(String),
}

/// Schema extraction engine for async XML parsing
///
/// Uses cached regexes from OnceLock for efficient schema URL extraction.
/// Regexes are compiled once on first use and reused for all subsequent operations.
pub struct SchemaExtractor;

impl SchemaExtractor {
    /// Create a new schema extractor
    ///
    /// This is a zero-cost operation since the actual regex compilation
    /// is deferred to first use via OnceLock caching.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Extract schema URLs from XML file using async I/O
    pub async fn extract_schema_urls(&self, file_path: &Path) -> Result<Vec<SchemaReference>> {
        let file = File::open(file_path).await.map_err(ValidationError::Io)?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut schema_refs = Vec::new();

        // Get cached regexes (lazily initialized on first use)
        let schema_location_regex = get_schema_location_regex();
        let no_namespace_regex = get_no_namespace_regex();

        while let Some(line) = lines.next_line().await.map_err(ValidationError::Io)? {
            // Check for xsi:schemaLocation
            if let Some(caps) = schema_location_regex.captures(&line) {
                let url = caps[1].to_owned();
                let source_type = Self::determine_source_type(&url, file_path);
                schema_refs.push(SchemaReference { url, source_type });
            }

            // Check for xsi:noNamespaceSchemaLocation
            if let Some(caps) = no_namespace_regex.captures(&line) {
                let url = caps[1].to_owned();
                let source_type = Self::determine_source_type(&url, file_path);
                schema_refs.push(SchemaReference { url, source_type });
            }

            // Stop after processing the root element (optimization)
            if line.trim_start().starts_with("</") {
                break;
            }
        }

        if schema_refs.is_empty() {
            return Err(ValidationError::SchemaUrlNotFound {
                file: file_path.to_path_buf(),
            });
        }

        Ok(schema_refs)
    }

    /// Extract schema URLs from an async reader
    pub async fn extract_from_reader<R>(&self, reader: R) -> Result<Vec<SchemaReference>>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let buf_reader = BufReader::new(reader);
        let mut lines = buf_reader.lines();
        let mut schema_refs = Vec::new();

        // Get cached regexes (lazily initialized on first use)
        let schema_location_regex = get_schema_location_regex();
        let no_namespace_regex = get_no_namespace_regex();

        while let Some(line) = lines.next_line().await.map_err(ValidationError::Io)? {
            // Check for xsi:schemaLocation
            if let Some(caps) = schema_location_regex.captures(&line) {
                let url = caps[1].to_owned();
                let source_type = Self::determine_source_type(&url, Path::new(""));
                schema_refs.push(SchemaReference { url, source_type });
            }

            // Check for xsi:noNamespaceSchemaLocation
            if let Some(caps) = no_namespace_regex.captures(&line) {
                let url = caps[1].to_owned();
                let source_type = Self::determine_source_type(&url, Path::new(""));
                schema_refs.push(SchemaReference { url, source_type });
            }

            // Stop after processing the root element (optimization)
            if line.trim_start().starts_with("</") {
                break;
            }
        }

        Ok(schema_refs)
    }

    /// Determine if a schema URL is local or remote
    fn determine_source_type(url: &str, xml_file_path: &Path) -> SchemaSourceType {
        if url.starts_with("http://") || url.starts_with("https://") {
            SchemaSourceType::Remote(url.to_string())
        } else {
            // Resolve relative paths relative to the XML file's directory
            let schema_path = if url.starts_with('/') {
                PathBuf::from(url)
            } else {
                xml_file_path.parent().unwrap_or(Path::new(".")).join(url)
            };
            SchemaSourceType::Local(schema_path)
        }
    }
}

/// Unified async schema loader that handles both local and remote schemas
pub struct SchemaLoader {
    extractor: SchemaExtractor,
    cache: Arc<SchemaCache>,
    http_client: AsyncHttpClient,
}

impl SchemaLoader {
    pub fn new(cache: Arc<SchemaCache>, http_client: AsyncHttpClient) -> Result<Self> {
        let extractor = SchemaExtractor::new()?;

        Ok(Self {
            extractor,
            cache,
            http_client,
        })
    }

    /// Load schema for an XML file, handling both local and remote schemas
    pub async fn load_schema_for_file(&self, xml_file_path: &Path) -> Result<Arc<CachedSchema>> {
        // Extract schema references from the XML file
        let schema_refs = self.extractor.extract_schema_urls(xml_file_path).await?;

        // For now, use the first schema reference found
        // TODO: In the future, we might want to handle multiple schemas
        let schema_ref =
            schema_refs
                .into_iter()
                .next()
                .ok_or_else(|| ValidationError::SchemaUrlNotFound {
                    file: xml_file_path.to_path_buf(),
                })?;

        self.load_schema(&schema_ref).await
    }

    /// Load a schema by reference (local or remote)
    pub async fn load_schema(&self, schema_ref: &SchemaReference) -> Result<Arc<CachedSchema>> {
        match &schema_ref.source_type {
            SchemaSourceType::Local(path) => self.load_local_schema(path).await,
            SchemaSourceType::Remote(url) => self.load_remote_schema(url).await,
        }
    }

    /// Load a local schema file
    pub async fn load_local_schema(&self, schema_path: &Path) -> Result<Arc<CachedSchema>> {
        // For local files, we use the file path as the cache key
        let cache_key = format!("local:{}", schema_path.display());

        // Check cache first
        if let Some(cached_schema) = self.cache.get(&cache_key).await? {
            return Ok(cached_schema);
        }

        // Read the local schema file
        let schema_data = tokio::fs::read(schema_path)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ValidationError::SchemaNotFound {
                    url: schema_path.display().to_string(),
                },
                _ => ValidationError::Io(e),
            })?;

        // Validate the schema content
        self.validate_schema_content(&schema_data, &schema_path.display().to_string())?;

        // Cache the schema (local schemas don't have ETags or Last-Modified headers)
        self.cache.set(&cache_key, schema_data, None, None).await?;

        // Return the cached schema
        self.cache.get(&cache_key).await?.ok_or_else(|| {
            ValidationError::Cache("Failed to retrieve just-cached local schema".to_string())
        })
    }

    /// Load a remote schema with caching
    pub async fn load_remote_schema(&self, url: &str) -> Result<Arc<CachedSchema>> {
        // Check cache first
        if let Some(cached_schema) = self.cache.get(url).await? {
            return Ok(cached_schema);
        }

        // Download the schema
        let schema_data = self.http_client.download_schema(url).await?;

        // Validate the schema content
        self.validate_schema_content(&schema_data, url)?;

        // Cache the schema (TODO: extract ETags and Last-Modified from HTTP response)
        self.cache.set(url, schema_data, None, None).await?;

        // Return the cached schema
        self.cache.get(url).await?.ok_or_else(|| {
            ValidationError::Cache("Failed to retrieve just-cached remote schema".to_string())
        })
    }

    /// Validate that the schema content is well-formed XML
    fn validate_schema_content(&self, data: &[u8], source: &str) -> Result<()> {
        // Basic validation: check if it's valid UTF-8 and contains XML-like content
        let content = std::str::from_utf8(data).map_err(|_| ValidationError::SchemaParsing {
            url: source.to_string(),
            details: "Schema content is not valid UTF-8".to_string(),
        })?;

        // Check for basic XML structure
        if !content.trim_start().starts_with("<?xml") && !content.trim_start().starts_with("<") {
            return Err(ValidationError::SchemaParsing {
                url: source.to_string(),
                details: "Schema content does not appear to be XML".to_string(),
            });
        }

        // Check for schema-specific elements
        if !content.contains("<xs:schema")
            && !content.contains("<xsd:schema")
            && !content.contains("<schema")
        {
            return Err(ValidationError::SchemaParsing {
                url: source.to_string(),
                details: "Content does not appear to be an XML Schema (XSD)".to_string(),
            });
        }

        Ok(())
    }

    /// Get the schema extractor for direct use
    pub fn extractor(&self) -> &SchemaExtractor {
        &self.extractor
    }

    /// Get the cache for direct access
    pub fn cache(&self) -> &Arc<SchemaCache> {
        &self.cache
    }

    /// Get the HTTP client for direct access
    pub fn http_client(&self) -> &AsyncHttpClient {
        &self.http_client
    }
}

/// Convenience function for extracting schema URL from file (backward compatibility)
pub async fn extract_schema_url_async(path: &Path) -> Result<String> {
    let extractor = SchemaExtractor::new()?;

    let schema_refs = extractor.extract_schema_urls(path).await?;
    let first_ref =
        schema_refs
            .into_iter()
            .next()
            .ok_or_else(|| ValidationError::SchemaUrlNotFound {
                file: path.to_path_buf(),
            })?;

    Ok(first_ref.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir};

    use crate::config::CacheConfig;
    use crate::http_client::HttpClientConfig;
    use std::io::Write;

    fn create_test_cache() -> (Arc<SchemaCache>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            directory: temp_dir.path().to_path_buf(),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        let cache = Arc::new(SchemaCache::new(config));
        (cache, temp_dir)
    }

    fn create_test_http_client() -> AsyncHttpClient {
        let config = HttpClientConfig::default();
        AsyncHttpClient::new(config).unwrap()
    }

    #[tokio::test]
    async fn test_schema_extractor_creation() {
        let extractor = SchemaExtractor::new();
        assert!(extractor.is_ok());
    }

    #[tokio::test]
    async fn test_extract_schema_location() {
        let extractor = SchemaExtractor::new().unwrap();

        // Create a temporary XML file with schema location
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            temp_file,
            r#"<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
        )
        .unwrap();
        writeln!(
            temp_file,
            r#"      xsi:schemaLocation="http://example.com/ns http://example.com/schema.xsd">"#
        )
        .unwrap();
        writeln!(temp_file, r#"  <element>content</element>"#).unwrap();
        writeln!(temp_file, r#"</root>"#).unwrap();
        temp_file.flush().unwrap();

        let schema_refs = extractor
            .extract_schema_urls(temp_file.path())
            .await
            .unwrap();
        assert_eq!(schema_refs.len(), 1);
        assert_eq!(schema_refs[0].url, "http://example.com/schema.xsd");
        assert_eq!(
            schema_refs[0].source_type,
            SchemaSourceType::Remote("http://example.com/schema.xsd".to_string())
        );
    }

    #[tokio::test]
    async fn test_extract_no_namespace_schema_location() {
        let extractor = SchemaExtractor::new().unwrap();

        // Create a temporary XML file with noNamespaceSchemaLocation
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            temp_file,
            r#"<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
        )
        .unwrap();
        writeln!(
            temp_file,
            r#"      xsi:noNamespaceSchemaLocation="schema.xsd">"#
        )
        .unwrap();
        writeln!(temp_file, r#"  <element>content</element>"#).unwrap();
        writeln!(temp_file, r#"</root>"#).unwrap();
        temp_file.flush().unwrap();

        let schema_refs = extractor
            .extract_schema_urls(temp_file.path())
            .await
            .unwrap();
        assert_eq!(schema_refs.len(), 1);
        assert_eq!(schema_refs[0].url, "schema.xsd");

        // Should be local since it's a relative path
        match &schema_refs[0].source_type {
            SchemaSourceType::Local(path) => {
                assert!(path.to_string_lossy().ends_with("schema.xsd"));
            }
            _ => panic!("Expected local schema source type"),
        }
    }

    #[tokio::test]
    async fn test_extract_local_absolute_path() {
        let extractor = SchemaExtractor::new().unwrap();

        // Create a temporary XML file with absolute local path
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            temp_file,
            r#"<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
        )
        .unwrap();
        writeln!(
            temp_file,
            r#"      xsi:schemaLocation="http://example.com/ns /absolute/path/schema.xsd">"#
        )
        .unwrap();
        writeln!(temp_file, r#"  <element>content</element>"#).unwrap();
        writeln!(temp_file, r#"</root>"#).unwrap();
        temp_file.flush().unwrap();

        let schema_refs = extractor
            .extract_schema_urls(temp_file.path())
            .await
            .unwrap();
        assert_eq!(schema_refs.len(), 1);
        assert_eq!(schema_refs[0].url, "/absolute/path/schema.xsd");
        assert_eq!(
            schema_refs[0].source_type,
            SchemaSourceType::Local(PathBuf::from("/absolute/path/schema.xsd"))
        );
    }

    #[tokio::test]
    async fn test_extract_no_schema_found() {
        let extractor = SchemaExtractor::new().unwrap();

        // Create a temporary XML file without schema location
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(temp_file, r#"<root>"#).unwrap();
        writeln!(temp_file, r#"  <element>content</element>"#).unwrap();
        writeln!(temp_file, r#"</root>"#).unwrap();
        temp_file.flush().unwrap();

        let result = extractor.extract_schema_urls(temp_file.path()).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidationError::SchemaUrlNotFound { .. } => (),
            _ => panic!("Expected SchemaUrlNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_schema_loader_creation() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();

        let loader = SchemaLoader::new(cache, http_client);
        assert!(loader.is_ok());
    }

    #[tokio::test]
    async fn test_load_local_schema() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        // Create a temporary schema file
        let mut schema_file = NamedTempFile::new().unwrap();
        writeln!(schema_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            schema_file,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">"#
        )
        .unwrap();
        writeln!(
            schema_file,
            r#"  <xs:element name="root" type="xs:string"/>"#
        )
        .unwrap();
        writeln!(schema_file, r#"</xs:schema>"#).unwrap();
        schema_file.flush().unwrap();

        let result = loader.load_local_schema(schema_file.path()).await;
        assert!(result.is_ok());

        let cached_schema = result.unwrap();
        let schema_content = std::str::from_utf8(&cached_schema.data).unwrap();
        assert!(schema_content.contains("<xs:schema"));
    }

    #[tokio::test]
    async fn test_load_local_schema_not_found() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        let result = loader
            .load_local_schema(Path::new("/nonexistent/schema.xsd"))
            .await;
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidationError::SchemaNotFound { .. } => (),
            _ => panic!("Expected SchemaNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_validate_schema_content_valid() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        let valid_schema = br#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

        let result = loader.validate_schema_content(valid_schema, "test.xsd");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_schema_content_invalid_utf8() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        let invalid_utf8 = &[0xFF, 0xFE, 0xFD];

        let result = loader.validate_schema_content(invalid_utf8, "test.xsd");
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidationError::SchemaParsing { details, .. } => {
                assert!(details.contains("not valid UTF-8"));
            }
            _ => panic!("Expected SchemaParsing error"),
        }
    }

    #[tokio::test]
    async fn test_validate_schema_content_not_xml() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        let not_xml = b"This is not XML content";

        let result = loader.validate_schema_content(not_xml, "test.xsd");
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidationError::SchemaParsing { details, .. } => {
                assert!(details.contains("does not appear to be XML"));
            }
            _ => panic!("Expected SchemaParsing error"),
        }
    }

    #[tokio::test]
    async fn test_validate_schema_content_not_schema() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache, http_client).unwrap();

        let not_schema = br#"<?xml version="1.0" encoding="UTF-8"?>
<root>
  <element>This is XML but not a schema</element>
</root>"#;

        let result = loader.validate_schema_content(not_schema, "test.xsd");
        assert!(result.is_err());

        match result.unwrap_err() {
            ValidationError::SchemaParsing { details, .. } => {
                assert!(details.contains("does not appear to be an XML Schema"));
            }
            _ => panic!("Expected SchemaParsing error"),
        }
    }

    #[tokio::test]
    async fn test_extract_schema_url_async_backward_compatibility() {
        // Create a temporary XML file with schema location
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            temp_file,
            r#"<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#
        )
        .unwrap();
        writeln!(
            temp_file,
            r#"      xsi:schemaLocation="http://example.com/ns http://example.com/schema.xsd">"#
        )
        .unwrap();
        writeln!(temp_file, r#"  <element>content</element>"#).unwrap();
        writeln!(temp_file, r#"</root>"#).unwrap();
        temp_file.flush().unwrap();

        let result = extract_schema_url_async(temp_file.path()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "http://example.com/schema.xsd");
    }

    #[tokio::test]
    async fn test_schema_loader_caching() {
        let (cache, _temp_dir) = create_test_cache();
        let http_client = create_test_http_client();
        let loader = SchemaLoader::new(cache.clone(), http_client).unwrap();

        // Create a temporary schema file
        let mut schema_file = NamedTempFile::new().unwrap();
        writeln!(schema_file, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
        writeln!(
            schema_file,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">"#
        )
        .unwrap();
        writeln!(
            schema_file,
            r#"  <xs:element name="root" type="xs:string"/>"#
        )
        .unwrap();
        writeln!(schema_file, r#"</xs:schema>"#).unwrap();
        schema_file.flush().unwrap();

        // Load schema first time
        let result1 = loader.load_local_schema(schema_file.path()).await;
        assert!(result1.is_ok());

        // Load schema second time (should hit cache)
        let result2 = loader.load_local_schema(schema_file.path()).await;
        assert!(result2.is_ok());

        // Verify both results are equivalent
        let schema1 = result1.unwrap();
        let schema2 = result2.unwrap();
        assert_eq!(schema1.data, schema2.data);
    }

    #[tokio::test]
    async fn test_determine_source_type() {
        let xml_path = Path::new("/path/to/xml/file.xml");

        // Test remote URL
        let remote_type =
            SchemaExtractor::determine_source_type("https://example.com/schema.xsd", xml_path);
        assert_eq!(
            remote_type,
            SchemaSourceType::Remote("https://example.com/schema.xsd".to_string())
        );

        // Test absolute local path
        let absolute_type =
            SchemaExtractor::determine_source_type("/absolute/schema.xsd", xml_path);
        assert_eq!(
            absolute_type,
            SchemaSourceType::Local(PathBuf::from("/absolute/schema.xsd"))
        );

        // Test relative local path
        let relative_type = SchemaExtractor::determine_source_type("schema.xsd", xml_path);
        let expected_path = PathBuf::from("/path/to/xml/schema.xsd");
        assert_eq!(relative_type, SchemaSourceType::Local(expected_path));
    }
}
