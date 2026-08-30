//! Hybrid Async/Sync Validation Engine
//!
//! This module provides a high-performance validation engine using a hybrid architecture:
//! - **Async I/O**: File discovery, schema loading, HTTP downloads, and caching
//! - **Sync CPU-bound work**: schema parsing and XML validation on blocking threads
//! - **Concurrent orchestration**: a bounded future set drives parallel validation tasks
//! - **Bounded concurrency**: only the configured number of files are in flight
//!
//! Blocking work is kept off Tokio's async worker threads while the scheduler bounds
//! total validation concurrency and can stop scheduling promptly in fail-fast mode.

use futures::stream::{FuturesUnordered, StreamExt};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::backend::{self, SchemaValidation};
use crate::cache::SchemaCache;
use crate::error::{Result, ValidationError};
use crate::file_discovery::FileDiscovery;
use crate::http_client::AsyncHttpClient;
use crate::schema_loader::SchemaLoader;

/// Validation configuration
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationConfig {
    /// Number of concurrent validation threads
    max_concurrent_validations: NonZeroUsize,
    /// Deadline for reporting a validation result. A libxml2 call that has
    /// already started cannot be interrupted; it retains its concurrency slot
    /// until returning even if this deadline has elapsed.
    validation_timeout: Duration,
    /// Stop scheduling new files on first error, then drain admitted work.
    fail_fast: bool,
    /// Collect performance metrics
    collect_metrics: bool,
    /// Override schema path (skip schema extraction from XML)
    schema_override: Option<PathBuf>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            max_concurrent_validations: std::thread::available_parallelism()
                .unwrap_or(NonZeroUsize::MIN),
            validation_timeout: Duration::from_secs(30),
            fail_fast: false,
            collect_metrics: true,
            schema_override: None,
        }
    }
}

impl ValidationConfig {
    /// Construct a validated configuration. Zero concurrency or timeout is rejected.
    pub fn new(max_concurrent_validations: usize, validation_timeout: Duration) -> Result<Self> {
        let max_concurrent_validations =
            NonZeroUsize::new(max_concurrent_validations).ok_or_else(|| {
                ValidationError::Config("validation concurrency must be non-zero".into())
            })?;
        if validation_timeout.is_zero() {
            return Err(ValidationError::Config(
                "validation timeout must be non-zero".into(),
            ));
        }
        Ok(Self {
            max_concurrent_validations,
            validation_timeout,
            ..Self::default()
        })
    }

    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    pub fn with_metrics(mut self, collect_metrics: bool) -> Self {
        self.collect_metrics = collect_metrics;
        self
    }

    pub fn with_schema_override(mut self, schema_override: Option<PathBuf>) -> Self {
        self.schema_override = schema_override;
        self
    }

    pub fn max_concurrent_validations(&self) -> NonZeroUsize {
        self.max_concurrent_validations
    }

    pub fn validation_timeout(&self) -> Duration {
        self.validation_timeout
    }

    pub fn fail_fast(&self) -> bool {
        self.fail_fast
    }

    pub fn collect_metrics(&self) -> bool {
        self.collect_metrics
    }

    pub fn schema_override(&self) -> Option<&Path> {
        self.schema_override.as_deref()
    }
}

/// A non-empty collection of schema violations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violations {
    first: String,
    rest: Vec<String>,
}

impl Violations {
    fn from_vec(mut values: Vec<String>) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let first = values.remove(0);
        Some(Self {
            first,
            rest: values,
        })
    }

    pub fn len(&self) -> usize {
        1 + self.rest.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.first.as_str()).chain(self.rest.iter().map(String::as_str))
    }
}

/// Mutually exclusive outcome of validating one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationOutcome {
    Valid {
        schema: String,
    },
    Invalid {
        schema: String,
        violations: Violations,
    },
    Error {
        message: String,
    },
    Skipped {
        reason: String,
    },
}

impl ValidationOutcome {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid { .. })
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped { .. })
    }

    pub fn schema(&self) -> Option<&str> {
        match self {
            Self::Valid { schema } | Self::Invalid { schema, .. } => Some(schema),
            Self::Error { .. } | Self::Skipped { .. } => None,
        }
    }
}

/// Result of validating a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileValidationResult {
    /// Path to the validated file
    pub path: PathBuf,
    /// Validation outcome and state-specific data
    pub outcome: ValidationOutcome,
    /// Duration of validation
    pub duration: Duration,
}

