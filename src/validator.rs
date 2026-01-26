//! Hybrid Async/Sync Validation Engine
//!
//! This module provides a high-performance validation engine using a hybrid architecture:
//! - **Async I/O**: File discovery, schema loading, HTTP downloads, and caching
//! - **Sync CPU-bound work**: libxml2 validation (thread-safe, no spawn_blocking overhead)
//! - **Concurrent orchestration**: tokio::spawn creates parallel validation tasks
//! - **Bounded concurrency**: Semaphore limits prevent resource exhaustion
//!
//! The hybrid design maximizes throughput by avoiding spawn_blocking for CPU-bound
//! libxml2 operations, enabling true parallel validation across multiple cores.

use futures::future::try_join_all;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cache::SchemaCache;
use crate::error::{Result, ValidationError};
use crate::file_discovery::FileDiscovery;
use crate::http_client::AsyncHttpClient;
use crate::libxml2::{LibXml2Wrapper, ValidationResult};
use crate::schema_loader::SchemaLoader;

/// Validation configuration
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationConfig {
    /// Number of concurrent validation threads
    pub max_concurrent_validations: usize,
    /// Timeout for validation operations
    pub validation_timeout: Duration,
    /// Stop validation on first error
    pub fail_fast: bool,
    /// Show progress indicators
    pub show_progress: bool,
    /// Collect performance metrics
    pub collect_metrics: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_validations: num_cpus::get(),
            validation_timeout: Duration::from_secs(30),
            fail_fast: false,
            show_progress: false,
            collect_metrics: true,
        }
    }
}

/// Status of a single file validation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// File validated successfully
    Valid,
    /// File failed validation with schema violations
    Invalid { error_count: i32 },
    /// Internal error occurred during validation
    Error { message: String },
    /// File was skipped (e.g., no schema found)
    Skipped { reason: String },
}

impl ValidationStatus {
    /// Check if the validation was successful
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationStatus::Valid)
    }

    /// Check if the validation failed due to schema violations
    pub fn is_invalid(&self) -> bool {
        matches!(self, ValidationStatus::Invalid { .. })
    }

    /// Check if an error occurred
    pub fn is_error(&self) -> bool {
        matches!(self, ValidationStatus::Error { .. })
    }

    /// Check if the file was skipped
    pub fn is_skipped(&self) -> bool {
        matches!(self, ValidationStatus::Skipped { .. })
    }
}

impl From<ValidationResult> for ValidationStatus {
    fn from(result: ValidationResult) -> Self {
        match result {
            ValidationResult::Valid => ValidationStatus::Valid,
            ValidationResult::Invalid { error_count, .. } => {
                ValidationStatus::Invalid { error_count }
            }
            ValidationResult::InternalError { code } => ValidationStatus::Error {
                message: format!("LibXML2 internal error: {}", code),
            },
        }
    }
}

/// Result of validating a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileValidationResult {
    /// Path to the validated file
    pub path: PathBuf,
    /// Validation status
    pub status: ValidationStatus,
    /// Schema URL used for validation
    pub schema_url: Option<String>,
    /// Duration of validation
    pub duration: Duration,
    /// Error details if validation failed
    pub error_details: Vec<String>,
}

impl FileValidationResult {
    /// Create a new successful validation result
    pub fn valid(path: PathBuf, schema_url: String, duration: Duration) -> Self {
        Self {
            path,
            status: ValidationStatus::Valid,
            schema_url: Some(schema_url),
            duration,
            error_details: Vec::new(),
        }
    }

    /// Create a new invalid validation result
    pub fn invalid(
        path: PathBuf,
        schema_url: String,
        error_count: i32,
        duration: Duration,
        error_details: Vec<String>,
    ) -> Self {
        Self {
            path,
            status: ValidationStatus::Invalid { error_count },
            schema_url: Some(schema_url),
            duration,
            error_details,
        }
    }

    /// Create a new error validation result
    pub fn error(path: PathBuf, error: ValidationError, duration: Duration) -> Self {
        Self {
            path,
            status: ValidationStatus::Error {
                message: error.to_string(),
            },
            schema_url: None,
            duration,
            error_details: vec![error.to_string()],
        }
    }

