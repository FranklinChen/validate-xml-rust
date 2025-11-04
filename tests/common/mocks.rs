use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use validate_xml::ValidationError;

/// Mock HTTP client for testing network operations without actual network calls
pub struct MockHttpClient {
    responses: Arc<Mutex<HashMap<String, MockHttpResponse>>>,
    request_log: Arc<Mutex<Vec<HttpRequest>>>,
    default_delay: Duration,
}

#[derive(Clone, Debug)]
pub struct MockHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub delay: Option<Duration>,
    pub should_fail: bool,
    pub failure_type: Option<MockFailureType>,
}

#[derive(Clone, Debug)]
pub enum MockFailureType {
    Timeout,
    NetworkError,
    InvalidResponse,
}

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub url: String,
    pub timestamp: std::time::Instant,
}

impl MockHttpClient {
    pub fn new() -> Self {
        Self {
            responses: Arc::new(Mutex::new(HashMap::new())),
            request_log: Arc::new(Mutex::new(Vec::new())),
            default_delay: Duration::from_millis(10),
        }
    }

    pub fn add_response(&self, url: &str, response: MockHttpResponse) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_string(), response);
    }

    pub fn add_success_response(&self, url: &str, body: Vec<u8>) {
        self.add_response(
            url,
            MockHttpResponse {
                status: 200,
                body,
                headers: HashMap::new(),
                delay: None,
                should_fail: false,
                failure_type: None,
            },
        );
    }

    pub fn add_error_response(&self, url: &str, status: u16) {
        self.add_response(
            url,
            MockHttpResponse {
                status,
                body: Vec::new(),
                headers: HashMap::new(),
                delay: None,
                should_fail: false,
                failure_type: None,
            },
        );
    }

    pub fn add_timeout_response(&self, url: &str) {
        self.add_response(
            url,
            MockHttpResponse {
                status: 0,
                body: Vec::new(),
                headers: HashMap::new(),
                delay: Some(Duration::from_secs(60)), // Long delay to simulate timeout
                should_fail: true,
                failure_type: Some(MockFailureType::Timeout),
            },
        );
    }

    pub fn add_network_error_response(&self, url: &str) {
        self.add_response(
            url,
            MockHttpResponse {
                status: 0,
                body: Vec::new(),
                headers: HashMap::new(),
                delay: None,
                should_fail: true,
                failure_type: Some(MockFailureType::NetworkError),
            },
        );
    }

    pub fn get_request_log(&self) -> Vec<HttpRequest> {
        self.request_log.lock().unwrap().clone()
    }

    pub fn clear_request_log(&self) {
        self.request_log.lock().unwrap().clear();
    }

    pub async fn download_schema(&self, url: &str) -> Result<Vec<u8>, ValidationError> {
        // Log the request
        self.request_log.lock().unwrap().push(HttpRequest {
            url: url.to_string(),
            timestamp: std::time::Instant::now(),
        });

        // Get response configuration
        let response = {
            let responses = self.responses.lock().unwrap();
            responses.get(url).cloned()
        };

        if let Some(response) = response {
            // Simulate delay
            let delay = response.delay.unwrap_or(self.default_delay);
            tokio::time::sleep(delay).await;

            // Simulate failure
            if response.should_fail {
                return match response.failure_type {
                    Some(MockFailureType::Timeout) => Err(ValidationError::Timeout {
                        url: url.to_string(),
                        timeout_seconds: delay.as_secs(),
                    }),
                    Some(MockFailureType::NetworkError) => Err(ValidationError::HttpStatus {
                        url: url.to_string(),
                        status: 503,
                        message: "Network error".to_string(),
                    }),
                    Some(MockFailureType::InvalidResponse) => Err(ValidationError::HttpStatus {
                        url: url.to_string(),
                        status: 502,
                        message: "Invalid response".to_string(),
                    }),
                    None => Err(ValidationError::HttpStatus {
                        url: url.to_string(),
                        status: 500,
                        message: "Unknown error".to_string(),
                    }),
                };
            }

            // Return success response
            if response.status == 200 {
                Ok(response.body)
            } else {
                Err(ValidationError::HttpStatus {
                    status: response.status,
                    url: url.to_string(),
                    message: format!("HTTP {}", response.status),
                })
            }
        } else {
            // Default: not found
            Err(ValidationError::HttpStatus {
                status: 404,
                url: url.to_string(),
                message: "Not Found".to_string(),
            })
        }
    }

    pub async fn download_schema_with_progress<F>(
        &self,
        url: &str,
        mut progress_callback: F,
    ) -> Result<Vec<u8>, ValidationError>
    where
        F: FnMut(u64, Option<u64>),
    {
        let result = self.download_schema(url).await?;

        // Simulate progress callbacks
        let total_size = result.len() as u64;
        progress_callback(0, Some(total_size));

        // Simulate chunked progress
        let chunk_size = (total_size / 10).max(1);
        for i in 1..=10 {
            let downloaded = (i * chunk_size).min(total_size);
            progress_callback(downloaded, Some(total_size));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        Ok(result)
    }
}

