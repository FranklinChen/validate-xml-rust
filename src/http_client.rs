use crate::error::ValidationError;
use futures::TryStreamExt;
use reqwest::{Client, Response};
use std::time::Duration;
use tokio::time::{sleep, timeout};

/// Configuration for the HTTP client
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Request timeout in seconds
    pub timeout_seconds: u64,
    /// Number of retry attempts
    pub retry_attempts: u32,
    /// Initial retry delay in milliseconds
    pub retry_delay_ms: u64,
    /// Maximum retry delay in milliseconds (for exponential backoff cap)
    pub max_retry_delay_ms: u64,
    /// User agent string
    pub user_agent: String,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            retry_attempts: 3,
            retry_delay_ms: 1000,
            max_retry_delay_ms: 30000,
            user_agent: format!("validate-xml/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Async HTTP client for downloading remote schemas
pub struct AsyncHttpClient {
    client: Client,
    config: HttpClientConfig,
}

impl AsyncHttpClient {
    /// Create a new async HTTP client with the given configuration
    pub fn new(config: HttpClientConfig) -> Result<Self, ValidationError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent(&config.user_agent)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(ValidationError::from)?;

        Ok(Self { client, config })
    }

    /// Download schema from URL with retry logic and exponential backoff
    pub async fn download_schema(&self, url: &str) -> Result<Vec<u8>, ValidationError> {
        self.download_with_retry(url, 0).await
    }

    /// Download schema with progress tracking using async streams
    pub async fn download_schema_with_progress<F>(
        &self,
        url: &str,
        mut progress_callback: F,
    ) -> Result<Vec<u8>, ValidationError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let response = self.get_response_with_retry(url, 0).await?;

        let total_size = response.content_length();
        let mut downloaded = 0u64;
        let mut buffer = Vec::new();

        // Report initial progress
        progress_callback(0, total_size);

        let mut stream = response.bytes_stream();
        while let Some(chunk) = TryStreamExt::try_next(&mut stream)
            .await
            .map_err(ValidationError::from)?
        {
            buffer.extend_from_slice(&chunk);
            downloaded += chunk.len() as u64;
            progress_callback(downloaded, total_size);
        }

        Ok(buffer)
    }

    /// Internal method to handle retries with exponential backoff
    async fn download_with_retry(
        &self,
        url: &str,
        attempt: u32,
    ) -> Result<Vec<u8>, ValidationError> {
        let response = self.get_response_with_retry(url, attempt).await?;
        let bytes = response.bytes().await.map_err(ValidationError::from)?;
        Ok(bytes.to_vec())
    }

    /// Get response with retry logic
    async fn get_response_with_retry(
        &self,
        url: &str,
        attempt: u32,
    ) -> Result<Response, ValidationError> {
        // Use a loop instead of recursion to avoid boxing issues
        let mut current_attempt = attempt;

        loop {
            match self.make_request(url).await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(response);
                    } else {
                        let status = response.status();
                        let error = ValidationError::HttpStatus {
                            url: url.to_string(),
                            status: status.as_u16(),
                            message: format!(
                                "HTTP {}: {}",
                                status.as_u16(),
                                status.canonical_reason().unwrap_or("Unknown")
                            ),
                        };

                        // Retry on server errors (5xx) but not client errors (4xx)
                        if status.is_server_error() && current_attempt < self.config.retry_attempts
                        {
                            self.wait_before_retry(current_attempt).await;
                            current_attempt += 1;
                            continue;
                        }

                        return Err(error);
                    }
                }
                Err(error) => {
                    if current_attempt < self.config.retry_attempts {
                        // Check if this is a retryable error
                        if self.is_retryable_error(&error) {
                            self.wait_before_retry(current_attempt).await;
                            current_attempt += 1;
                            continue;
                        }
                    }
                    return Err(error);
                }
            }
        }
    }

    /// Make a single HTTP request with timeout
    async fn make_request(&self, url: &str) -> Result<Response, ValidationError> {
        let request_future = self.client.get(url).send();

        timeout(
            Duration::from_secs(self.config.timeout_seconds),
            request_future,
        )
        .await
        .map_err(|_| ValidationError::Timeout {
            url: url.to_string(),
            timeout_seconds: self.config.timeout_seconds,
        })?
        .map_err(ValidationError::from)
    }

    /// Wait before retry with exponential backoff
    async fn wait_before_retry(&self, attempt: u32) {
        let delay_ms = self.config.retry_delay_ms * 2_u64.pow(attempt);
        let capped_delay = delay_ms.min(self.config.max_retry_delay_ms);
        sleep(Duration::from_millis(capped_delay)).await;
    }

    /// Check if an error is retryable
    fn is_retryable_error(&self, error: &ValidationError) -> bool {
        match error {
            ValidationError::Http(reqwest_error) => {
                // Retry on network errors, timeouts, but not on invalid URLs or similar
                reqwest_error.is_timeout()
                    || reqwest_error.is_connect()
                    || reqwest_error.is_request()
            }
            ValidationError::Timeout { .. } => true,
            _ => false,
        }
    }

    /// Get the underlying reqwest client (for advanced usage)
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Get the client configuration
    pub fn config(&self) -> &HttpClientConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_http_client_creation() {
        let config = HttpClientConfig::default();
        let client = AsyncHttpClient::new(config);
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_exponential_backoff_calculation() {
        let config = HttpClientConfig {
            retry_delay_ms: 1000,
            max_retry_delay_ms: 10000,
            ..Default::default()
        };
        let client = AsyncHttpClient::new(config).unwrap();

        // Test that delays increase exponentially but are capped
        let start = std::time::Instant::now();
        client.wait_before_retry(0).await; // Should wait ~1000ms
        let first_delay = start.elapsed();

        let start = std::time::Instant::now();
        client.wait_before_retry(1).await; // Should wait ~2000ms
        let second_delay = start.elapsed();

        let start = std::time::Instant::now();
        client.wait_before_retry(2).await; // Should wait ~4000ms
        let third_delay = start.elapsed();

        // Allow some tolerance for timing
        assert!(first_delay >= Duration::from_millis(900));
        assert!(first_delay <= Duration::from_millis(1100));

        assert!(second_delay >= Duration::from_millis(1900));
        assert!(second_delay <= Duration::from_millis(2100));

        assert!(third_delay >= Duration::from_millis(3900));
        assert!(third_delay <= Duration::from_millis(4100));
    }

    #[tokio::test]
    async fn test_retryable_error_detection() {
        let config = HttpClientConfig::default();
        let client = AsyncHttpClient::new(config).unwrap();

        // Test timeout error is retryable
        let timeout_error = ValidationError::Timeout {
            url: "http://example.com".to_string(),
            timeout_seconds: 30,
        };
        assert!(client.is_retryable_error(&timeout_error));

        // Test non-retryable error
        let parse_error = ValidationError::SchemaParsing {
            url: "http://example.com".to_string(),
            details: "Invalid XML".to_string(),
        };
        assert!(!client.is_retryable_error(&parse_error));
    }

    #[tokio::test]
    async fn test_progress_callback() {
        let config = HttpClientConfig::default();
        let _client = AsyncHttpClient::new(config).unwrap();

        let progress_calls = Arc::new(Mutex::new(Vec::new()));
        let progress_calls_clone = progress_calls.clone();

        let progress_callback = move |downloaded: u64, total: Option<u64>| {
            let calls = progress_calls_clone.clone();
            tokio::spawn(async move {
                calls.lock().await.push((downloaded, total));
            });
        };

        // This test would need a mock server to work properly
        // For now, we just test that the callback mechanism works
        progress_callback(0, Some(1000));
        progress_callback(500, Some(1000));
        progress_callback(1000, Some(1000));

        // Give async tasks time to complete
        tokio::time::sleep(Duration::from_millis(10)).await;

        let calls = progress_calls.lock().await;
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], (0, Some(1000)));
        assert_eq!(calls[1], (500, Some(1000)));
        assert_eq!(calls[2], (1000, Some(1000)));
    }
}