    /// Create a new skipped validation result
    pub fn skipped(path: PathBuf, reason: String, duration: Duration) -> Self {
        Self {
            path,
            status: ValidationStatus::Skipped {
                reason: reason.clone(),
            },
            schema_url: None,
            duration,
            error_details: vec![reason],
        }
    }
}

/// Progress update for validation
#[derive(Debug, Clone)]
pub struct ValidationProgress {
    /// File currently being processed
    pub current_file: Option<PathBuf>,
    /// Number of files completed
    pub completed: usize,
    /// Total number of files to process
    pub total: usize,
    /// Current phase of validation
    pub phase: ValidationPhase,
}

/// Phase of validation process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationPhase {
    /// Discovering files
    Discovery,
    /// Loading schemas
    SchemaLoading,
    /// Validating files
    Validation,
    /// Aggregating results
    Aggregation,
    /// Complete
    Complete,
}

/// Performance metrics for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Total validation duration
    pub total_duration: Duration,
    /// File discovery duration
    pub discovery_duration: Duration,
    /// Schema loading duration
    pub schema_loading_duration: Duration,
    /// Validation duration
    pub validation_duration: Duration,
    /// Average time per file
    pub average_time_per_file: Duration,
    /// Files processed per second
    pub throughput_files_per_second: f64,
    /// Peak memory usage in MB
    pub peak_memory_mb: u64,
    /// Cache hit rate percentage
    pub cache_hit_rate: f64,
    /// Number of concurrent validations
    pub concurrent_validations: usize,
    /// Schema cache statistics
    pub schema_cache_stats: SchemaCacheStats,
}

/// Schema cache statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchemaCacheStats {
    /// Number of cache hits
    pub hits: usize,
    /// Number of cache misses
    pub misses: usize,
    /// Number of schemas loaded
    pub schemas_loaded: usize,
    /// Total cache size in bytes
    pub cache_size_bytes: u64,
}

/// Aggregated results of validating multiple files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResults {
    /// Total number of files processed
    pub total_files: usize,
    /// Number of valid files
    pub valid_files: usize,
    /// Number of invalid files
    pub invalid_files: usize,
    /// Number of files with errors
    pub error_files: usize,
    /// Number of skipped files
    pub skipped_files: usize,
    /// Total duration of validation
    pub total_duration: Duration,
    /// Average duration per file
    pub average_duration: Duration,
    /// Individual file results
    pub file_results: Vec<FileValidationResult>,
    /// Schemas used during validation
    pub schemas_used: Vec<String>,
    /// Performance metrics
    pub performance_metrics: PerformanceMetrics,
}

impl ValidationResults {
    /// Aggregate individual file results into summary
    pub fn aggregate(file_results: Vec<FileValidationResult>) -> Self {
        let total_files = file_results.len();
        let mut valid_files = 0;
        let mut invalid_files = 0;
        let mut error_files = 0;
        let mut skipped_files = 0;
        let mut total_duration = Duration::ZERO;
        let mut schemas_used = std::collections::HashSet::new();

        for result in &file_results {
            match result.status {
                ValidationStatus::Valid => valid_files += 1,
                ValidationStatus::Invalid { .. } => invalid_files += 1,
                ValidationStatus::Error { .. } => error_files += 1,
                ValidationStatus::Skipped { .. } => skipped_files += 1,
            }

            total_duration += result.duration;

            if let Some(ref schema_url) = result.schema_url {
                schemas_used.insert(schema_url.clone());
            }
        }

        let average_duration = if total_files > 0 {
            total_duration / total_files as u32
        } else {
            Duration::ZERO
        };

        // Create default performance metrics
        let performance_metrics = PerformanceMetrics {
            total_duration,
            discovery_duration: Duration::ZERO,
            schema_loading_duration: Duration::ZERO,
            validation_duration: total_duration,
            average_time_per_file: average_duration,
            throughput_files_per_second: if total_duration.as_secs_f64() > 0.0 {
                total_files as f64 / total_duration.as_secs_f64()
            } else {
                0.0
            },
            peak_memory_mb: 0,
            cache_hit_rate: 0.0,
            concurrent_validations: 1,
            schema_cache_stats: SchemaCacheStats {
                hits: 0,
                misses: 0,
                schemas_loaded: schemas_used.len(),
                cache_size_bytes: 0,
            },
        };

        Self {
            total_files,
            valid_files,
            invalid_files,
            error_files,
            skipped_files,
            total_duration,
            average_duration,
            file_results,
            schemas_used: schemas_used.into_iter().collect(),
            performance_metrics,
        }
    }