/// Mock file system for testing file operations without actual file I/O
pub struct MockFileSystem {
    files: Arc<Mutex<HashMap<PathBuf, Vec<u8>>>>,
    directories: Arc<Mutex<HashMap<PathBuf, Vec<PathBuf>>>>,
    operation_log: Arc<Mutex<Vec<FileOperation>>>,
}

#[derive(Clone, Debug)]
pub struct FileOperation {
    pub operation_type: FileOperationType,
    pub path: PathBuf,
    pub timestamp: std::time::Instant,
}

#[derive(Clone, Debug)]
pub enum FileOperationType {
    Read,
    Write,
    Create,
    Delete,
    List,
}

impl MockFileSystem {
    pub fn new() -> Self {
        Self {
            files: Arc::new(Mutex::new(HashMap::new())),
            directories: Arc::new(Mutex::new(HashMap::new())),
            operation_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn add_file(&self, path: PathBuf, content: Vec<u8>) {
        self.files.lock().unwrap().insert(path.clone(), content);

        // Add to parent directory
        if let Some(parent) = path.parent() {
            self.directories
                .lock()
                .unwrap()
                .entry(parent.to_path_buf())
                .or_insert_with(Vec::new)
                .push(path);
        }
    }

    pub fn add_directory(&self, path: PathBuf) {
        self.directories.lock().unwrap().insert(path, Vec::new());
    }

    pub async fn read_file(&self, path: &Path) -> Result<Vec<u8>, ValidationError> {
        self.log_operation(FileOperationType::Read, path);

        let files = self.files.lock().unwrap();
        files.get(path).cloned().ok_or_else(|| {
            ValidationError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found",
            ))
        })
    }

    pub async fn write_file(&self, path: &Path, content: Vec<u8>) -> Result<(), ValidationError> {
        self.log_operation(FileOperationType::Write, path);

        self.files
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), content);
        Ok(())
    }

    pub async fn list_directory(&self, path: &Path) -> Result<Vec<PathBuf>, ValidationError> {
        self.log_operation(FileOperationType::List, path);

        let directories = self.directories.lock().unwrap();
        directories.get(path).cloned().ok_or_else(|| {
            ValidationError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Directory not found",
            ))
        })
    }

    pub fn file_exists(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }

    pub fn directory_exists(&self, path: &Path) -> bool {
        self.directories.lock().unwrap().contains_key(path)
    }

    pub fn get_operation_log(&self) -> Vec<FileOperation> {
        self.operation_log.lock().unwrap().clone()
    }

    pub fn clear_operation_log(&self) {
        self.operation_log.lock().unwrap().clear();
    }

    fn log_operation(&self, operation_type: FileOperationType, path: &Path) {
        self.operation_log.lock().unwrap().push(FileOperation {
            operation_type,
            path: path.to_path_buf(),
            timestamp: std::time::Instant::now(),
        });
    }
}

/// Mock schema cache for testing caching behavior
pub struct MockSchemaCache {
    memory_cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    disk_cache: Arc<Mutex<HashMap<String, (Vec<u8>, std::time::Instant)>>>,
    cache_hits: Arc<Mutex<u64>>,
    cache_misses: Arc<Mutex<u64>>,
    ttl: Duration,
}

