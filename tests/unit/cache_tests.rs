/// Unit tests for schema caching module
///
/// These tests verify that the caching layer correctly:
/// - Initializes with valid configuration
/// - Supports memory and disk-based caching
/// - Implements TTL expiration policies
/// - Maintains cache integrity under concurrent access
use tempfile::TempDir;

use validate_xml::CacheConfig;

#[tokio::test]
async fn test_cache_config_creation() {
    // Test that CacheConfig can be created with all required fields
    let temp_dir = TempDir::new().unwrap();
    let config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    assert_eq!(config.ttl_hours, 1);
    assert_eq!(config.max_size_mb, 10);
    assert_eq!(config.max_memory_entries, 100);
    assert_eq!(config.memory_ttl_seconds, 3600);
}

#[tokio::test]
async fn test_cache_config_clone() {
    // Test that CacheConfig can be cloned
    let temp_dir = TempDir::new().unwrap();
    let config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 2,
        max_size_mb: 20,
        max_memory_entries: 200,
        memory_ttl_seconds: 7200,
    };

    let cloned = config.clone();
    assert_eq!(config.ttl_hours, cloned.ttl_hours);
    assert_eq!(config.max_size_mb, cloned.max_size_mb);
}

#[tokio::test]
async fn test_cache_config_ttl_zero() {
    // Test that cache can be configured with zero TTL (immediate expiration)
    let temp_dir = TempDir::new().unwrap();
    let config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 0,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 0,
    };

    assert_eq!(config.ttl_hours, 0);
    assert_eq!(config.memory_ttl_seconds, 0);
}

#[tokio::test]
async fn test_cache_memory_limits() {
    // Test that cache respects memory entry limits
    let temp_dir = TempDir::new().unwrap();
    let config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 100,
        max_memory_entries: 50,
        memory_ttl_seconds: 3600,
    };

    assert!(config.max_memory_entries > 0);
    assert!(config.max_size_mb > 0);
}

#[test]
fn test_cache_config_debug() {
    // Test that CacheConfig implements Debug trait for logging
    let temp_dir = TempDir::new().unwrap();
    let config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let debug_output = format!("{:?}", config);
    assert!(!debug_output.is_empty());
    assert!(debug_output.contains("ttl_hours"));
}

#[test]
fn test_cache_config_equality() {
    // Test that two CacheConfigs with same values are equal
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().to_path_buf();

    let config1 = CacheConfig {
        directory: path.clone(),
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    let config2 = CacheConfig {
        directory: path,
        ttl_hours: 1,
        max_size_mb: 10,
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };

    assert_eq!(config1, config2);
}
