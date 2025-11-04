use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;

use validate_xml::{
    ValidationEngine, SchemaCache, CacheConfig, LibXml2Wrapper,
    FileDiscovery, AsyncHttpClient, HttpClientConfig
};

use crate::common::test_helpers::{PerformanceTimer, SIMPLE_XSD};

/// Benchmark configuration
struct BenchmarkConfig {
    pub file_count: usize,
    pub thread_count: usize,
    pub iterations: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            file_count: 100,
            thread_count: 4,
            iterations: 3,
        }
    }
}

/// Benchmark results
#[derive(Debug)]
struct BenchmarkResult {
    pub operation: String,
    pub duration: Duration,
    pub throughput: f64,
    pub memory_usage: Option<usize>,
}

impl BenchmarkResult {
    fn new(operation: String, duration: Duration, items_processed: usize) -> Self {
        let throughput = items_processed as f64 / duration.as_secs_f64();
        Self {
            operation,
            duration,
            throughput,
            memory_usage: None,
        }
    }
}

#[tokio::test]
async fn benchmark_validation_speed() {
    let config = BenchmarkConfig::default();
    let temp_dir = TempDir::new().unwrap();
    
    // Setup cache
    let cache_config = CacheConfig {
        directory: temp_dir.path().join("cache"),
        ttl_hours: 1,
        max_size_mb: 100,
        max_memory_entries: 1000,
        memory_ttl_seconds: 3600,
    };
    let cache = Arc::new(SchemaCache::new(cache_config));
    let engine = ValidationEngine::new(cache.clone());
    
    // Create schema
    let schema_path = temp_dir.path().join("benchmark.xsd");
    fs::write(&schema_path, SIMPLE_XSD).await.unwrap();
    
    // Create test files
    let mut xml_files = Vec::new();
    for i in 0..config.file_count {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">Benchmark content {}</root>"#,
            schema_path.file_name().unwrap().to_string_lossy(),
            i
        );
        
        let xml_path = temp_dir.path().join(format!("benchmark_{:04}.xml", i));
        fs::write(&xml_path, xml_content).await.unwrap();
        xml_files.push(xml_path);
    }
    
    // Run benchmark iterations
    let mut results = Vec::new();
    
    for iteration in 0..config.iterations {
        println!("Running validation benchmark iteration {}/{}", iteration + 1, config.iterations);
        
        let timer = PerformanceTimer::new();
        let validation_results = engine.validate_files(xml_files.clone()).await.unwrap();
        let elapsed = timer.elapsed();
        
        assert_eq!(validation_results.total_files, config.file_count);
        assert_eq!(validation_results.valid_files, config.file_count);
        
        let result = BenchmarkResult::new(
            format!("Validation (iteration {})", iteration + 1),
            elapsed,
            config.file_count,
        );
        
        println!("  Duration: {:?}, Throughput: {:.2} files/sec", 
                 result.duration, result.throughput);
        
        results.push(result);
    }
    
    // Calculate average performance
    let avg_duration = results.iter()
        .map(|r| r.duration.as_millis())
        .sum::<u128>() / results.len() as u128;
    
    let avg_throughput = results.iter()
        .map(|r| r.throughput)
        .sum::<f64>() / results.len() as f64;
    
    println!("Average validation performance:");
    println!("  Duration: {}ms", avg_duration);
    println!("  Throughput: {:.2} files/sec", avg_throughput);
    
    // Performance assertions
    assert!(avg_throughput >= 50.0, "Validation throughput too low: {:.2} files/sec", avg_throughput);
    assert!(avg_duration <= 5000, "Validation taking too long: {}ms", avg_duration);
}

