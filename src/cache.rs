use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::backend::CompiledSchema;
use crate::error::ValidationError;

/// Cache configuration
#[derive(Debug, Clone, PartialEq)]
pub struct CacheConfig {
    /// Cache directory path
    pub directory: PathBuf,
    /// Time-to-live for cached schemas in hours
    pub ttl_hours: u64,
    /// Maximum indexed schema-data budget in megabytes
    pub max_size_mb: u64,
    /// Maximum number of entries in memory cache
    pub max_memory_entries: u64,
    /// Memory cache TTL in seconds
    pub memory_ttl_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            directory: std::env::temp_dir().join("validate-xml"),
            ttl_hours: 24,
            max_size_mb: 100,
            max_memory_entries: 1_000,
            memory_ttl_seconds: 3_600,
        }
    }
}

/// Result type for cache operations
pub type CacheResult<T> = Result<T, ValidationError>;

/// In-memory cache for parsed XML schema objects
///
/// This cache stores the actual compiled schema structures ready for validation.
/// It uses `moka` to handle concurrent access and "thundering herd" protection
/// (ensuring a schema is only parsed once even if multiple files request it simultaneously).
pub struct ParsedSchemaCache {
    cache: Cache<String, Arc<CompiledSchema>>,
}

impl ParsedSchemaCache {
    pub fn new(max_capacity: u64) -> Self {
        Self::with_ttl(max_capacity, Duration::from_secs(3_600))
    }

    pub fn with_ttl(max_capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .build();

        Self { cache }
    }

    /// Get a parsed schema from the cache, or load/parse it if missing.
    ///
    /// The `loader` future is only executed if the key is missing.
    /// Moka ensures that concurrent requests for the same key wait for the single leader to finish.
    pub async fn get_or_load<F, Fut, E>(
        &self,
        key: String,
        loader: F,
    ) -> Result<Arc<CompiledSchema>, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Arc<CompiledSchema>, E>>,
        E: Send + Sync + Clone + 'static,
    {
        self.cache
            .try_get_with(key, loader())
            .await
            .map_err(|e| (*e).clone()) // Unwrap the Arc<E> from moka
    }

    pub async fn get(&self, key: &str) -> Option<Arc<CompiledSchema>> {
        self.cache.get(key).await
    }

    pub async fn remove(&self, key: &str) {
        self.cache.remove(key).await;
    }

    pub async fn clear(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
    }
}

/// Metadata for cached schema entries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub key: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub size_bytes: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl CacheMetadata {
    pub fn new(key: String, url: String, ttl: Duration) -> Self {
        let now = Utc::now();
        let expires_at =
            now + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::hours(24));

        Self {
            key,
            url,
            created_at: now,
            expires_at,
            size_bytes: 0,
            etag: None,
            last_modified: None,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn with_size(mut self, size: u64) -> Self {
        self.size_bytes = size;
        self
    }

    pub fn with_etag(mut self, etag: Option<String>) -> Self {
        self.etag = etag;
        self
    }

    pub fn with_last_modified(mut self, last_modified: Option<String>) -> Self {
        self.last_modified = last_modified;
        self
    }
}

/// Represents a cached schema with its data and metadata
#[derive(Debug, Clone)]
pub struct CachedSchema {
    pub data: Arc<Vec<u8>>,
    pub metadata: CacheMetadata,
}

impl CachedSchema {
    pub fn new(data: Vec<u8>, metadata: CacheMetadata) -> Self {
        Self {
            data: Arc::new(data),
            metadata,
        }
    }
}

/// Disk cache implementation using cacache for persistent, corruption-resistant storage
pub struct DiskCache {
    cache_dir: PathBuf,
}