    /// Create results with detailed performance metrics
    pub fn with_metrics(
        file_results: Vec<FileValidationResult>,
        performance_metrics: PerformanceMetrics,
    ) -> Self {
        let mut results = Self::aggregate(file_results);
        results.performance_metrics = performance_metrics;
        results
    }

    /// Check if all files validated successfully
    pub fn all_valid(&self) -> bool {
        self.valid_files == self.total_files && self.total_files > 0
    }

    /// Check if any files had validation errors
    pub fn has_errors(&self) -> bool {
        self.error_files > 0 || self.invalid_files > 0
    }

    /// Get success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.valid_files as f64 / self.total_files as f64) * 100.0
        }
    }
}

/// Progress callback type for validation updates
pub type ProgressCallback = Arc<dyn Fn(ValidationProgress) + Send + Sync>;

/// Hybrid async/sync validation engine for high-performance XML validation
///
/// Orchestrates validation using a hybrid architecture:
/// - **Async operations**: File discovery, schema loading/caching, HTTP downloads
/// - **Sync operations**: libxml2 validation (CPU-bound, thread-safe, runs directly in tokio tasks)
/// - **Concurrency**: Semaphore-bounded tokio::spawn tasks for parallel validation
/// - **Result aggregation**: futures::try_join_all collects all validation results
///
/// This design enables true parallel validation across CPU cores without spawn_blocking overhead.
pub struct ValidationEngine {
    schema_loader: Arc<SchemaLoader>,
    libxml2_wrapper: Arc<LibXml2Wrapper>,
    config: ValidationConfig,
}

impl ValidationEngine {
    /// Create a new validation engine
    pub fn new(
        schema_cache: Arc<SchemaCache>,
        http_client: AsyncHttpClient,
        config: ValidationConfig,
    ) -> Result<Self> {
        let schema_loader = Arc::new(SchemaLoader::new(schema_cache, http_client)?);
        let libxml2_wrapper = Arc::new(LibXml2Wrapper::new());

        Ok(Self {
            schema_loader,
            libxml2_wrapper,
            config,
        })
    }

    /// Validate XML files at a path using fully async operations with comprehensive workflow
    pub async fn validate_path(
        &self,
        path: &Path,
        file_discovery: &FileDiscovery,
    ) -> Result<ValidationResults> {
        self.validate_path_with_progress(path, file_discovery, None)
            .await
    }