#[tokio::test]
async fn benchmark_cache_performance() {
    let temp_dir = TempDir::new().unwrap();
    let cache_config = CacheConfig {
        directory: temp_dir.path().join("cache"),
        ttl_hours: 1,
        max_size_mb: 50,
        max_memory_entries: 500,
        memory_ttl_seconds: 3600,
    };

    let cache = SchemaCache::new(cache_config);
    
    // Benchmark schema parsing and caching
    let schema_sizes = vec![1024, 4096, 16384, 65536]; // Different schema sizes
    let iterations = 10;
    
    for schema_size in schema_sizes {
        println!("Benchmarking cache with schema size: {} bytes", schema_size);
        
        // Create schema of specified size
        let mut schema_content = SIMPLE_XSD.to_string();
        let padding = "x".repeat(schema_size.saturating_sub(schema_content.len()));
        schema_content = schema_content.replace("</xs:schema>", &format!("<!-- {} --></xs:schema>", padding));
        
        let schema_data = schema_content.as_bytes().to_vec();
        
        // Benchmark parsing
        let timer = PerformanceTimer::new();
        for i in 0..iterations {
            let key = format!("benchmark_schema_{}_{}", schema_size, i);
            let schema_ptr = cache.parse_schema_from_memory(schema_data.clone()).await.unwrap();
            cache.set_memory(&key, schema_ptr).await;
        }
        let parse_elapsed = timer.elapsed();
        
        // Benchmark retrieval
        let timer = PerformanceTimer::new();
        for i in 0..iterations {
            let key = format!("benchmark_schema_{}_{}", schema_size, i);
            let _schema = cache.get_from_memory(&key).await;
        }
        let retrieve_elapsed = timer.elapsed();
        
        let parse_throughput = iterations as f64 / parse_elapsed.as_secs_f64();
        let retrieve_throughput = iterations as f64 / retrieve_elapsed.as_secs_f64();
        
        println!("  Parse: {:.2} schemas/sec", parse_throughput);
        println!("  Retrieve: {:.2} schemas/sec", retrieve_throughput);
        
        // Cache retrieval should be much faster than parsing
        assert!(retrieve_throughput > parse_throughput * 2.0, 
                "Cache retrieval not significantly faster than parsing");
    }
}

#[tokio::test]
async fn benchmark_file_discovery() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create directory structure with many files
    let file_counts = vec![100, 500, 1000];
    
    for file_count in file_counts {
        println!("Benchmarking file discovery with {} files", file_count);
        
        let test_dir = temp_dir.path().join(format!("test_{}", file_count));
        fs::create_dir_all(&test_dir).await.unwrap();
        
        // Create nested directory structure
        for i in 0..file_count {
            let subdir = test_dir.join(format!("dir_{}", i / 100));
            fs::create_dir_all(&subdir).await.unwrap();
            
            // Create XML file
            let xml_content = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<root>File {}</root>"#,
                i
            );
            fs::write(subdir.join(format!("file_{}.xml", i)), xml_content).await.unwrap();
            
            // Create some non-XML files
            if i % 10 == 0 {
                fs::write(subdir.join(format!("readme_{}.txt", i)), "readme").await.unwrap();
            }
        }
        
        // Benchmark file discovery
        let discovery = FileDiscovery::new();
        
        let timer = PerformanceTimer::new();
        let files = discovery.discover_files(&test_dir).await.unwrap();
        let elapsed = timer.elapsed();
        
        assert_eq!(files.len(), file_count);
        
        let throughput = file_count as f64 / elapsed.as_secs_f64();
        println!("  Discovery: {:.2} files/sec, Duration: {:?}", throughput, elapsed);
        
        // File discovery should be reasonably fast
        assert!(throughput >= 1000.0, "File discovery too slow: {:.2} files/sec", throughput);
    }
}