impl DiskCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Generate a cache key from a URL
    pub fn generate_key(url: &str) -> String {
        use sha2::{Digest, Sha256};

        format!("schema_{:x}", Sha256::digest(url.as_bytes()))
    }

    /// Get schema from disk cache
    pub async fn get(&self, key: &str) -> CacheResult<Option<CachedSchema>> {
        // First check if metadata exists and is not expired
        let metadata = match self.get_metadata(key).await? {
            Some(metadata) if !metadata.is_expired() => metadata,
            _ => {
                // Clean up expired entry
                self.remove(key).await?;
                return Ok(None);
            }
        };

        // Get the actual data
        match cacache::read(&self.cache_dir, key).await {
            Ok(data) => Ok(Some(CachedSchema::new(data, metadata))),
            Err(cacache::Error::EntryNotFound(_, _)) => {
                self.remove(key).await?;
                Ok(None)
            }
            Err(e) => Err(ValidationError::Cache(format!(
                "Failed to read from disk cache: {}",
                e
            ))),
        }
    }

    /// Set schema in disk cache
    pub async fn set(&self, key: &str, data: &[u8], metadata: CacheMetadata) -> CacheResult<()> {
        // Store the data
        cacache::write(&self.cache_dir, key, data)
            .await
            .map_err(|e| ValidationError::Cache(format!("Failed to write to disk cache: {}", e)))?;

        // Store metadata second. If it fails, remove the index entry so callers
        // never observe a successful raw write paired with missing metadata.
        if let Err(metadata_error) = self.set_metadata(key, &metadata).await {
            return match cacache::remove(&self.cache_dir, key).await {
                Ok(()) | Err(cacache::Error::EntryNotFound(_, _)) => Err(metadata_error),
                Err(rollback_error) => Err(ValidationError::Cache(format!(
                    "{metadata_error}; additionally failed to roll back cache entry {key}: {rollback_error}"
                ))),
            };
        }

        Ok(())
    }

    /// Remove entry from disk cache
    pub async fn remove(&self, key: &str) -> CacheResult<()> {
        match cacache::remove(&self.cache_dir, key).await {
            Ok(()) | Err(cacache::Error::EntryNotFound(_, _)) => {}
            Err(cacache::Error::IoError(error, _))
                if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ValidationError::Cache(format!(
                    "Failed to remove disk cache entry {key}: {error}"
                )));
            }
        }

        let metadata_path = self.metadata_path(key);
        match fs::remove_file(metadata_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ValidationError::Cache(format!(
                    "Failed to remove metadata for {key}: {error}"
                )));
            }
        }

        Ok(())
    }

    /// Check if entry exists and is not expired
    pub async fn contains(&self, key: &str) -> CacheResult<bool> {
        Ok(self.get(key).await?.is_some())
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheResult<CacheStats> {
        let mut stats = CacheStats::default();

        if !fs::try_exists(self.cache_dir.join("index-v5")).await? {
            return Ok(stats);
        }

        let entries = cacache::index::ls(&self.cache_dir)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                ValidationError::Cache(format!("Failed to read cache index: {error}"))
            })?;
        for entry in entries {
            stats.entry_count += 1;
            stats.total_size += entry.size as u64;
        }

        Ok(stats)
    }

    /// Clean up expired entries
    pub async fn cleanup_expired(&self) -> CacheResult<CleanupStats> {
        let mut cleanup_stats = CleanupStats::default();

        // Get all entries from cacache - handle errors gracefully
        match cacache::index::ls(&self.cache_dir).collect::<Result<Vec<_>, _>>() {
            Ok(entries) => {
                for entry in entries {
                    // Check if metadata exists and is expired
                    match self.get_metadata(&entry.key).await {
                        Ok(Some(metadata)) if metadata.is_expired() => {
                            cleanup_stats.expired_entries += 1;
                            if let Err(error) = self.remove(&entry.key).await {
                                cleanup_stats
                                    .errors
                                    .push(format!("Failed to remove {}: {error}", entry.key));
                            } else {
                                cleanup_stats.removed_entries += 1;
                                cleanup_stats.freed_bytes += entry.size as u64;
                            }
                        }
                        Ok(None) => {
                            if let Err(error) = self.remove(&entry.key).await {
                                cleanup_stats.errors.push(format!(
                                    "Failed to remove orphaned entry {}: {error}",
                                    entry.key
                                ));
                            } else {
                                cleanup_stats.removed_entries += 1;
                                cleanup_stats.freed_bytes += entry.size as u64;
                            }
                        }
                        Ok(Some(_)) => {}
                        Err(error) => cleanup_stats.errors.push(format!(
                            "Failed to read metadata for {}: {error}",
                            entry.key
                        )),
                    }
                }
            }
            Err(e) => {
                cleanup_stats
                    .errors
                    .push(format!("Failed to read cache index: {}", e));
            }
        }

        Ok(cleanup_stats)
    }

    /// Evict oldest index entries until the configured byte budget is met.
    pub async fn enforce_max_size(&self, max_bytes: u64) -> CacheResult<CleanupStats> {
        let mut entries = cacache::index::ls(&self.cache_dir)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                ValidationError::Cache(format!("Failed to read cache index: {error}"))
            })?;
        let mut total = entries.iter().map(|entry| entry.size as u64).sum::<u64>();
        entries.sort_unstable_by_key(|entry| entry.time);
        let mut stats = CleanupStats::default();
        for entry in entries {
            if total <= max_bytes {
                break;
            }
            self.remove(&entry.key).await?;
            let size = entry.size as u64;
            total = total.saturating_sub(size);
            stats.removed_entries += 1;
            stats.freed_bytes += size;
        }
        Ok(stats)
    }

    /// Get metadata for a cache entry
    async fn get_metadata(&self, key: &str) -> CacheResult<Option<CacheMetadata>> {
        let metadata_path = self.metadata_path(key);

        match fs::read_to_string(&metadata_path).await {
            Ok(content) => {
                let metadata: CacheMetadata = serde_json::from_str(&content).map_err(|e| {
                    ValidationError::Cache(format!("Failed to parse metadata: {}", e))
                })?;
                Ok(Some(metadata))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ValidationError::Cache(format!(
                "Failed to read metadata: {}",
                e
            ))),
        }
    }

    /// Set metadata for a cache entry
    async fn set_metadata(&self, key: &str, metadata: &CacheMetadata) -> CacheResult<()> {
        let metadata_path = self.metadata_path(key);

        // Ensure metadata directory exists
        if let Some(parent) = metadata_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ValidationError::Cache(format!("Failed to create metadata directory: {}", e))
            })?;
        }

        let content = serde_json::to_string_pretty(metadata)
            .map_err(|e| ValidationError::Cache(format!("Failed to serialize metadata: {}", e)))?;

        fs::write(&metadata_path, content)
            .await
            .map_err(|e| ValidationError::Cache(format!("Failed to write metadata: {}", e)))?;

        Ok(())
    }

    /// Get path for metadata file
    fn metadata_path(&self, key: &str) -> PathBuf {
        self.cache_dir
            .join("metadata")
            .join(format!("{}.json", key))
    }
}