    /// Validate XML files at a path (directory or file) with progress tracking
    pub async fn validate_path_with_progress(
        &self,
        path: &Path,
        file_discovery: &FileDiscovery,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<ValidationResults> {
        let workflow_start = Instant::now();
        let mut performance_metrics = PerformanceMetrics {
            total_duration: Duration::ZERO,
            discovery_duration: Duration::ZERO,
            schema_loading_duration: Duration::ZERO,
            validation_duration: Duration::ZERO,
            average_time_per_file: Duration::ZERO,
            throughput_files_per_second: 0.0,
            peak_memory_mb: 0,
            cache_hit_rate: 0.0,
            concurrent_validations: self.config.max_concurrent_validations,
            schema_cache_stats: SchemaCacheStats::default(),
        };

        // Phase 1: File Discovery
        let discovery_start = Instant::now();
        if let Some(ref callback) = progress_callback {
            callback(ValidationProgress {
                current_file: None,
                completed: 0,
                total: 0,
                phase: ValidationPhase::Discovery,
            });
        }

        let files = file_discovery.discover_files(path).await?;
        performance_metrics.discovery_duration = discovery_start.elapsed();

        if files.is_empty() {
            performance_metrics.total_duration = workflow_start.elapsed();
            return Ok(ValidationResults::with_metrics(
                Vec::new(),
                performance_metrics,
            ));
        }

        // Phase 2: Schema Loading and Validation
        let validation_start = Instant::now();
        if let Some(ref callback) = progress_callback {
            callback(ValidationProgress {
                current_file: None,
                completed: 0,
                total: files.len(),
                phase: ValidationPhase::SchemaLoading,
            });
        }

        // Validate files with progress tracking
        let results = self
            .validate_files_with_progress(files, progress_callback.clone())
            .await?;
        performance_metrics.validation_duration = validation_start.elapsed();

        // Phase 3: Result Aggregation
        if let Some(ref callback) = progress_callback {
            callback(ValidationProgress {
                current_file: None,
                completed: results.len(),
                total: results.len(),
                phase: ValidationPhase::Aggregation,
            });
        }

        // Collect cache statistics if available
        if let Ok(cache_stats) = self.collect_cache_statistics().await {
            performance_metrics.schema_cache_stats = cache_stats;
        }

        // Calculate final metrics
        performance_metrics.total_duration = workflow_start.elapsed();
        performance_metrics.average_time_per_file = if !results.is_empty() {
            performance_metrics.validation_duration / results.len() as u32
        } else {
            Duration::ZERO
        };
        performance_metrics.throughput_files_per_second =
            if performance_metrics.total_duration.as_secs_f64() > 0.0 {
                results.len() as f64 / performance_metrics.total_duration.as_secs_f64()
            } else {
                0.0
            };

        // Collect memory usage if metrics are enabled
        if self.config.collect_metrics {
            performance_metrics.peak_memory_mb = self.get_peak_memory_usage().await;
        }

        let final_results = ValidationResults::with_metrics(results, performance_metrics);

        // Phase 4: Complete
        if let Some(ref callback) = progress_callback {
            callback(ValidationProgress {
                current_file: None,
                completed: final_results.total_files,
                total: final_results.total_files,
                phase: ValidationPhase::Complete,
            });
        }

        Ok(final_results)
    }

    /// Validate a list of files using concurrent async operations
    pub async fn validate_files(&self, files: Vec<PathBuf>) -> Result<Vec<FileValidationResult>> {
        self.validate_files_with_progress(files, None).await
    }

    /// Validate a list of files with progress tracking
    pub async fn validate_files_with_progress(
        &self,
        files: Vec<PathBuf>,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<Vec<FileValidationResult>> {
        if files.is_empty() {
            return Ok(Vec::new());
        }

        let total_files = files.len();
        let completed = Arc::new(AtomicUsize::new(0));

        // Create a semaphore to limit concurrent validations
        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.max_concurrent_validations,
        ));

        // Create validation tasks for each file
        let validation_tasks: Vec<_> = files
            .into_iter()
            .map(|file_path| {
                let schema_loader = Arc::clone(&self.schema_loader);
                let libxml2_wrapper = Arc::clone(&self.libxml2_wrapper);
                let semaphore = Arc::clone(&semaphore);
                let timeout = self.config.validation_timeout;
                let progress_callback = progress_callback.clone();
                let completed = Arc::clone(&completed);

                tokio::spawn(async move {
                    // Acquire semaphore permit to limit concurrency
                    let _permit = semaphore.acquire().await.map_err(|_| {
                        ValidationError::Config(
                            "Failed to acquire validation semaphore".to_string(),
                        )
                    })?;

                    // Validate single file with timeout
                    let result = tokio::time::timeout(
                        timeout,
                        Self::validate_single_file_internal(
                            file_path.clone(),
                            schema_loader,
                            libxml2_wrapper,
                        ),
                    )
                    .await;

                    let validation_result = match result {
                        Ok(validation_result) => validation_result,
                        Err(_) => FileValidationResult::error(
                            file_path.clone(),
                            ValidationError::Config(format!(
                                "Validation timeout after {:?}",
                                timeout
                            )),
                            timeout,
                        ),
                    };

                    // Update progress
                    let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(ref callback) = progress_callback {
                        callback(ValidationProgress {
                            current_file: Some(file_path),
                            completed: done,
                            total: total_files,
                            phase: ValidationPhase::Validation,
                        });
                    }

                    Ok::<FileValidationResult, ValidationError>(validation_result)
                })
            })
            .collect();

        // Collect all results
        let task_results =
            try_join_all(validation_tasks)
                .await
                .map_err(|e| ValidationError::Concurrency {
                    details: format!("Task join error: {}", e),
                })?;

        // Extract FileValidationResult from Result<FileValidationResult, ValidationError>
        let mut file_results = Vec::with_capacity(task_results.len());
        for result in task_results {
            file_results.push(result?);
        }

        Ok(file_results)
    }