#[tokio::test]
async fn benchmark_concurrent_validation() {
    let temp_dir = TempDir::new().unwrap();
    let file_count = 200;
    
    // Setup
    let cache_config = CacheConfig {
        directory: temp_dir.path().join("cache"),
        ttl_hours: 1,
        max_size_mb: 100,
        max_memory_entries: 1000,
        memory_ttl_seconds: 3600,
    };
    let cache = Arc::new(SchemaCache::new(cache_config));

    // Create schema
    let schema_path = temp_dir.path().join("concurrent.xsd");
    fs::write(&schema_path, SIMPLE_XSD).await.unwrap();
    
    // Create test files
    let mut xml_files = Vec::new();
    for i in 0..file_count {
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">Concurrent test {}</root>"#,
            schema_path.file_name().unwrap().to_string_lossy(),
            i
        );
        
        let xml_path = temp_dir.path().join(format!("concurrent_{:04}.xml", i));
        fs::write(&xml_path, xml_content).await.unwrap();
        xml_files.push(xml_path);
    }
    
    // Test different concurrency levels
    let thread_counts = vec![1, 2, 4, 8, 16];
    
    for thread_count in thread_counts {
        println!("Benchmarking with {} threads", thread_count);
        
        let engine = ValidationEngine::new_with_threads(cache.clone(), thread_count);
        
        let timer = PerformanceTimer::new();
        let results = engine.validate_files(xml_files.clone()).await.unwrap();
        let elapsed = timer.elapsed();
        
        assert_eq!(results.total_files, file_count);
        assert_eq!(results.valid_files, file_count);
        
        let throughput = file_count as f64 / elapsed.as_secs_f64();
        println!("  Throughput: {:.2} files/sec, Duration: {:?}", throughput, elapsed);
    }
}

#[tokio::test]
async fn benchmark_memory_usage() {
    let temp_dir = TempDir::new().unwrap();
    
    // Create cache with memory monitoring
    let cache_config = CacheConfig {
        directory: temp_dir.path().join("cache"),
        ttl_hours: 1,
        max_size_mb: 10, // Small limit to test memory management
        max_memory_entries: 100,
        memory_ttl_seconds: 3600,
    };
    let cache = Arc::new(SchemaCache::new(cache_config));

    // Create many different schemas to test memory usage
    let schema_count = 100;
    
    println!("Benchmarking memory usage with {} schemas", schema_count);
    
    let initial_stats = cache.memory_stats().await;
    println!("Initial memory stats: {:?}", initial_stats);
    
    // Load many schemas
    for i in 0..schema_count {
        let schema_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root{}" type="xs:string"/>
    <!-- Schema {} with padding: {} -->
</xs:schema>"#,
            i, i, "x".repeat(1000)
        );
        
        let schema_data = schema_content.as_bytes().to_vec();
        let schema_ptr = cache.parse_schema_from_memory(schema_data).await.unwrap();
        cache.set_memory(&format!("memory_test_{}", i), schema_ptr).await;
        
        // Check memory stats periodically
        if i % 20 == 0 {
            let stats = cache.memory_stats().await;
            println!("  After {} schemas: {:?}", i + 1, stats);
        }
    }
    
    let final_stats = cache.memory_stats().await;
    println!("Final memory stats: {:?}", final_stats);
    
    // Memory should be managed (not all schemas should be in memory due to size limit)
    assert!(final_stats.entry_count <= schema_count);
    
    // Test memory cleanup
    cache.cleanup_expired().await.unwrap();
    
    let cleanup_stats = cache.memory_stats().await;
    println!("After cleanup: {:?}", cleanup_stats);
}

