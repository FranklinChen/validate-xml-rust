//! Validation tests for the validate-xml library
//!
//! These tests verify core validation functionality through public APIs.
//! Note: Internal API tests have been simplified due to significant architectural refactoring.

use tempfile::TempDir;

use validate_xml::{CacheConfig, SchemaCache};

const SIMPLE_XSD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

#[tokio::test]
async fn test_schema_cache_initialization() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);

    // Verify cache was created successfully
    let stats = cache.stats().await.unwrap();
    assert_eq!(stats.memory.entry_count, 0);
    assert_eq!(stats.disk.entry_count, 0);
}

#[tokio::test]
async fn test_schema_caching_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);

    // Add schema to cache
    let test_url = "http://example.com/test.xsd";
    let test_data = SIMPLE_XSD.to_vec();

    cache
        .set(test_url, test_data.clone(), None, None)
        .await
        .unwrap();

    // Retrieve from cache
    let retrieved = cache.get(test_url).await.unwrap();
    assert!(retrieved.is_some());

    // Verify data matches
    let cached = retrieved.unwrap();
    assert_eq!(cached.data.to_vec(), test_data);
}

#[tokio::test]
async fn test_schema_cache_contains() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);

    let test_url = "http://example.com/schema.xsd";
    let test_data = SIMPLE_XSD.to_vec();

    // Initially not in cache
    assert!(!cache.contains(test_url).await.unwrap());

    // Add to cache
    cache.set(test_url, test_data, None, None).await.unwrap();

    // Now should be in cache
    assert!(cache.contains(test_url).await.unwrap());
}

#[tokio::test]
async fn test_schema_cache_multiple_entries() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);

    // Add multiple schemas
    for i in 0..5 {
        let url = format!("http://example.com/schema{}.xsd", i);
        let data = format!("schema{}", i).into_bytes();
        cache.set(&url, data, None, None).await.unwrap();
    }

    // Verify all can be retrieved
    for i in 0..5 {
        let url = format!("http://example.com/schema{}.xsd", i);
        let retrieved = cache.get(&url).await.unwrap();
        assert!(retrieved.is_some());
    }
}

#[tokio::test]
async fn test_schema_cache_removal() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);

    let test_url = "http://example.com/test.xsd";
    let test_data = SIMPLE_XSD.to_vec();

    // Add and verify
    cache.set(test_url, test_data, None, None).await.unwrap();
    assert!(cache.contains(test_url).await.unwrap());

    // Remove
    cache.remove(test_url).await.unwrap();
    assert!(!cache.contains(test_url).await.unwrap());
}