    /// Validate a single file (internal implementation)
    async fn validate_single_file_internal(
        file_path: PathBuf,
        schema_loader: Arc<SchemaLoader>,
        libxml2_wrapper: Arc<LibXml2Wrapper>,
    ) -> FileValidationResult {
        let start_time = Instant::now();

        // RE-IMPLEMENTATION with correct scope capture
        // We need to resolve the reference first.
        let schema_ref = match schema_loader
            .extractor()
            .extract_schema_urls(&file_path)
            .await
        {
            Ok(refs) => match refs.into_iter().next() {
                Some(r) => r,
                None => {
                    return FileValidationResult::skipped(
                        file_path,
                        "No schema URL found in XML file".to_string(),
                        start_time.elapsed(),
                    );
                }
            },
            Err(ValidationError::SchemaUrlNotFound { .. }) => {
                return FileValidationResult::skipped(
                    file_path,
                    "No schema URL found in XML file".to_string(),
                    start_time.elapsed(),
                );
            }
            Err(e) => return FileValidationResult::error(file_path, e, start_time.elapsed()),
        };

        let target_url = schema_ref.url.clone();
        let loader_clone = schema_loader.clone();
        let wrapper_clone = libxml2_wrapper.clone();

        let schema_ptr = match schema_loader
            .cache()
            .parsed()
            .get_or_load(target_url.clone(), || async move {
                // 1. Load Bytes (Async)
                let cached_bytes = loader_clone.load_schema(&schema_ref).await?;

                // 2. Parse Bytes (Blocking)
                // Libxml2 parsing is CPU intensive and blocking.
                // We MUST offload this to a blocking thread to avoid starving the async runtime.
                let data = cached_bytes.data.clone();
                let wrapper = wrapper_clone;

                let ptr =
                    tokio::task::spawn_blocking(move || wrapper.parse_schema_from_memory(&data))
                        .await
                        .map_err(|e| ValidationError::Concurrency {
                            details: e.to_string(),
                        })??;

                Ok(Arc::new(ptr))
            })
            .await
        {
            Ok(ptr) => ptr,
            Err(e) => return FileValidationResult::error(file_path, e, start_time.elapsed()),
        };

        // Step 3: Validate File (Blocking)
        // Validation is also CPU intensive and blocking.
        // While libxml2 uses the file system, it blocks the thread.
        // Offloading this allows the async runtime to process other I/O events (like downloads).
        let validate_wrapper = libxml2_wrapper.clone();
        let validate_path = file_path.clone();
        let validate_ptr = schema_ptr; // Arc clone is cheap

        let validation_result = tokio::task::spawn_blocking(move || {
            validate_wrapper.validate_file(&validate_ptr, &validate_path)
        })
        .await;

        let duration = start_time.elapsed();

        match validation_result {
            Ok(Ok(result)) => match result {
                ValidationResult::Valid => {
                    FileValidationResult::valid(file_path, target_url, duration)
                }
                ValidationResult::Invalid {
                    error_count,
                    errors,
                } => FileValidationResult::invalid(
                    file_path,
                    target_url,
                    error_count,
                    duration,
                    errors,
                ),
                ValidationResult::InternalError { code } => FileValidationResult::error(
                    file_path,
                    ValidationError::LibXml2Internal {
                        details: format!("Internal error code: {}", code),
                    },
                    duration,
                ),
            },
            Ok(Err(e)) => FileValidationResult::error(file_path, e.into(), duration),
            Err(e) => FileValidationResult::error(
                file_path,
                ValidationError::Concurrency {
                    details: format!("Join error: {}", e),
                },
                duration,
            ),
        }
    }

    /// Validate a single file (public interface)
    pub async fn validate_single_file(&self, file_path: &Path) -> Result<FileValidationResult> {
        let result = Self::validate_single_file_internal(
            file_path.to_path_buf(),
            Arc::clone(&self.schema_loader),
            Arc::clone(&self.libxml2_wrapper),
        )
        .await;

        Ok(result)
    }

    /// Get the schema loader for direct access
    pub fn schema_loader(&self) -> &Arc<SchemaLoader> {
        &self.schema_loader
    }

    /// Get the libxml2 wrapper for direct access
    pub fn libxml2_wrapper(&self) -> &Arc<LibXml2Wrapper> {
        &self.libxml2_wrapper
    }

    /// Get the validation configuration
    pub fn config(&self) -> &ValidationConfig {
        &self.config
    }

