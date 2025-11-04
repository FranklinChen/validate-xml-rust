use tempfile::TempDir;

use validate_xml::{CacheConfig, Config};

#[tokio::test]
async fn test_default_config() {
    let config = Config::default();

    // Verify default configuration has valid structure
    assert_eq!(config.cache.ttl_hours, 24); // 24 hours
    assert!(config.cache.max_size_mb > 0);
    assert!(config.cache.max_memory_entries > 0);
    assert!(config.cache.memory_ttl_seconds > 0);
}

#[tokio::test]
async fn test_cache_config_creation() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 24,
        max_size_mb: 100,
        max_memory_entries: 1000,
        memory_ttl_seconds: 3600,
    };

    assert_eq!(cache_config.ttl_hours, 24);
    assert_eq!(cache_config.max_size_mb, 100);
    assert_eq!(cache_config.max_memory_entries, 1000);
    assert_eq!(cache_config.memory_ttl_seconds, 3600);
}

#[tokio::test]
async fn test_cache_config_with_default_values() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().to_path_buf(),
        ttl_hours: 12,
        max_size_mb: 50,
        max_memory_entries: 500,
        memory_ttl_seconds: 1800,
    };

    // Verify custom values are respected
    assert_eq!(cache_config.ttl_hours, 12);
    assert_eq!(cache_config.max_size_mb, 50);
}

#[tokio::test]
async fn test_config_validation_section() {
    let config = Config::default();

    // Verify validation config has reasonable defaults
    assert_eq!(config.validation.fail_fast, false);
    assert_eq!(config.validation.show_progress, false);
}

#[tokio::test]
async fn test_config_network_section() {
    let config = Config::default();

    // Verify network config is present
    assert!(config.network.timeout_seconds > 0);
}

#[tokio::test]
async fn test_config_file_section() {
    let config = Config::default();

    // Verify file config is present
    assert!(!config.files.extensions.is_empty());
}
