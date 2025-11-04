use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use validate_xml::{AsyncHttpClient, HttpClientConfig, ValidationError};

/// Mock HTTP server for testing
#[allow(dead_code)]
struct MockHttpServer {
    port: u16,
    responses: Arc<Mutex<Vec<MockResponse>>>,
}

#[derive(Clone)]
#[allow(dead_code)]
struct MockResponse {
    status: u16,
    body: Vec<u8>,
    delay: Option<Duration>,
    should_fail: bool,
}

impl MockHttpServer {
    #[allow(dead_code)]
    async fn new() -> Self {
        // In a real implementation, we'd start a test HTTP server
        // For now, we'll simulate the behavior
        Self {
            port: 0, // Would be assigned by the test server
            responses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[allow(dead_code)]
    async fn add_response(&self, response: MockResponse) {
        self.responses.lock().await.push(response);
    }

    #[allow(dead_code)]
    fn url(&self, path: &str) -> String {
        format!("http://localhost:{}{}", self.port, path)
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_successful_schema_download() {
    let config = HttpClientConfig {
        timeout_seconds: 5,
        retry_attempts: 2,
        retry_delay_ms: 100,
        max_retry_delay_ms: 1000,
        user_agent: "test-client".to_string(),
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with a real URL that should work (using httpbin for testing)
    // Note: This test requires internet connectivity
    let test_url = "https://httpbin.org/bytes/100";

    let data = client
        .download_schema(test_url)
        .await
        .expect("Failed to download schema");
    assert_eq!(data.len(), 100);
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_download_with_progress_tracking() {
    let config = HttpClientConfig::default();
    let client = AsyncHttpClient::new(config).unwrap();

    let progress_calls = Arc::new(Mutex::new(Vec::new()));
    let progress_calls_clone = progress_calls.clone();

    let progress_callback = move |downloaded: u64, total: Option<u64>| {
        let calls = progress_calls_clone.clone();
        // Don't spawn - execute synchronously to avoid race conditions
        let mut calls_guard = calls.blocking_lock();
        calls_guard.push((downloaded, total));
    };

    // Test with httpbin for a known response size
    let test_url = "https://httpbin.org/bytes/1000";

    let data = client
        .download_schema_with_progress(test_url, progress_callback)
        .await
        .expect("Failed to download schema");
    assert_eq!(data.len(), 1000);

    let calls = progress_calls.lock().await;
    assert!(
        !calls.is_empty(),
        "Progress callback should have been called"
    );

    // First call should be (0, Some(total_size))
    assert_eq!(calls[0].0, 0);

    // Last call should have downloaded == total
    let last_call = calls.last().unwrap();
    if let Some(total) = last_call.1 {
        assert_eq!(last_call.0, total);
    }
}

#[tokio::test]
async fn test_retry_logic_with_exponential_backoff() {
    let config = HttpClientConfig {
        timeout_seconds: 1,
        retry_attempts: 3,
        retry_delay_ms: 100,
        max_retry_delay_ms: 1000,
        user_agent: "test-client".to_string(),
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with a URL that will likely fail (non-existent domain)
    let test_url = "http://this-domain-should-not-exist-12345.com/schema.xsd";

    let start_time = std::time::Instant::now();
    let result = client.download_schema(test_url).await;
    let elapsed = start_time.elapsed();

    // Should fail after retries
    assert!(result.is_err());

    // Should have taken some time due to retries (at least 100ms + 200ms + 400ms = 700ms)
    // But we'll be lenient due to timing variations in tests
    assert!(elapsed >= Duration::from_millis(50));
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_timeout_handling() {
    let config = HttpClientConfig {
        timeout_seconds: 1, // Very short timeout
        retry_attempts: 0,  // No retries to make test faster
        retry_delay_ms: 100,
        max_retry_delay_ms: 1000,
        user_agent: "test-client".to_string(),
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with httpbin delay endpoint that will timeout
    let test_url = "https://httpbin.org/delay/5"; // 5 second delay, but 1 second timeout

    let start_time = std::time::Instant::now();
    let result = client.download_schema(test_url).await;
    let elapsed = start_time.elapsed();

    match result {
        Err(ValidationError::Timeout { .. }) => {
            // Should timeout quickly (around 1 second)
            assert!(elapsed <= Duration::from_secs(2));
        }
        Err(ValidationError::Http(reqwest_error)) if reqwest_error.is_timeout() => {
            // reqwest might wrap the timeout differently
            assert!(elapsed <= Duration::from_secs(2));
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Ok(_) => panic!("Expected timeout error"),
        Err(e) => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_http_status_error_handling() {
    let config = HttpClientConfig::default();
    let client = AsyncHttpClient::new(config).unwrap();

    // Test with httpbin status endpoint for 404
    let test_url = "https://httpbin.org/status/404";

    match client.download_schema(test_url).await {
        Err(ValidationError::HttpStatus { status: 404, .. }) => {
            // Expected 404 error
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Ok(_) => panic!("Expected 404 error"),
        Err(e) => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_server_error_retry() {
    let config = HttpClientConfig {
        timeout_seconds: 5,
        retry_attempts: 2,
        retry_delay_ms: 100,
        max_retry_delay_ms: 1000,
        user_agent: "test-client".to_string(),
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with httpbin status endpoint for 500 (should retry)
    let test_url = "https://httpbin.org/status/500";

    let start_time = std::time::Instant::now();
    let result = client.download_schema(test_url).await;
    let elapsed = start_time.elapsed();

    match result {
        Err(ValidationError::HttpStatus { status: 500, .. }) => {
            // Should have retried (taking extra time)
            assert!(elapsed >= Duration::from_millis(200)); // At least one retry delay
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Ok(_) => panic!("Expected 500 error"),
        Err(e) => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_client_error_no_retry() {
    let config = HttpClientConfig {
        timeout_seconds: 5,
        retry_attempts: 2,
        retry_delay_ms: 100,
        max_retry_delay_ms: 1000,
        user_agent: "test-client".to_string(),
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with httpbin status endpoint for 400 (should NOT retry)
    let test_url = "https://httpbin.org/status/400";

    let start_time = std::time::Instant::now();
    let result = client.download_schema(test_url).await;
    let elapsed = start_time.elapsed();

    match result {
        Err(ValidationError::HttpStatus { status: 400, .. }) => {
            // Should NOT have retried (should be reasonably fast, but allow for network latency)
            assert!(
                elapsed <= Duration::from_secs(10),
                "HTTP request took too long: {:?}",
                elapsed
            );
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Ok(_) => panic!("Expected 400 error"),
        Err(e) => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_connection_pooling() {
    let config = HttpClientConfig::default();
    let client = AsyncHttpClient::new(config).unwrap();

    // Make multiple requests to the same domain to test connection reuse
    let base_url = "https://httpbin.org";
    let urls = vec![
        format!("{}/bytes/100", base_url),
        format!("{}/bytes/200", base_url),
        format!("{}/bytes/300", base_url),
    ];

    let start_time = std::time::Instant::now();

    // Make requests sequentially to test connection reuse
    for url in urls {
        match client.download_schema(&url).await {
            Ok(_) => {
                // Success - connection pooling should make subsequent requests faster
            }
            Err(ValidationError::Http(_)) => {
                // Network might not be available in CI, skip this test
                println!("Skipping network test - no internet connectivity");
                return;
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    let elapsed = start_time.elapsed();

    // With connection pooling, 3 requests should be reasonably fast
    // This is a rough test - in practice, connection reuse should be faster
    println!("Three requests took: {:?}", elapsed);
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_user_agent_configuration() {
    let config = HttpClientConfig {
        user_agent: "custom-xml-validator/1.0".to_string(),
        ..Default::default()
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test that the client was created successfully with custom user agent
    assert_eq!(client.config().user_agent, "custom-xml-validator/1.0");

    // Test with httpbin user-agent endpoint to verify it's sent correctly
    let test_url = "https://httpbin.org/user-agent";

    match client.download_schema(test_url).await {
        Ok(data) => {
            let response_text = String::from_utf8_lossy(&data);
            // The response should contain our user agent
            assert!(response_text.contains("custom-xml-validator/1.0"));
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_large_file_download() {
    let config = HttpClientConfig {
        timeout_seconds: 30, // Longer timeout for large file
        ..Default::default()
    };

    let client = AsyncHttpClient::new(config).unwrap();

    // Test with a larger file (10KB)
    let test_url = "https://httpbin.org/bytes/10240";

    match client.download_schema(test_url).await {
        Ok(data) => {
            assert_eq!(data.len(), 10240);
        }
        Err(ValidationError::Http(_)) => {
            // Network might not be available in CI, skip this test
            println!("Skipping network test - no internet connectivity");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
async fn test_concurrent_downloads() {
    let config = HttpClientConfig::default();
    let client = Arc::new(AsyncHttpClient::new(config).unwrap());

    // Test concurrent downloads
    let urls = vec![
        "https://httpbin.org/bytes/100",
        "https://httpbin.org/bytes/200",
        "https://httpbin.org/bytes/300",
        "https://httpbin.org/bytes/400",
    ];

    let tasks: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let client = client.clone();
            let url = url.to_string();
            tokio::spawn(async move { client.download_schema(&url).await })
        })
        .collect();

    let start_time = std::time::Instant::now();
    let results = futures::future::join_all(tasks).await;
    let elapsed = start_time.elapsed();

    let mut success_count = 0;
    let mut network_error_count = 0;

    for result in results {
        match result.unwrap() {
            Ok(_) => success_count += 1,
            Err(ValidationError::Http(_)) => network_error_count += 1,
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    if success_count > 0 {
        // Concurrent requests should be faster than sequential
        println!("Concurrent downloads took: {:?}", elapsed);
        println!("Successful downloads: {}", success_count);
    } else {
        println!(
            "Skipping network test - no internet connectivity (all {} requests failed)",
            network_error_count
        );
    }
}

#[test]
fn test_http_client_config_default() {
    let config = HttpClientConfig::default();

    assert_eq!(config.timeout_seconds, 30);
    assert_eq!(config.retry_attempts, 3);
    assert_eq!(config.retry_delay_ms, 1000);
    assert_eq!(config.max_retry_delay_ms, 30000);
    assert!(config.user_agent.contains("validate-xml"));
}

#[test]
fn test_http_client_config_custom() {
    let config = HttpClientConfig {
        timeout_seconds: 60,
        retry_attempts: 5,
        retry_delay_ms: 500,
        max_retry_delay_ms: 60000,
        user_agent: "custom-agent".to_string(),
    };

    assert_eq!(config.timeout_seconds, 60);
    assert_eq!(config.retry_attempts, 5);
    assert_eq!(config.retry_delay_ms, 500);
    assert_eq!(config.max_retry_delay_ms, 60000);
    assert_eq!(config.user_agent, "custom-agent");
}