    /// Collect cache statistics for performance metrics
    async fn collect_cache_statistics(&self) -> Result<SchemaCacheStats> {
        // Get statistics from the schema loader's cache
        let cache = self.schema_loader.cache();
        match cache.stats().await {
            Ok(stats) => Ok(SchemaCacheStats {
                // Note: moka cache doesn't track hits/misses directly,
                // so we report the entry counts as a proxy for loaded schemas
                hits: 0,
                misses: 0,
                schemas_loaded: stats.memory.entry_count as usize,
                cache_size_bytes: stats.disk.total_size,
            }),
            Err(_) => Ok(SchemaCacheStats::default()),
        }
    }

    /// Get peak memory usage in MB
    async fn get_peak_memory_usage(&self) -> u64 {
        // This is a placeholder implementation
        // In a real implementation, you might use system APIs or memory profiling
        #[cfg(target_os = "linux")]
        {
            if let Ok(status) = tokio::fs::read_to_string("/proc/self/status").await {
                for line in status.lines() {
                    if line.starts_with("VmPeak:") {
                        if let Some(kb_str) = line.split_whitespace().nth(1) {
                            if let Ok(kb) = kb_str.parse::<u64>() {
                                return kb / 1024; // Convert KB to MB
                            }
                        }
                    }
                }
            }
        }

        // Fallback: estimate based on process memory
        0
    }