impl MockSchemaCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            memory_cache: Arc::new(Mutex::new(HashMap::new())),
            disk_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_hits: Arc::new(Mutex::new(0)),
            cache_misses: Arc::new(Mutex::new(0)),
            ttl,
        }
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        // Try memory cache first
        if let Some(data) = self.memory_cache.lock().unwrap().get(key) {
            *self.cache_hits.lock().unwrap() += 1;
            return Some(data.clone());
        }

        // Try disk cache
        if let Some((data, timestamp)) = self.disk_cache.lock().unwrap().get(key) {
            if timestamp.elapsed() < self.ttl {
                // Move to memory cache
                self.memory_cache
                    .lock()
                    .unwrap()
                    .insert(key.to_string(), data.clone());
                *self.cache_hits.lock().unwrap() += 1;
                return Some(data.clone());
            } else {
                // Expired, remove from disk cache
                self.disk_cache.lock().unwrap().remove(key);
            }
        }

        *self.cache_misses.lock().unwrap() += 1;
        None
    }

    pub async fn set(&self, key: &str, data: Vec<u8>) {
        let now = std::time::Instant::now();

        // Set in both caches
        self.memory_cache
            .lock()
            .unwrap()
            .insert(key.to_string(), data.clone());
        self.disk_cache
            .lock()
            .unwrap()
            .insert(key.to_string(), (data, now));
    }

    pub fn get_stats(&self) -> CacheStats {
        CacheStats {
            hits: *self.cache_hits.lock().unwrap(),
            misses: *self.cache_misses.lock().unwrap(),
            memory_entries: self.memory_cache.lock().unwrap().len(),
            disk_entries: self.disk_cache.lock().unwrap().len(),
        }
    }

    pub async fn cleanup_expired(&self) {
        let now = std::time::Instant::now();
        self.disk_cache
            .lock()
            .unwrap()
            .retain(|_, (_, timestamp)| now.duration_since(*timestamp) < self.ttl);
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub memory_entries: usize,
    pub disk_entries: usize,
}

/// Mock validation engine for testing validation logic
pub struct MockValidationEngine {
    validation_results: Arc<Mutex<HashMap<PathBuf, MockValidationResult>>>,
    validation_delay: Duration,
    call_count: Arc<Mutex<u64>>,
}

#[derive(Clone, Debug)]
pub struct MockValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub processing_time: Duration,
}

impl MockValidationEngine {
    pub fn new() -> Self {
        Self {
            validation_results: Arc::new(Mutex::new(HashMap::new())),
            validation_delay: Duration::from_millis(10),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    pub fn set_validation_result(&self, path: PathBuf, result: MockValidationResult) {
        self.validation_results.lock().unwrap().insert(path, result);
    }

    pub fn set_validation_delay(&mut self, delay: Duration) {
        self.validation_delay = delay;
    }

    pub async fn validate_file(
        &self,
        path: &Path,
    ) -> Result<MockValidationResult, ValidationError> {
        *self.call_count.lock().unwrap() += 1;

        // Simulate processing time
        tokio::time::sleep(self.validation_delay).await;

        // Return configured result or default
        let results = self.validation_results.lock().unwrap();
        Ok(results.get(path).cloned().unwrap_or(MockValidationResult {
            is_valid: true,
            errors: Vec::new(),
            processing_time: self.validation_delay,
        }))
    }

    pub fn get_call_count(&self) -> u64 {
        *self.call_count.lock().unwrap()
    }

    pub fn reset_call_count(&self) {
        *self.call_count.lock().unwrap() = 0;
    }
}

/// Test utilities for creating mock data
pub struct MockDataBuilder;

impl MockDataBuilder {
    pub fn create_valid_xml_schema_pair() -> (String, String) {
        let schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="element" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="test.xsd">
    <element>Valid content</element>
</root>"#;

        (schema.to_string(), xml.to_string())
    }

    pub fn create_invalid_xml_for_schema(schema_url: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">
    <invalid_element>This doesn't match the schema</invalid_element>
</root>"#,
            schema_url
        )
    }

    pub fn create_malformed_xml() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <unclosed_element>
    <another>content</another>
</root>"#
            .to_string()
    }

    pub fn create_large_xml_content(size_kb: usize) -> String {
        let base = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>CONTENT_PLACEHOLDER</root>"#;

        let content_size =
            (size_kb * 1024).saturating_sub(base.len() - "CONTENT_PLACEHOLDER".len());
        let content = "x".repeat(content_size);

        base.replace("CONTENT_PLACEHOLDER", &content)
    }
}