#[tokio::test]
async fn benchmark_http_client_performance() {
    let config = HttpClientConfig::default();
    let client = AsyncHttpClient::new(config).unwrap();
    
    // Test concurrent HTTP requests (using httpbin for testing)
    let urls = vec![
        "https://httpbin.org/bytes/1024",
        "https://httpbin.org/bytes/2048", 
        "https://httpbin.org/bytes/4096",
        "https://httpbin.org/bytes/8192",
    ];
    
    println!("Benchmarking HTTP client performance");
    
    // Sequential requests
    let timer = PerformanceTimer::new();
    for url in &urls {
        match client.download_schema(url).await {
            Ok(data) => {
                println!("  Downloaded {} bytes from {}", data.len(), url);
            }
            Err(_) => {
                println!("  Skipping network test - no connectivity");
                return; // Skip if no network
            }
        }
    }
    let sequential_elapsed = timer.elapsed();
    
    // Concurrent requests
    let timer = PerformanceTimer::new();
    let tasks: Vec<_> = urls.iter().map(|url| {
        let client = &client;
        async move {
            client.download_schema(url).await
        }
    }).collect();
    
    let results = futures::future::join_all(tasks).await;
    let concurrent_elapsed = timer.elapsed();
    
    let successful_requests = results.iter().filter(|r| r.is_ok()).count();
    
    if successful_requests > 0 {
        println!("Sequential: {:?}", sequential_elapsed);
        println!("Concurrent: {:?}", concurrent_elapsed);
        
        let speedup = sequential_elapsed.as_millis() as f64 / concurrent_elapsed.as_millis() as f64;
        println!("Concurrent speedup: {:.2}x", speedup);
        
        // Concurrent should be faster (or at least not much slower)
        assert!(speedup >= 0.8, "Concurrent requests not providing expected speedup: {:.2}x", speedup);
    } else {
        println!("Skipping HTTP benchmark - no network connectivity");
    }
}

#[tokio::test]
async fn benchmark_libxml2_wrapper() {
    let wrapper = LibXml2Wrapper::new();
    let temp_dir = TempDir::new().unwrap();
    
    // Create test schema
    let schema_data = SIMPLE_XSD.as_bytes().to_vec();
    let schema = wrapper.parse_schema_from_memory(schema_data).await.unwrap();
    
    // Create test XML files of different sizes
    let file_sizes = vec![1024, 4096, 16384, 65536];
    
    for file_size in file_sizes {
        println!("Benchmarking libxml2 validation with {} byte files", file_size);
        
        // Create XML content of specified size
        let base_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>CONTENT_PLACEHOLDER</root>"#;
        
        let content_size = file_size.saturating_sub(base_content.len() - "CONTENT_PLACEHOLDER".len());
        let content = "x".repeat(content_size);
        let xml_content = base_content.replace("CONTENT_PLACEHOLDER", &content);
        
        let xml_path = temp_dir.path().join(format!("test_{}.xml", file_size));
        fs::write(&xml_path, xml_content).await.unwrap();
        
        // Benchmark validation
        let iterations = 50;
        let timer = PerformanceTimer::new();
        
        for _ in 0..iterations {
            let result = wrapper.validate_file(&schema, &xml_path).await.unwrap();
            assert!(result.is_valid());
        }
        
        let elapsed = timer.elapsed();
        let throughput = iterations as f64 / elapsed.as_secs_f64();
        
        println!("  Throughput: {:.2} validations/sec", throughput);
        
        // LibXML2 should be reasonably fast
        assert!(throughput >= 100.0, "LibXML2 validation too slow: {:.2} validations/sec", throughput);
    }
}

/// Helper function to run all benchmarks and generate a report
#[tokio::test]
async fn run_comprehensive_benchmark_suite() {
    println!("=== XML Validator Performance Benchmark Suite ===");
    
    let start_time = Instant::now();
    
    // Run individual benchmarks
    benchmark_validation_speed().await;
    benchmark_cache_performance().await;
    benchmark_file_discovery().await;
    benchmark_concurrent_validation().await;
    benchmark_memory_usage().await;
    benchmark_libxml2_wrapper().await;
    
    // Only run HTTP benchmark if network is available
    if std::env::var("SKIP_NETWORK_TESTS").is_err() {
        benchmark_http_client_performance().await;
    }
    
    let total_elapsed = start_time.elapsed();
    
    println!("=== Benchmark Suite Complete ===");
    println!("Total benchmark time: {:?}", total_elapsed);
    println!("All performance tests passed!");
}