impl FileValidationResult {
    /// Create a new successful validation result
    pub fn valid(path: PathBuf, schema_url: String, duration: Duration) -> Self {
        Self {
            path,
            outcome: ValidationOutcome::Valid { schema: schema_url },
            duration,
        }
    }

    /// Create a new invalid validation result
    pub fn invalid(
        path: PathBuf,
        schema_url: String,
        duration: Duration,
        error_details: Vec<String>,
    ) -> Self {
        let violations = Violations::from_vec(error_details).unwrap_or_else(|| Violations {
            first: "validator reported an invalid document without details".to_string(),
            rest: Vec::new(),
        });
        Self {
            path,
            outcome: ValidationOutcome::Invalid {
                schema: schema_url,
                violations,
            },
            duration,
        }
    }

    /// Create a new error validation result
    pub fn error(path: PathBuf, error: ValidationError, duration: Duration) -> Self {
        Self {
            path,
            outcome: ValidationOutcome::Error {
                message: error.to_string(),
            },
            duration,
        }
    }

    /// Create a new skipped validation result
    pub fn skipped(path: PathBuf, reason: String, duration: Duration) -> Self {
        Self {
            path,
            outcome: ValidationOutcome::Skipped { reason },
            duration,
        }
    }

    pub fn is_failure(&self) -> bool {
        self.outcome.is_invalid() || self.outcome.is_error()
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
    /// Validation duration
    pub validation_duration: Duration,
    /// Average time per file
    pub average_time_per_file: Duration,
    /// Files processed per second
    pub throughput_files_per_second: f64,
    /// Peak memory usage in MB, when supported by the operating system
    pub peak_memory_mb: Option<u64>,
    /// Cache hit rate percentage
    pub cache_hit_rate: f64,
    /// Configured upper bound on concurrent validations
    pub concurrency_limit: usize,
    /// Schema cache statistics
    pub schema_cache_stats: SchemaCacheStats,
}

/// Schema cache statistics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchemaCacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of schemas loaded
    pub schemas_loaded: usize,
    /// Total indexed schema-data size in bytes
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
            match result.outcome {
                ValidationOutcome::Valid { .. } => valid_files += 1,
                ValidationOutcome::Invalid { .. } => invalid_files += 1,
                ValidationOutcome::Error { .. } => error_files += 1,
                ValidationOutcome::Skipped { .. } => skipped_files += 1,
            }

            total_duration += result.duration;