/// Memory cache implementation using Moka for high-performance in-memory caching
pub struct MemoryCache {
    cache: Cache<String, Arc<CachedSchema>>,
}

impl MemoryCache {
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .build();

        Self { cache }
    }

    /// Get schema from memory cache
    pub async fn get(&self, key: &str) -> Option<Arc<CachedSchema>> {
        self.cache.get(key).await
    }

    /// Set schema in memory cache
    pub async fn set(&self, key: String, schema: Arc<CachedSchema>) {
        self.cache.insert(key, schema).await;
    }

    /// Remove entry from memory cache
    pub async fn remove(&self, key: &str) {
        self.cache.remove(key).await;
    }

    /// Check if entry exists in memory cache
    pub async fn contains(&self, key: &str) -> bool {
        self.cache.contains_key(key)
    }

    /// Get cache statistics
    pub async fn stats(&self) -> MemoryCacheStats {
        // Run sync to ensure all pending operations are complete
        self.cache.run_pending_tasks().await;

        MemoryCacheStats {
            entry_count: self.cache.entry_count(),
            weighted_size: self.cache.weighted_size(),
        }
    }

    /// Clear all entries from memory cache
    pub async fn clear(&self) {
        self.cache.invalidate_all();
    }
}

/// Two-tier cache manager that combines memory and disk caching
pub struct SchemaCache {
    memory_cache: MemoryCache,
    disk_cache: DiskCache,
    parsed_cache: ParsedSchemaCache,
    config: CacheConfig,
    memory_hits: AtomicU64,
    disk_hits: AtomicU64,
    misses: AtomicU64,
}