    /// Create a comprehensive validation workflow coordinator
    pub async fn run_comprehensive_validation(
        &self,
        path: &Path,
        file_discovery: &FileDiscovery,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<ValidationResults> {
        // This is the main entry point for the comprehensive validation workflow
        // It coordinates all components and provides detailed progress tracking
        self.validate_path_with_progress(path, file_discovery, progress_callback)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use crate::http_client::HttpClientConfig;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn create_test_validation_engine() -> (ValidationEngine, TempDir) {
        let temp_dir = TempDir::new().unwrap();

        // Create cache
        let cache_config = CacheConfig {
            directory: temp_dir.path().join("cache"),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        let cache = Arc::new(SchemaCache::new(cache_config));

        // Create HTTP client
        let http_config = HttpClientConfig::default();
        let http_client = AsyncHttpClient::new(http_config).unwrap();

        // Create validation config
        let validation_config = ValidationConfig {
            max_concurrent_validations: 2, // Small number for testing
            validation_timeout: Duration::from_secs(5),
            fail_fast: false,
            show_progress: false,
            collect_metrics: true,
        };

        let engine = ValidationEngine::new(cache, http_client, validation_config).unwrap();
        (engine, temp_dir)
    }

    fn create_test_xml_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        file.flush().unwrap();
        file
    }

    #[tokio::test]
    async fn test_validation_engine_creation() {
        let (_engine, _temp_dir) = create_test_validation_engine();
        // Should not panic
    }

    #[tokio::test]
    async fn test_validation_status_predicates() {
        assert!(ValidationStatus::Valid.is_valid());
        assert!(!ValidationStatus::Valid.is_invalid());
        assert!(!ValidationStatus::Valid.is_error());
        assert!(!ValidationStatus::Valid.is_skipped());

        let invalid = ValidationStatus::Invalid { error_count: 1 };
        assert!(!invalid.is_valid());
        assert!(invalid.is_invalid());
        assert!(!invalid.is_error());
        assert!(!invalid.is_skipped());

        let error = ValidationStatus::Error {
            message: "test".to_string(),
        };
        assert!(!error.is_valid());
        assert!(!error.is_invalid());
        assert!(error.is_error());
        assert!(!error.is_skipped());

        let skipped = ValidationStatus::Skipped {
            reason: "test".to_string(),
        };
        assert!(!skipped.is_valid());
        assert!(!skipped.is_invalid());
        assert!(!skipped.is_error());
        assert!(skipped.is_skipped());
    }

    #[tokio::test]
    async fn test_file_validation_result_constructors() {
        let path = PathBuf::from("test.xml");
        let duration = Duration::from_millis(100);

        let valid_result = FileValidationResult::valid(
            path.clone(),
            "http://example.com/schema.xsd".to_string(),
            duration,
        );
        assert!(valid_result.status.is_valid());
        assert_eq!(
            valid_result.schema_url,
            Some("http://example.com/schema.xsd".to_string())
        );

        let invalid_result = FileValidationResult::invalid(
            path.clone(),
            "http://example.com/schema.xsd".to_string(),
            3,
            duration,
            vec![
                "error1".to_string(),
                "error2".to_string(),
                "error3".to_string(),
            ],
        );
        assert!(invalid_result.status.is_invalid());

        let error_result = FileValidationResult::error(
            path.clone(),
            ValidationError::Config("test error".to_string()),
            duration,
        );
        assert!(error_result.status.is_error());

        let skipped_result =
            FileValidationResult::skipped(path, "no schema found".to_string(), duration);
        assert!(skipped_result.status.is_skipped());
    }

    #[tokio::test]
    async fn test_validation_results_aggregation() {
        let results = vec![
            FileValidationResult::valid(
                PathBuf::from("valid1.xml"),
                "schema1.xsd".to_string(),
                Duration::from_millis(100),
            ),
            FileValidationResult::valid(
                PathBuf::from("valid2.xml"),
                "schema1.xsd".to_string(),
                Duration::from_millis(150),
            ),
            FileValidationResult::invalid(
                PathBuf::from("invalid1.xml"),
                "schema2.xsd".to_string(),
                2,
                Duration::from_millis(200),
                vec![],
            ),
            FileValidationResult::error(
                PathBuf::from("error1.xml"),
                ValidationError::Config("test error".to_string()),
                Duration::from_millis(50),
            ),
            FileValidationResult::skipped(
                PathBuf::from("skipped1.xml"),
                "no schema".to_string(),
                Duration::from_millis(25),
            ),
        ];

        let aggregated = ValidationResults::aggregate(results);

        assert_eq!(aggregated.total_files, 5);
        assert_eq!(aggregated.valid_files, 2);
        assert_eq!(aggregated.invalid_files, 1);
        assert_eq!(aggregated.error_files, 1);
        assert_eq!(aggregated.skipped_files, 1);
        assert_eq!(aggregated.total_duration, Duration::from_millis(525));
        assert_eq!(aggregated.average_duration, Duration::from_millis(105));
        assert_eq!(aggregated.schemas_used.len(), 2);
        assert!(aggregated.schemas_used.contains(&"schema1.xsd".to_string()));
        assert!(aggregated.schemas_used.contains(&"schema2.xsd".to_string()));

        assert!(!aggregated.all_valid());
        assert!(aggregated.has_errors());
        assert_eq!(aggregated.success_rate(), 40.0); // 2/5 * 100
    }

    #[tokio::test]
    async fn test_validation_results_empty() {
        let aggregated = ValidationResults::aggregate(Vec::new());

        assert_eq!(aggregated.total_files, 0);
        assert_eq!(aggregated.valid_files, 0);
        assert_eq!(aggregated.success_rate(), 0.0);
        assert!(!aggregated.all_valid());
        assert!(!aggregated.has_errors());
    }

    #[tokio::test]
    async fn test_validation_results_all_valid() {
        let results = vec![
            FileValidationResult::valid(
                PathBuf::from("valid1.xml"),
                "schema.xsd".to_string(),
                Duration::from_millis(100),
            ),
            FileValidationResult::valid(
                PathBuf::from("valid2.xml"),
                "schema.xsd".to_string(),
                Duration::from_millis(150),
            ),
        ];

        let aggregated = ValidationResults::aggregate(results);

        assert!(aggregated.all_valid());
        assert!(!aggregated.has_errors());
        assert_eq!(aggregated.success_rate(), 100.0);
    }

    #[tokio::test]
    async fn test_validate_files_empty_list() {
        let (engine, _temp_dir) = create_test_validation_engine();

        let results = engine.validate_files(Vec::new()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_validate_single_file_no_schema() {
        let (engine, _temp_dir) = create_test_validation_engine();

        // Create XML file without schema reference
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <element>content</element>
</root>"#;
        let xml_file = create_test_xml_file(xml_content);

        let result = engine.validate_single_file(xml_file.path()).await.unwrap();

        assert!(result.status.is_skipped());
        assert!(result.error_details[0].contains("No schema URL found"));
    }

    #[tokio::test]
    async fn test_validate_single_file_with_local_schema() {
        let (engine, temp_dir) = create_test_validation_engine();

        // Create a simple schema file
        let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="element" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_file = temp_dir.path().join("schema.xsd");
        tokio::fs::write(&schema_file, schema_content)
            .await
            .unwrap();

        // Create XML file that references the local schema
        let xml_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">
    <element>content</element>
</root>"#,
            schema_file.display()
        );
        let xml_file = create_test_xml_file(&xml_content);

        let result = engine.validate_single_file(xml_file.path()).await.unwrap();

        // Should be valid since the XML matches the schema
        assert!(
            result.status.is_valid(),
            "Expected valid result, got: {:?}",
            result
        );
        assert!(result.schema_url.is_some());
    }

    #[tokio::test]
    async fn test_validate_single_file_schema_not_found() {
        let (engine, _temp_dir) = create_test_validation_engine();

        // Create XML file that references a non-existent local schema
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="/nonexistent/schema.xsd">
    <element>content</element>
</root>"#;
        let xml_file = create_test_xml_file(xml_content);

        let result = engine.validate_single_file(xml_file.path()).await.unwrap();

        assert!(result.status.is_error());
        assert!(result.error_details[0].contains("Schema not found"));
    }

    #[tokio::test]
    async fn test_concurrent_validation() {
        let (engine, temp_dir) = create_test_validation_engine();

        // Create a simple schema file
        let schema_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

        let schema_file = temp_dir.path().join("schema.xsd");
        tokio::fs::write(&schema_file, schema_content)
            .await
            .unwrap();

        // Create multiple XML files
        let mut xml_files = Vec::new();
        for i in 0..5 {
            let xml_content = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">content{}</root>"#,
                schema_file.display(),
                i
            );
            let xml_file = temp_dir.path().join(format!("test{}.xml", i));
            tokio::fs::write(&xml_file, xml_content).await.unwrap();
            xml_files.push(xml_file);
        }

        let results = engine.validate_files(xml_files).await.unwrap();

        assert_eq!(results.len(), 5);
        for result in results {
            assert!(
                result.status.is_valid(),
                "Expected valid result, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_validation_timeout() {
        let temp_dir = TempDir::new().unwrap();

        // Create cache
        let cache_config = CacheConfig {
            directory: temp_dir.path().join("cache"),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        let cache = Arc::new(SchemaCache::new(cache_config));

        // Create HTTP client
        let http_config = HttpClientConfig::default();
        let http_client = AsyncHttpClient::new(http_config).unwrap();

        // Create validation config with very short timeout
        let validation_config = ValidationConfig {
            max_concurrent_validations: 1,
            validation_timeout: Duration::from_millis(1), // Very short timeout
            fail_fast: false,
            show_progress: false,
            collect_metrics: true,
        };

        let engine = ValidationEngine::new(cache, http_client, validation_config).unwrap();

        // Create XML file without schema (should be fast, but timeout is so short it might still timeout)
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>content</root>"#;
        let xml_file = create_test_xml_file(xml_content);

        let results = engine
            .validate_files(vec![xml_file.path().to_path_buf()])
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        // Result should either be skipped (no schema) or timeout error
        let result = &results[0];
        assert!(result.status.is_skipped() || result.status.is_error());
    }

    #[tokio::test]
    async fn test_validation_config_default() {
        let config = ValidationConfig::default();

        assert!(config.max_concurrent_validations > 0);
        assert!(config.validation_timeout > Duration::ZERO);
        assert!(!config.fail_fast);
        assert!(!config.show_progress);
    }

    #[tokio::test]
    async fn test_validation_status_from_validation_result() {
        let valid_result = ValidationResult::Valid;
        let status: ValidationStatus = valid_result.into();
        assert!(status.is_valid());

        let invalid_result = ValidationResult::Invalid {
            error_count: 3,
            errors: vec![],
        };
        let status: ValidationStatus = invalid_result.into();
        assert!(status.is_invalid());
        if let ValidationStatus::Invalid { error_count } = status {
            assert_eq!(error_count, 3);
        } else {
            panic!("Expected Invalid status");
        }

        let error_result = ValidationResult::InternalError { code: -1 };
        let status: ValidationStatus = error_result.into();
        assert!(status.is_error());
    }

    #[tokio::test]
    async fn test_engine_accessors() {
        let (engine, _temp_dir) = create_test_validation_engine();

        // Test that we can access the components
        let _schema_loader = engine.schema_loader();
        let _libxml2_wrapper = engine.libxml2_wrapper();
        let config = engine.config();

        assert_eq!(config.max_concurrent_validations, 2);
    }
}