            if let Some(schema) = result.outcome.schema() {
                schemas_used.insert(schema.to_owned());
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
            validation_duration: total_duration,
            average_time_per_file: average_duration,
            throughput_files_per_second: if total_duration.as_secs_f64() > 0.0 {
                total_files as f64 / total_duration.as_secs_f64()
            } else {
                0.0
            },
            peak_memory_mb: None,
            cache_hit_rate: 0.0,
            concurrency_limit: 1,
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
/// - **Sync operations**: schema parsing and XML validation on blocking threads
/// - **Concurrency**: a bounded in-flight future set drives parallel validation
/// - **Result aggregation**: completed outcomes are summarized without contradictory state
///
/// This design enables parallel validation without blocking Tokio's async workers.
pub struct ValidationEngine {
    schema_loader: Arc<SchemaLoader>,
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

        Ok(Self {
            schema_loader,
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
            validation_duration: Duration::ZERO,
            average_time_per_file: Duration::ZERO,
            throughput_files_per_second: 0.0,
            peak_memory_mb: None,
            cache_hit_rate: 0.0,
            concurrency_limit: self.config.max_concurrent_validations.get(),
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

        if self.config.collect_metrics {
            if let Ok(cache_stats) = self.collect_cache_statistics().await {
                let requests = cache_stats.hits + cache_stats.misses;
                performance_metrics.cache_hit_rate = if requests == 0 {
                    0.0
                } else {
                    cache_stats.hits as f64 * 100.0 / requests as f64
                };
                performance_metrics.schema_cache_stats = cache_stats;
            }
            performance_metrics.peak_memory_mb = self.get_peak_memory_usage().await;
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
        let mut remaining = files.into_iter();
        let mut pending = FuturesUnordered::new();
        let concurrency = self.config.max_concurrent_validations.get();
        let blocking_slots = Arc::new(Semaphore::new(concurrency));

        for file_path in remaining.by_ref().take(concurrency) {
            pending.push(Self::validate_with_timeout(
                file_path,
                Arc::clone(&self.schema_loader),
                Arc::clone(&blocking_slots),
                self.config.schema_override.clone(),
                self.config.validation_timeout,
            ));
        }

        let mut file_results = Vec::with_capacity(total_files);
        let mut stop_scheduling = false;
        while let Some((file_path, validation_result)) = pending.next().await {
            let should_stop = self.config.fail_fast && validation_result.is_failure();
            file_results.push(validation_result);

            if let Some(ref callback) = progress_callback {
                callback(ValidationProgress {
                    current_file: Some(file_path),
                    completed: file_results.len(),
                    total: total_files,
                    phase: ValidationPhase::Validation,
                });
            }

            if should_stop {
                // Do not detach work that was already admitted. Draining the
                // in-flight set keeps ownership/lifecycle behavior predictable
                // while still preventing any new files from being scheduled.
                stop_scheduling = true;
            }

            if !stop_scheduling && let Some(next_path) = remaining.next() {
                pending.push(Self::validate_with_timeout(
                    next_path,
                    Arc::clone(&self.schema_loader),
                    Arc::clone(&blocking_slots),
                    self.config.schema_override.clone(),
                    self.config.validation_timeout,
                ));
            }
        }

        Ok(file_results)
    }

    async fn validate_with_timeout(
        file_path: PathBuf,
        schema_loader: Arc<SchemaLoader>,
        blocking_slots: Arc<Semaphore>,
        schema_override: Option<PathBuf>,
        timeout: Duration,
    ) -> (PathBuf, FileValidationResult) {
        let result = tokio::time::timeout(
            timeout,
            Self::validate_single_file_internal(
                file_path.clone(),
                schema_loader,
                blocking_slots,
                schema_override,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            FileValidationResult::error(
                file_path.clone(),
                ValidationError::ValidationDeadline {
                    file: file_path.clone(),
                    timeout_ms: timeout.as_millis(),
                },
                timeout,
            )
        });
        (file_path, result)
    }

    /// Validate a single file (internal implementation)
    async fn validate_single_file_internal(
        file_path: PathBuf,
        schema_loader: Arc<SchemaLoader>,
        blocking_slots: Arc<Semaphore>,
        schema_override: Option<PathBuf>,
    ) -> FileValidationResult {
        let start_time = Instant::now();

        // Use schema override if provided, otherwise extract from XML
        let schema_ref = if let Some(schema_path) = schema_override {
            crate::schema_loader::SchemaReference::Local(schema_path)
        } else {
            match schema_loader
                .extractor()
                .extract_schema_hints(&file_path)
                .await
            {
                Ok(extracted) => match extracted.applicable().cloned() {
                    Some(reference) => reference,
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
            }
        };

        // `cache_key` (namespaced) keys the L1 cache and prevents
        // local/remote collisions; `schema_display` is the bare path/URL
        // surfaced to users through the outcome's schema field.
        let cache_key = match schema_loader.cache_key(&schema_ref).await {
            Ok(key) => key,
            Err(error) => {
                return FileValidationResult::error(file_path, error, start_time.elapsed());
            }
        };
        let schema_display = schema_ref.to_string();
        let parse_source = schema_display.clone();
        let local_schema_path = match &schema_ref {
            crate::schema_loader::SchemaReference::Local(path) => Some(path.clone()),
            crate::schema_loader::SchemaReference::Remote(_) => None,
        };
        let loader_clone = schema_loader.clone();
        let compile_slots = Arc::clone(&blocking_slots);

        let expected_cache_key = cache_key.clone();
        let schema = match schema_loader
            .cache()
            .parsed()
            .get_or_load(cache_key, || async move {
                let cached_bytes = loader_clone.load_schema(&schema_ref).await?;

                // Parse schema on a blocking thread (CPU-intensive for large schemas)
                let data = cached_bytes.data.clone();
                let compile_source = parse_source.clone();
                let parsed = Self::spawn_bounded(compile_slots, move || {
                    backend::compile(&data, &compile_source, local_schema_path.as_deref())
                })
                .await?;

                // Local schema graphs are multiple independently readable
                // files. Do not publish a compiled value under the digest
                // observed before loading if any member changed meanwhile.
                if matches!(schema_ref, crate::schema_loader::SchemaReference::Local(_)) {
                    let observed_cache_key = loader_clone.cache_key(&schema_ref).await?;
                    if observed_cache_key != expected_cache_key {
                        return Err(ValidationError::SchemaParsing {
                            url: parse_source,
                            details: "local schema graph changed while it was being compiled; retry validation".into(),
                        });
                    }
                }

                Ok(Arc::new(parsed))
            })
            .await
        {
            Ok(s) => s,
            Err(e) => return FileValidationResult::error(file_path, e, start_time.elapsed()),
        };

        // Validate file on a blocking thread (file I/O blocks)
        let validate_path = file_path.clone();
        let validate_schema = schema;

        let validation_result = Self::spawn_bounded(blocking_slots, move || {
            backend::validate(&validate_schema, &validate_path)
        })
        .await;

        let duration = start_time.elapsed();

        match validation_result {
            Ok(SchemaValidation::Valid) => {
                FileValidationResult::valid(file_path, schema_display, duration)
            }
            Ok(SchemaValidation::Invalid(errors)) => {
                FileValidationResult::invalid(file_path, schema_display, duration, errors)
            }
            Err(e) => FileValidationResult::error(file_path, e, duration),
        }
    }

    /// Run one FFI operation without allowing timed-out/dropped futures to
    /// exceed the configured blocking-work limit. The permit moves into the
    /// closure because Tokio cannot cancel a `spawn_blocking` task once started.
    async fn spawn_bounded<T, F>(blocking_slots: Arc<Semaphore>, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let permit =
            blocking_slots
                .acquire_owned()
                .await
                .map_err(|error| ValidationError::Concurrency {
                    details: format!("blocking-work limiter closed: {error}"),
                })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|error| ValidationError::Concurrency {
            details: format!("blocking task join error: {error}"),
        })?
    }

    /// Validate a single file (public interface)
    pub async fn validate_single_file(&self, file_path: &Path) -> Result<FileValidationResult> {
        let (_, result) = Self::validate_with_timeout(
            file_path.to_path_buf(),
            Arc::clone(&self.schema_loader),
            Arc::new(Semaphore::new(self.config.max_concurrent_validations.get())),
            self.config.schema_override.clone(),
            self.config.validation_timeout,
        )
        .await;

        Ok(result)
    }

    /// Get the schema loader for direct access
    pub fn schema_loader(&self) -> &Arc<SchemaLoader> {
        &self.schema_loader
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
                hits: stats.access.memory_hits + stats.access.disk_hits,
                misses: stats.access.misses,
                schemas_loaded: stats.memory.entry_count as usize,
                cache_size_bytes: stats.disk.total_size,
            }),
            Err(_) => Ok(SchemaCacheStats::default()),
        }
    }

    /// Get peak memory usage in MB
    async fn get_peak_memory_usage(&self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            if let Ok(status) = tokio::fs::read_to_string("/proc/self/status").await {
                for line in status.lines() {
                    if line.starts_with("VmPeak:")
                        && let Some(kb_str) = line.split_whitespace().nth(1)
                        && let Ok(kb) = kb_str.parse::<u64>()
                    {
                        return Some(kb / 1024); // Convert KB to MB
                    }
                }
            }
        }

        None
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
        let validation_config = ValidationConfig::new(2, Duration::from_secs(5)).unwrap();

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
    async fn test_validation_outcome_predicates() {
        let valid = ValidationOutcome::Valid {
            schema: "test.xsd".into(),
        };
        assert!(valid.is_valid());
        assert!(!valid.is_invalid());
        assert!(!valid.is_error());
        assert!(!valid.is_skipped());

        let invalid = ValidationOutcome::Invalid {
            schema: "test.xsd".into(),
            violations: Violations::from_vec(vec!["invalid".into()]).unwrap(),
        };
        assert!(!invalid.is_valid());
        assert!(invalid.is_invalid());
        assert!(!invalid.is_error());
        assert!(!invalid.is_skipped());

        let error = ValidationOutcome::Error {
            message: "test".to_string(),
        };
        assert!(!error.is_valid());
        assert!(!error.is_invalid());
        assert!(error.is_error());
        assert!(!error.is_skipped());

        let skipped = ValidationOutcome::Skipped {
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
        assert!(valid_result.outcome.is_valid());
        assert_eq!(
            valid_result.outcome.schema(),
            Some("http://example.com/schema.xsd")
        );

        let invalid_result = FileValidationResult::invalid(
            path.clone(),
            "http://example.com/schema.xsd".to_string(),
            duration,
            vec![
                "error1".to_string(),
                "error2".to_string(),
                "error3".to_string(),
            ],
        );
        assert!(invalid_result.outcome.is_invalid());

        let error_result = FileValidationResult::error(
            path.clone(),
            ValidationError::Config("test error".to_string()),
            duration,
        );
        assert!(error_result.outcome.is_error());

        let skipped_result =
            FileValidationResult::skipped(path, "no schema found".to_string(), duration);
        assert!(skipped_result.outcome.is_skipped());
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

        assert!(result.outcome.is_skipped());
        assert!(matches!(
            &result.outcome,
            ValidationOutcome::Skipped { reason } if reason.contains("No schema URL found")
        ));
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
            result.outcome.is_valid(),
            "Expected valid result, got: {:?}",
            result
        );
        assert!(result.outcome.schema().is_some());
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

        assert!(result.outcome.is_error());
        assert!(matches!(
            &result.outcome,
            ValidationOutcome::Error { message } if message.contains("Schema not found")
        ));
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
                result.outcome.is_valid(),
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
        let validation_config = ValidationConfig::new(1, Duration::from_millis(1)).unwrap();

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
        assert!(result.outcome.is_skipped() || result.outcome.is_error());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn timed_out_blocking_work_keeps_its_concurrency_permit() {
        let slots = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(ValidationEngine::spawn_bounded(
            Arc::clone(&slots),
            move || {
                let _ = started_tx.send(());
                release_rx
                    .recv()
                    .map_err(|error| ValidationError::Concurrency {
                        details: format!("test release channel closed: {error}"),
                    })?;
                Ok(())
            },
        ));

        started_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(slots.available_permits(), 0);

        release_tx.send(()).unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(5), slots.acquire())
            .await
            .expect("blocking operation did not release its permit")
            .expect("semaphore closed unexpectedly");
        drop(permit);
        assert_eq!(slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn test_validation_config_default() {
        let config = ValidationConfig::default();

        assert!(config.max_concurrent_validations().get() > 0);
        assert!(config.validation_timeout() > Duration::ZERO);
        assert!(!config.fail_fast());
    }

    #[test]
    fn validation_config_rejects_zero_values() {
        assert!(ValidationConfig::new(0, Duration::from_secs(1)).is_err());
        assert!(ValidationConfig::new(1, Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn fail_fast_stops_scheduling_new_files() {
        let (mut engine, temp_dir) = create_test_validation_engine();
        let schema = temp_dir.path().join("schema.xsd");
        tokio::fs::write(
            &schema,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="expected" type="xs:string"/></xs:schema>"#,
        )
        .await
        .unwrap();
        let invalid = temp_dir.path().join("a-invalid.xml");
        let unscheduled = temp_dir.path().join("b-valid.xml");
        tokio::fs::write(&invalid, "<wrong/>").await.unwrap();
        tokio::fs::write(&unscheduled, "<expected>ok</expected>")
            .await
            .unwrap();
        engine.config = ValidationConfig::new(1, Duration::from_secs(5))
            .unwrap()
            .with_fail_fast(true)
            .with_schema_override(Some(schema));

        let results = engine
            .validate_files(vec![invalid, unscheduled])
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].outcome.is_invalid());
    }

    #[tokio::test]
    async fn fail_fast_drains_files_already_admitted() {
        let (mut engine, temp_dir) = create_test_validation_engine();
        let schema = temp_dir.path().join("schema.xsd");
        tokio::fs::write(
            &schema,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:element name="expected" type="xs:string"/></xs:schema>"#,
        )
        .await
        .unwrap();
        let first = temp_dir.path().join("first.xml");
        let second = temp_dir.path().join("second.xml");
        tokio::fs::write(&first, "<wrong/>").await.unwrap();
        tokio::fs::write(&second, "<also-wrong/>").await.unwrap();
        engine.config = ValidationConfig::new(2, Duration::from_secs(5))
            .unwrap()
            .with_fail_fast(true)
            .with_schema_override(Some(schema));

        let results = engine.validate_files(vec![first, second]).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(FileValidationResult::is_failure));
    }

    #[tokio::test]
    async fn test_engine_accessors() {
        let (engine, _temp_dir) = create_test_validation_engine();

        let _schema_loader = engine.schema_loader();
        let config = engine.config();

        assert_eq!(config.max_concurrent_validations().get(), 2);
    }

    #[tokio::test]
    async fn test_validate_with_schema_override() {
        let temp_dir = TempDir::new().unwrap();

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

        // Create XML file WITHOUT any schema reference
        let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <element>content</element>
</root>"#;
        let xml_file = create_test_xml_file(xml_content);

        // Create engine with schema_override
        let cache_config = CacheConfig {
            directory: temp_dir.path().join("cache"),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        let cache = Arc::new(SchemaCache::new(cache_config));
        let http_config = HttpClientConfig::default();
        let http_client = AsyncHttpClient::new(http_config).unwrap();
        let validation_config = ValidationConfig::new(2, Duration::from_secs(5))
            .unwrap()
            .with_schema_override(Some(schema_file.clone()));

        let engine = ValidationEngine::new(cache, http_client, validation_config).unwrap();

        let result = engine.validate_single_file(xml_file.path()).await.unwrap();

        // Should be valid — the override schema was used instead of extracting from XML
        assert!(
            result.outcome.is_valid(),
            "Expected valid result with schema override, got: {:?}",
            result
        );
        assert_eq!(
            result.outcome.schema(),
            Some(schema_file.display().to_string()).as_deref()
        );
    }

    /// End-to-end regression test for the L1 parsed-schema cache-key
    /// collision that `SchemaReference::cache_key()` fixes.
    ///
    /// Before the fix, two XML files in different directories that each
    /// referenced `"schema.xsd"` (relative) shared an L1 cache entry
    /// keyed on the raw reference string. The second file would be
    /// validated against the *first* file's schema. Each directory here
    /// uses a differently-named root element (`alpha` vs `beta`) so that
    /// a mis-cached schema surfaces as an `Invalid` validation result —
    /// that's the signal the test watches for.
    #[tokio::test]
    async fn test_l1_cache_key_does_not_collide_across_directories()
    -> std::result::Result<(), ValidationError> {
        let tmp = TempDir::new()?;
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        std::fs::create_dir_all(&dir_a)?;
        std::fs::create_dir_all(&dir_b)?;

        let schema_a = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="alpha" type="xs:string"/>
</xs:schema>"#;
        let doc_a = r#"<?xml version="1.0" encoding="UTF-8"?>
<alpha xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
       xsi:noNamespaceSchemaLocation="schema.xsd">hello</alpha>"#;

        // Both docs reference `schema.xsd` (relative) — identical raw
        // strings, different resolved paths. That's the collision input.
        let schema_b = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="beta" type="xs:string"/>
</xs:schema>"#;
        let doc_b = r#"<?xml version="1.0" encoding="UTF-8"?>
<beta xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="schema.xsd">world</beta>"#;

        std::fs::write(dir_a.join("schema.xsd"), schema_a)?;
        std::fs::write(dir_b.join("schema.xsd"), schema_b)?;
        let xml_a = dir_a.join("doc.xml");
        let xml_b = dir_b.join("doc.xml");
        std::fs::write(&xml_a, doc_a)?;
        std::fs::write(&xml_b, doc_b)?;

        // One engine, one cache, two validate calls — a collision would
        // manifest as doc B reusing doc A's parsed schema.
        let cache = Arc::new(SchemaCache::new(CacheConfig {
            directory: tmp.path().join("cache"),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        }));
        let http_client = AsyncHttpClient::new(HttpClientConfig::default())?;
        let engine = ValidationEngine::new(
            cache,
            http_client,
            ValidationConfig::new(2, Duration::from_secs(5))?,
        )?;

        let result_a = engine.validate_single_file(&xml_a).await?;
        let result_b = engine.validate_single_file(&xml_b).await?;

        assert!(
            result_a.outcome.is_valid(),
            "doc A should validate against its own schema; got {:?}",
            result_a
        );
        assert!(
            result_b.outcome.is_valid(),
            "doc B should validate against its own schema; the old cache-key bug \
             would have reused A's parsed schema here, failing with an Invalid; got {:?}",
            result_b
        );
        Ok(())
    }
}