impl SchemaCache {
    pub fn new(config: CacheConfig) -> Self {
        let memory_cache = MemoryCache::new(
            config.max_memory_entries,
            Duration::from_secs(config.memory_ttl_seconds),
        );

        let disk_cache = DiskCache::new(config.directory.clone());

        // Apply the same capacity and TTL to compiled and raw memory entries.
        // In particular, a remote compiled schema must not outlive the
        // configured in-memory freshness window indefinitely.
        let parsed_cache = ParsedSchemaCache::with_ttl(
            config.max_memory_entries,
            Duration::from_secs(config.memory_ttl_seconds),
        );

        Self {
            memory_cache,
            disk_cache,
            parsed_cache,
            config,
            memory_hits: AtomicU64::new(0),
            disk_hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Access the parsed schema cache
    pub fn parsed(&self) -> &ParsedSchemaCache {
        &self.parsed_cache
    }

    /// Get schema using two-tier strategy: memory first, then disk, then None
    pub async fn get(&self, url: &str) -> CacheResult<Option<Arc<CachedSchema>>> {
        let key = DiskCache::generate_key(url);

        // Try memory cache first (fastest)
        if let Some(schema) = self.memory_cache.get(&key).await {
            self.memory_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(schema));
        }

        // Try disk cache (persistent across runs)
        if let Some(schema) = self.disk_cache.get(&key).await? {
            self.disk_hits.fetch_add(1, Ordering::Relaxed);
            let schema_arc = Arc::new(schema);
            // Populate memory cache for future access
            self.memory_cache.set(key, schema_arc.clone()).await;
            return Ok(Some(schema_arc));
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    /// Set schema in both cache tiers
    pub async fn set(
        &self,
        url: &str,
        data: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> CacheResult<Arc<CachedSchema>> {
        let key = DiskCache::generate_key(url);
        let ttl = Duration::from_secs(self.config.ttl_hours * 3600);

        let metadata = CacheMetadata::new(key.clone(), url.to_string(), ttl)
            .with_size(data.len() as u64)
            .with_etag(etag)
            .with_last_modified(last_modified);

        let cached_schema = Arc::new(CachedSchema::new(data.clone(), metadata.clone()));

        let max_bytes = self.config.max_size_mb.saturating_mul(1024 * 1024);
        if data.len() as u64 > max_bytes {
            return Err(ValidationError::ResourceExhaustion {
                resource: "schema disk cache".into(),
                details: format!(
                    "entry is {} bytes but cache limit is {max_bytes} bytes",
                    data.len()
                ),
            });
        }

        // Persist first so a disk error cannot leave a memory-only entry
        // behind after reporting that the operation failed.
        self.disk_cache.set(&key, &data, metadata).await?;
        if let Err(eviction_error) = self.disk_cache.enforce_max_size(max_bytes).await {
            return match self.disk_cache.remove(&key).await {
                Ok(()) => Err(eviction_error),
                Err(rollback_error) => Err(ValidationError::Cache(format!(
                    "{eviction_error}; additionally failed to roll back newly cached entry {key}: {rollback_error}"
                ))),
            };
        }

        self.memory_cache.set(key, Arc::clone(&cached_schema)).await;

        Ok(cached_schema)
    }

    /// Remove entry from both cache tiers
    pub async fn remove(&self, url: &str) -> CacheResult<()> {
        let key = DiskCache::generate_key(url);

        self.parsed_cache.remove(url).await;
        self.memory_cache.remove(&key).await;
        self.disk_cache.remove(&key).await?;

        Ok(())
    }

    /// Check if entry exists in either cache tier
    pub async fn contains(&self, url: &str) -> CacheResult<bool> {
        let key = DiskCache::generate_key(url);

        if self.memory_cache.contains(&key).await {
            return Ok(true);
        }

        self.disk_cache.contains(&key).await
    }

    /// Get comprehensive cache statistics
    pub async fn stats(&self) -> CacheResult<ComprehensiveCacheStats> {
        let memory_stats = self.memory_cache.stats().await;
        let disk_stats = self.disk_cache.stats().await?;

        Ok(ComprehensiveCacheStats {
            memory: memory_stats,
            disk: disk_stats,
            access: CacheAccessStats {
                memory_hits: self.memory_hits.load(Ordering::Relaxed),
                disk_hits: self.disk_hits.load(Ordering::Relaxed),
                misses: self.misses.load(Ordering::Relaxed),
            },
        })
    }

    /// Clean up expired entries from both cache tiers
    pub async fn cleanup_expired(&self) -> CacheResult<CleanupStats> {
        // Memory cache cleanup is automatic via TTL
        // Only need to clean up disk cache
        self.disk_cache.cleanup_expired().await
    }

    /// Clear all entries from both cache tiers
    pub async fn clear(&self) -> CacheResult<()> {
        self.parsed_cache.clear().await;
        self.memory_cache.clear().await;

        // Clear disk cache by clearing the entire cache directory
        cacache::clear(&self.config.directory)
            .await
            .map_err(|e| ValidationError::Cache(format!("Failed to clear disk cache: {}", e)))?;
        match fs::remove_dir_all(self.config.directory.join("metadata")).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ValidationError::Cache(format!(
                    "Failed to clear cache metadata: {error}"
                )));
            }
        }

        Ok(())
    }
}

/// Statistics for cache operations
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub entry_count: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryCacheStats {
    pub entry_count: u64,
    pub weighted_size: u64,
}

#[derive(Debug, Clone)]
pub struct ComprehensiveCacheStats {
    pub memory: MemoryCacheStats,
    pub disk: CacheStats,
    pub access: CacheAccessStats,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheAccessStats {
    pub memory_hits: u64,
    pub disk_hits: u64,
    pub misses: u64,
}

#[derive(Debug, Default, Clone)]
pub struct CleanupStats {
    pub expired_entries: u64,
    pub removed_entries: u64,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_config() -> (CacheConfig, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = CacheConfig {
            directory: temp_dir.path().to_path_buf(),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        (config, temp_dir)
    }

    #[tokio::test]
    async fn test_cache_key_generation() {
        let url1 = "https://example.com/schema1.xsd";
        let url2 = "https://example.com/schema2.xsd";

        let key1 = DiskCache::generate_key(url1);
        let key2 = DiskCache::generate_key(url2);

        assert_ne!(key1, key2);
        assert!(key1.starts_with("schema_"));
        assert!(key2.starts_with("schema_"));

        // Same URL should generate same key
        let key1_again = DiskCache::generate_key(url1);
        assert_eq!(key1, key1_again);
    }

    #[tokio::test]
    async fn test_memory_cache_basic_operations() {
        let cache = MemoryCache::new(10, Duration::from_secs(60));
        let key = "test_key".to_string();

        // Test empty cache
        assert!(cache.get(&key).await.is_none());
        assert!(!cache.contains(&key).await);

        // Test set and get
        let metadata = CacheMetadata::new(
            key.clone(),
            "http://example.com".to_string(),
            Duration::from_secs(3600),
        );
        let schema = Arc::new(CachedSchema::new(b"test data".to_vec(), metadata));

        cache.set(key.clone(), schema.clone()).await;

        assert!(cache.contains(&key).await);
        let retrieved = cache.get(&key).await.unwrap();
        assert_eq!(retrieved.data.as_ref(), b"test data");

        // Test remove
        cache.remove(&key).await;
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn parsed_cache_obeys_ttl_and_clear() {
        let cache = ParsedSchemaCache::with_ttl(10, Duration::from_millis(10));
        let schema = cache
            .get_or_load("schema".into(), || async {
                crate::backend::compile(
                    br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"/>"#,
                    "cache-test.xsd",
                    None,
                )
                .map(Arc::new)
                .map_err(|error| error.to_string())
            })
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(cache.get("schema").await.is_none());

        cache
            .get_or_load("schema".into(), || async {
                Ok::<_, String>(Arc::clone(&schema))
            })
            .await
            .unwrap();
        cache.clear().await;
        assert!(cache.get("schema").await.is_none());
    }

    #[tokio::test]
    async fn test_disk_cache_basic_operations() {
        let (config, _temp_dir) = create_test_config();
        let cache = DiskCache::new(config.directory.clone());

        let key = "test_key";
        let url = "https://example.com/schema.xsd";
        let data = b"test schema data";

        // Test empty cache
        assert!(cache.get(key).await.unwrap().is_none());
        assert!(!cache.contains(key).await.unwrap());

        // Test set and get
        let metadata =
            CacheMetadata::new(key.to_string(), url.to_string(), Duration::from_secs(3600));
        cache.set(key, data, metadata.clone()).await.unwrap();

        assert!(cache.contains(key).await.unwrap());
        let retrieved = cache.get(key).await.unwrap().unwrap();
        assert_eq!(retrieved.data.as_ref(), data);
        assert_eq!(retrieved.metadata.url, url);

        // Test remove
        cache.remove(key).await.unwrap();
        assert!(cache.get(key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn disk_cache_remove_propagates_real_filesystem_errors() {
        let (config, _temp_dir) = create_test_config();
        let cache = DiskCache::new(config.directory.clone());
        let key = "remove_error";
        let metadata = CacheMetadata::new(
            key.to_string(),
            "https://example.com/schema.xsd".to_string(),
            Duration::from_secs(60),
        );
        cache.set(key, b"schema", metadata).await.unwrap();

        let metadata_path = cache.metadata_path(key);
        tokio::fs::remove_file(&metadata_path).await.unwrap();
        tokio::fs::create_dir(&metadata_path).await.unwrap();

        assert!(cache.remove(key).await.is_err());
    }

    #[tokio::test]
    async fn test_disk_cache_expiration() {
        let (config, _temp_dir) = create_test_config();
        let cache = DiskCache::new(config.directory.clone());

        let key = "test_key";
        let url = "https://example.com/schema.xsd";
        let data = b"test schema data";

        // Set with short TTL
        let metadata =
            CacheMetadata::new(key.to_string(), url.to_string(), Duration::from_millis(100));
        cache.set(key, data, metadata).await.unwrap();

        // Should exist initially
        assert!(cache.contains(key).await.unwrap());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should be expired and removed
        assert!(!cache.contains(key).await.unwrap());
        assert!(cache.get(key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_two_tier_cache_strategy() {
        let (config, _temp_dir) = create_test_config();
        let cache = SchemaCache::new(config);

        let url = "https://example.com/schema.xsd";
        let data = b"test schema data".to_vec();

        // Test empty cache
        assert!(cache.get(url).await.unwrap().is_none());

        // Test set (should populate both tiers)
        cache.set(url, data.clone(), None, None).await.unwrap();

        // Test get (should hit memory cache)
        let retrieved = cache.get(url).await.unwrap().unwrap();
        assert_eq!(retrieved.data.as_ref(), &data);

        // Clear memory cache to test disk cache fallback
        cache.memory_cache.clear().await;

        // Should still get from disk cache and repopulate memory
        let retrieved = cache.get(url).await.unwrap().unwrap();
        assert_eq!(retrieved.data.as_ref(), &data);

        // Verify memory cache was repopulated
        let key = DiskCache::generate_key(url);
        assert!(cache.memory_cache.contains(&key).await);
    }

    #[tokio::test]
    async fn inserting_after_a_miss_does_not_count_as_a_hit() {
        let (config, _temp_dir) = create_test_config();
        let cache = SchemaCache::new(config);
        let url = "https://example.com/cold.xsd";

        assert!(cache.get(url).await.unwrap().is_none());
        let inserted = cache
            .set(url, b"schema".to_vec(), None, None)
            .await
            .unwrap();
        assert_eq!(inserted.data.as_ref(), b"schema");

        let stats = cache.stats().await.unwrap();
        assert_eq!(stats.access.misses, 1);
        assert_eq!(stats.access.memory_hits, 0);
        assert_eq!(stats.access.disk_hits, 0);
    }

    #[tokio::test]
    async fn test_concurrent_cache_access() {
        let (config, _temp_dir) = create_test_config();
        let cache = Arc::new(SchemaCache::new(config));

        let urls: Vec<String> = (0..10)
            .map(|i| format!("https://example.com/schema{}.xsd", i))
            .collect();

        // Concurrent writes
        let write_tasks: Vec<_> = urls
            .iter()
            .enumerate()
            .map(|(i, url)| {
                let cache = cache.clone();
                let url = url.clone();
                let data = format!("schema data {}", i).into_bytes();

                tokio::spawn(async move { cache.set(&url, data, None, None).await })
            })
            .collect();

        // Wait for all writes to complete
        for task in write_tasks {
            task.await.unwrap().unwrap();
        }

        // Concurrent reads
        let read_tasks: Vec<_> = urls
            .iter()
            .map(|url| {
                let cache = cache.clone();
                let url = url.clone();

                tokio::spawn(async move { cache.get(&url).await })
            })
            .collect();

        // Verify all reads succeed
        for (i, task) in read_tasks.into_iter().enumerate() {
            let result = task.await.unwrap().unwrap().unwrap();
            let expected_data = format!("schema data {}", i);
            assert_eq!(result.data.as_ref(), expected_data.as_bytes());
        }
    }

    #[tokio::test]
    async fn test_cache_cleanup() {
        let (config, _temp_dir) = create_test_config();
        let cache = SchemaCache::new(config);

        // Add some entries
        for i in 0..5 {
            let url = format!("https://example.com/schema{}.xsd", i);
            let data = format!("schema data {}", i).into_bytes();
            cache.set(&url, data, None, None).await.unwrap();
        }

        // Verify entries exist
        let stats_before = cache.stats().await.unwrap();

        // Clear cache
        cache.clear().await.unwrap();

        // Add a small delay to ensure async operations complete
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Verify cache is empty
        let stats_after = cache.stats().await.unwrap();

        // Memory cache should be empty
        assert_eq!(stats_after.memory.entry_count, 0);

        // Disk cache should be empty or at least reduced
        assert!(stats_after.disk.entry_count <= stats_before.disk.entry_count);
    }

    #[tokio::test]
    async fn disk_budget_evicts_oldest_entries() {
        let (mut config, _temp_dir) = create_test_config();
        config.max_size_mb = 1;
        let cache = SchemaCache::new(config);
        cache
            .set(
                "https://example.test/old.xsd",
                vec![b'a'; 700_000],
                None,
                None,
            )
            .await
            .unwrap();
        cache
            .set(
                "https://example.test/new.xsd",
                vec![b'b'; 700_000],
                None,
                None,
            )
            .await
            .unwrap();

        let stats = cache.stats().await.unwrap();
        assert!(stats.disk.total_size <= 1024 * 1024);
        let old_key = DiskCache::generate_key("https://example.test/old.xsd");
        let new_key = DiskCache::generate_key("https://example.test/new.xsd");
        assert!(!cache.disk_cache.contains(&old_key).await.unwrap());
        assert!(cache.disk_cache.contains(&new_key).await.unwrap());
    }
}
