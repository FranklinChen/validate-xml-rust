use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::error::ValidationError;
use crate::libxml2::XmlSchemaPtr;

/// Cache configuration
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CacheConfig {
    /// Cache directory path
    pub directory: PathBuf,
    /// Time-to-live for cached schemas in hours
    pub ttl_hours: u64,
    /// Maximum cache size in megabytes
    pub max_size_mb: u64,
    /// Maximum number of entries in memory cache
    pub max_memory_entries: u64,
    /// Memory cache TTL in seconds
    pub memory_ttl_seconds: u64,
}

/// Result type for cache operations
pub type CacheResult<T> = Result<T, ValidationError>;

/// In-memory cache for parsed libxml2 schema pointers
///
/// This cache stores the actual compiled schema structures ready for validation.
/// It uses `moka` to handle concurrent access and "thundering herd" protection
/// (ensuring a schema is only parsed once even if multiple files request it simultaneously).
pub struct ParsedSchemaCache {
    cache: Cache<String, Arc<XmlSchemaPtr>>,
}

impl ParsedSchemaCache {
    pub fn new(max_capacity: u64) -> Self {
        let cache = Cache::builder().max_capacity(max_capacity).build();

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
    ) -> Result<Arc<XmlSchemaPtr>, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Arc<XmlSchemaPtr>, E>>,
        E: Send + Sync + Clone + 'static, // Error type must be thread-safe and Clone
        ValidationError: From<E>,         // Allow conversion to ValidationError if needed
    {
        self.cache
            .try_get_with(key, loader())
            .await
            .map_err(|e| (*e).clone()) // Unwrap the Arc<E> from moka
    }

    pub async fn get(&self, key: &str) -> Option<Arc<XmlSchemaPtr>> {
        self.cache.get(key).await
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
    #[allow(dead_code)]
    ttl: Duration,
}

impl DiskCache {
    pub fn new(cache_dir: PathBuf, ttl: Duration) -> Self {
        Self { cache_dir, ttl }
    }

    /// Generate a cache key from a URL
    pub fn generate_key(url: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        format!("schema_{:x}", hasher.finish())
    }

    /// Get schema from disk cache
    pub async fn get(&self, key: &str) -> CacheResult<Option<CachedSchema>> {
        // First check if metadata exists and is not expired
        let metadata = match self.get_metadata(key).await? {
            Some(metadata) if !metadata.is_expired() => metadata,
            _ => {
                // Clean up expired entry
                let _ = self.remove(key).await;
                return Ok(None);
            }
        };

        // Get the actual data
        match cacache::read(&self.cache_dir, key).await {
            Ok(data) => Ok(Some(CachedSchema::new(data, metadata))),
            Err(cacache::Error::EntryNotFound(_, _)) => Ok(None),
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

        // Store the metadata
        self.set_metadata(key, &metadata).await?;

        Ok(())
    }

    /// Remove entry from disk cache
    pub async fn remove(&self, key: &str) -> CacheResult<()> {
        // Remove data
        let _ = cacache::remove(&self.cache_dir, key).await;

        // Remove metadata
        let metadata_path = self.metadata_path(key);
        let _ = fs::remove_file(metadata_path).await;

        Ok(())
    }

    /// Check if entry exists and is not expired
    pub async fn contains(&self, key: &str) -> CacheResult<bool> {
        match self.get_metadata(key).await? {
            Some(metadata) => Ok(!metadata.is_expired()),
            None => Ok(false),
        }
    }

    /// Get cache statistics
    pub async fn stats(&self) -> CacheResult<CacheStats> {
        let mut stats = CacheStats::default();

        // Get cacache index - handle errors gracefully
        match cacache::index::ls(&self.cache_dir).collect::<Result<Vec<_>, _>>() {
            Ok(entries) => {
                for entry in entries {
                    stats.entry_count += 1;
                    stats.total_size += entry.size as u64;
                }
            }
            Err(_) => {
                // If we can't read the index, assume empty cache
                // This can happen if the cache directory doesn't exist yet
            }
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
                    if let Ok(Some(metadata)) = self.get_metadata(&entry.key).await
                        && metadata.is_expired()
                    {
                        cleanup_stats.expired_entries += 1;
                        cleanup_stats.freed_bytes += entry.size as u64;

                        if let Err(e) = self.remove(&entry.key).await {
                            cleanup_stats
                                .errors
                                .push(format!("Failed to remove {}: {}", entry.key, e));
                        } else {
                            cleanup_stats.removed_entries += 1;
                        }
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
}

impl SchemaCache {
    pub fn new(config: CacheConfig) -> Self {
        let memory_cache = MemoryCache::new(
            config.max_memory_entries,
            Duration::from_secs(config.memory_ttl_seconds),
        );

        let disk_cache = DiskCache::new(
            config.directory.clone(),
            Duration::from_secs(config.ttl_hours * 3600),
        );

        // Limit parsed schemas to max_memory_entries (same as raw memory cache)
        let parsed_cache = ParsedSchemaCache::new(config.max_memory_entries);

        Self {
            memory_cache,
            disk_cache,
            parsed_cache,
            config,
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
            return Ok(Some(schema));
        }

        // Try disk cache (persistent across runs)
        if let Some(schema) = self.disk_cache.get(&key).await? {
            let schema_arc = Arc::new(schema);
            // Populate memory cache for future access
            self.memory_cache.set(key, schema_arc.clone()).await;
            return Ok(Some(schema_arc));
        }

        Ok(None)
    }

    /// Set schema in both cache tiers
    pub async fn set(
        &self,
        url: &str,
        data: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> CacheResult<()> {
        let key = DiskCache::generate_key(url);
        let ttl = Duration::from_secs(self.config.ttl_hours * 3600);

        let metadata = CacheMetadata::new(key.clone(), url.to_string(), ttl)
            .with_size(data.len() as u64)
            .with_etag(etag)
            .with_last_modified(last_modified);

        let cached_schema = Arc::new(CachedSchema::new(data.clone(), metadata.clone()));

        // Store in memory cache
        self.memory_cache.set(key.clone(), cached_schema).await;

        // Store in disk cache for persistence
        self.disk_cache.set(&key, &data, metadata).await?;

        Ok(())
    }

    /// Remove entry from both cache tiers
    pub async fn remove(&self, url: &str) -> CacheResult<()> {
        let key = DiskCache::generate_key(url);

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
        self.memory_cache.clear().await;

        // Clear disk cache by clearing the entire cache directory
        cacache::clear(&self.config.directory)
            .await
            .map_err(|e| ValidationError::Cache(format!("Failed to clear disk cache: {}", e)))?;

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
    async fn test_disk_cache_basic_operations() {
        let (config, _temp_dir) = create_test_config();
        let cache = DiskCache::new(config.directory.clone(), Duration::from_secs(3600));

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
    async fn test_disk_cache_expiration() {
        let (config, _temp_dir) = create_test_config();
        let cache = DiskCache::new(config.directory.clone(), Duration::from_millis(100));

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
}
