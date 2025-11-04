//! Enhanced Output and Reporting System
//!
//! This module provides structured output formatters for different verbosity levels,
//! machine-readable output formats (JSON), progress indicators, and comprehensive
//! validation summaries with statistics and performance metrics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use crate::cli::OutputFormat;
use crate::error_reporter::VerbosityLevel;
use crate::validator::{
    FileValidationResult, PerformanceMetrics, ValidationResults, ValidationStatus,
};

/// Output formatter trait for different output formats
pub trait OutputFormatter {
    /// Format validation results
    fn format_results(&self, results: &ValidationResults) -> Result<String, OutputError>;

    /// Format progress update
    fn format_progress(
        &self,
        current: usize,
        total: usize,
        current_file: Option<&PathBuf>,
    ) -> Result<String, OutputError>;

    /// Format individual file result
    fn format_file_result(&self, result: &FileValidationResult) -> Result<String, OutputError>;

    /// Format summary statistics
    fn format_summary(&self, results: &ValidationResults) -> Result<String, OutputError>;
}

/// Human-readable output formatter
pub struct HumanFormatter {
    verbosity: VerbosityLevel,
    show_timestamps: bool,
    show_colors: bool,
}

impl HumanFormatter {
    pub fn new(verbosity: VerbosityLevel) -> Self {
        Self {
            verbosity,
            show_timestamps: false,
            show_colors: atty::is(atty::Stream::Stdout),
        }
    }

    pub fn with_options(
        verbosity: VerbosityLevel,
        show_timestamps: bool,
        show_colors: bool,
    ) -> Self {
        Self {
            verbosity,
            show_timestamps,
            show_colors,
        }
    }

    fn colorize(&self, text: &str, color: &str) -> String {
        if self.show_colors {
            format!("\x1b[{}m{}\x1b[0m", color, text)
        } else {
            text.to_string()
        }
    }

    fn format_timestamp(&self) -> String {
        if self.show_timestamps {
            format!("[{}] ", Utc::now().format("%H:%M:%S"))
        } else {
            String::new()
        }
    }

    fn format_duration(&self, duration: Duration) -> String {
        let total_secs = duration.as_secs_f64();
        if total_secs < 1.0 {
            format!("{:.0}ms", duration.as_millis())
        } else if total_secs < 60.0 {
            format!("{:.2}s", total_secs)
        } else {
            let mins = (total_secs / 60.0) as u64;
            let secs = total_secs % 60.0;
            format!("{}m{:.1}s", mins, secs)
        }
    }
}

impl OutputFormatter for HumanFormatter {
    fn format_results(&self, results: &ValidationResults) -> Result<String, OutputError> {
        let mut output = String::new();

        match self.verbosity {
            VerbosityLevel::Quiet => {
                if results.has_errors() {
                    output.push_str(&format!(
                        "Errors: {} Invalid: {}\n",
                        results.error_files, results.invalid_files
                    ));
                }
            }
            VerbosityLevel::Normal => {
                output.push_str(&self.format_summary(results)?);
            }
            VerbosityLevel::Verbose | VerbosityLevel::Debug => {
                output.push_str(&self.format_summary(results)?);
                output.push('\n');

                // Show individual file results for failed files
                for file_result in &results.file_results {
                    if !file_result.status.is_valid() {
                        output.push_str(&self.format_file_result(file_result)?);
                        output.push('\n');
                    }
                }

                if self.verbosity == VerbosityLevel::Debug {
                    output.push_str(&self.format_debug_info(results)?);
                }
            }
        }

        Ok(output)
    }

    fn format_progress(
        &self,
        current: usize,
        total: usize,
        current_file: Option<&PathBuf>,
    ) -> Result<String, OutputError> {
        if matches!(self.verbosity, VerbosityLevel::Quiet) {
            return Ok(String::new());
        }

        let percentage = if total > 0 {
            (current as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };

        let progress_bar = self.create_progress_bar(current, total, 40);

        match self.verbosity {
            VerbosityLevel::Normal => Ok(format!(
                "\r{}{} {}/{} ({}%)",
                self.format_timestamp(),
                progress_bar,
                current,
                total,
                percentage
            )),
            VerbosityLevel::Verbose | VerbosityLevel::Debug => {
                if let Some(file) = current_file {
                    let filename = file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    Ok(format!(
                        "\r{}{} {}/{} ({}%) - {}",
                        self.format_timestamp(),
                        progress_bar,
                        current,
                        total,
                        percentage,
                        filename
                    ))
                } else {
                    Ok(format!(
                        "\r{}{} {}/{} ({}%)",
                        self.format_timestamp(),
                        progress_bar,
                        current,
                        total,
                        percentage
                    ))
                }
            }
            VerbosityLevel::Quiet => Ok(String::new()),
        }
    }

    fn format_file_result(&self, result: &FileValidationResult) -> Result<String, OutputError> {
        let timestamp = self.format_timestamp();
        let path_display = result.path.display();
        let duration_str = self.format_duration(result.duration);

        match &result.status {
            ValidationStatus::Valid => {
                Ok(format!(
                    "{}{}  {} ({})",
                    timestamp,
                    self.colorize("✓ VALID", "32"), // Green
                    path_display,
                    duration_str
                ))
            }
            ValidationStatus::Invalid { error_count } => {
                let mut output = format!(
                    "{}{}  {} ({}) - {} error{}",
                    timestamp,
                    self.colorize("✗ INVALID", "31"), // Red
                    path_display,
                    duration_str,
                    error_count,
                    if *error_count == 1 { "" } else { "s" }
                );

                if matches!(
                    self.verbosity,
                    VerbosityLevel::Verbose | VerbosityLevel::Debug
                ) {
                    for error_detail in &result.error_details {
                        output.push_str(&format!("\n    {}", error_detail));
                    }
                }

                Ok(output)
            }
            ValidationStatus::Error { message } => {
                Ok(format!(
                    "{}{}  {} ({}) - {}",
                    timestamp,
                    self.colorize("⚠ ERROR", "33"), // Yellow
                    path_display,
                    duration_str,
                    message
                ))
            }
            ValidationStatus::Skipped { reason } => {
                Ok(format!(
                    "{}{}  {} ({}) - {}",
                    timestamp,
                    self.colorize("- SKIPPED", "36"), // Cyan
                    path_display,
                    duration_str,
                    reason
                ))
            }
        }
    }

    fn format_summary(&self, results: &ValidationResults) -> Result<String, OutputError> {
        let mut output = String::new();

        output.push_str(&format!("{}Validation Summary:\n", self.format_timestamp()));
        output.push_str(&format!("  Total files: {}\n", results.total_files));
        output.push_str(&format!(
            "  {} {}\n",
            self.colorize("Valid:", "32"),
            results.valid_files
        ));

        if results.invalid_files > 0 {
            output.push_str(&format!(
                "  {} {}\n",
                self.colorize("Invalid:", "31"),
                results.invalid_files
            ));
        }

        if results.error_files > 0 {
            output.push_str(&format!(
                "  {} {}\n",
                self.colorize("Errors:", "33"),
                results.error_files
            ));
        }

        if results.skipped_files > 0 {
            output.push_str(&format!(
                "  {} {}\n",
                self.colorize("Skipped:", "36"),
                results.skipped_files
            ));
        }

        output.push_str(&format!("  Success rate: {:.1}%\n", results.success_rate()));
        output.push_str(&format!(
            "  Duration: {}\n",
            self.format_duration(results.total_duration)
        ));

        if matches!(
            self.verbosity,
            VerbosityLevel::Verbose | VerbosityLevel::Debug
        ) {
            output.push_str(&self.format_performance_metrics(&results.performance_metrics)?);
        }

        Ok(output)
    }
}

impl HumanFormatter {
    fn create_progress_bar(&self, current: usize, total: usize, width: usize) -> String {
        if total == 0 {
            return "".to_string();
        }

        let filled = (current * width) / total;
        let empty = width - filled;

        format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
    }

    fn format_performance_metrics(
        &self,
        metrics: &PerformanceMetrics,
    ) -> Result<String, OutputError> {
        let mut output = String::new();

        output.push_str("\nPerformance Metrics:\n");
        output.push_str(&format!(
            "  Discovery time: {}\n",
            self.format_duration(metrics.discovery_duration)
        ));
        output.push_str(&format!(
            "  Validation time: {}\n",
            self.format_duration(metrics.validation_duration)
        ));
        output.push_str(&format!(
            "  Average per file: {}\n",
            self.format_duration(metrics.average_time_per_file)
        ));
        output.push_str(&format!(
            "  Throughput: {:.1} files/sec\n",
            metrics.throughput_files_per_second
        ));
        output.push_str(&format!(
            "  Concurrent validations: {}\n",
            metrics.concurrent_validations
        ));

        if self.verbosity == VerbosityLevel::Debug {
            output.push_str(&format!("  Peak memory: {} MB\n", metrics.peak_memory_mb));
            output.push_str(&format!(
                "  Cache hit rate: {:.1}%\n",
                metrics.cache_hit_rate
            ));
            output.push_str(&format!(
                "  Schemas loaded: {}\n",
                metrics.schema_cache_stats.schemas_loaded
            ));
        }

        Ok(output)
    }

    fn format_debug_info(&self, results: &ValidationResults) -> Result<String, OutputError> {
        let mut output = String::new();

        output.push_str("\nDebug Information:\n");
        output.push_str(&format!("  Schemas used: {}\n", results.schemas_used.len()));

        for (i, schema) in results.schemas_used.iter().enumerate() {
            output.push_str(&format!("    {}: {}\n", i + 1, schema));
        }

        output.push_str("  Cache statistics:\n");
        output.push_str(&format!(
            "    Hits: {}\n",
            results.performance_metrics.schema_cache_stats.hits
        ));
        output.push_str(&format!(
            "    Misses: {}\n",
            results.performance_metrics.schema_cache_stats.misses
        ));
        output.push_str(&format!(
            "    Size: {} bytes\n",
            results
                .performance_metrics
                .schema_cache_stats
                .cache_size_bytes
        ));

        Ok(output)
    }
}

/// JSON output formatter for machine-readable output
pub struct JsonFormatter {
    pretty_print: bool,
}

impl JsonFormatter {
    pub fn new(pretty_print: bool) -> Self {
        Self { pretty_print }
    }
}

impl OutputFormatter for JsonFormatter {
    fn format_results(&self, results: &ValidationResults) -> Result<String, OutputError> {
        let json_results = JsonValidationResults::from(results);

        if self.pretty_print {
            serde_json::to_string_pretty(&json_results)
                .map_err(|e| OutputError::SerializationError(e.to_string()))
        } else {
            serde_json::to_string(&json_results)
                .map_err(|e| OutputError::SerializationError(e.to_string()))
        }
    }

    fn format_progress(
        &self,
        current: usize,
        total: usize,
        current_file: Option<&PathBuf>,
    ) -> Result<String, OutputError> {
        let progress = JsonProgress {
            current,
            total,
            percentage: if total > 0 {
                current as f64 / total as f64 * 100.0
            } else {
                0.0
            },
            current_file: current_file.map(|p| p.to_string_lossy().to_string()),
            timestamp: Utc::now(),
        };

        serde_json::to_string(&progress).map_err(|e| OutputError::SerializationError(e.to_string()))
    }

    fn format_file_result(&self, result: &FileValidationResult) -> Result<String, OutputError> {
        let json_result = JsonFileResult::from(result);

        serde_json::to_string(&json_result)
            .map_err(|e| OutputError::SerializationError(e.to_string()))
    }

    fn format_summary(&self, results: &ValidationResults) -> Result<String, OutputError> {
        let summary = JsonSummary::from(results);

        if self.pretty_print {
            serde_json::to_string_pretty(&summary)
                .map_err(|e| OutputError::SerializationError(e.to_string()))
        } else {
            serde_json::to_string(&summary)
                .map_err(|e| OutputError::SerializationError(e.to_string()))
        }
    }
}

/// Compact summary formatter
pub struct SummaryFormatter;

impl OutputFormatter for SummaryFormatter {
    fn format_results(&self, results: &ValidationResults) -> Result<String, OutputError> {
        Ok(format!(
            "{}/{} valid ({:.1}%) in {:.2}s",
            results.valid_files,
            results.total_files,
            results.success_rate(),
            results.total_duration.as_secs_f64()
        ))
    }

    fn format_progress(
        &self,
        current: usize,
        total: usize,
        _current_file: Option<&PathBuf>,
    ) -> Result<String, OutputError> {
        let percentage = if total > 0 {
            (current as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };

        Ok(format!("\r{}/{} ({}%)", current, total, percentage))
    }

    fn format_file_result(&self, result: &FileValidationResult) -> Result<String, OutputError> {
        let status_char = match result.status {
            ValidationStatus::Valid => "✓",
            ValidationStatus::Invalid { .. } => "✗",
            ValidationStatus::Error { .. } => "⚠",
            ValidationStatus::Skipped { .. } => "-",
        };

        Ok(format!("{} {}", status_char, result.path.display()))
    }

    fn format_summary(&self, results: &ValidationResults) -> Result<String, OutputError> {
        Ok(format!(
            "Total: {} Valid: {} Invalid: {} Errors: {} ({:.1}%)",
            results.total_files,
            results.valid_files,
            results.invalid_files,
            results.error_files,
            results.success_rate()
        ))
    }
}

/// Progress indicator for long-running operations
pub struct ProgressIndicator {
    formatter: Box<dyn OutputFormatter + Send + Sync>,
    writer: Box<dyn Write + Send + Sync>,
    last_update: std::time::Instant,
    update_interval: Duration,
}

impl ProgressIndicator {
    pub fn new(formatter: Box<dyn OutputFormatter + Send + Sync>) -> Self {
        Self {
            formatter,
            writer: Box::new(io::stderr()),
            last_update: std::time::Instant::now(),
            update_interval: Duration::from_millis(100),
        }
    }

    pub fn with_writer(mut self, writer: Box<dyn Write + Send + Sync>) -> Self {
        self.writer = writer;
        self
    }

    pub fn with_update_interval(mut self, interval: Duration) -> Self {
        self.update_interval = interval;
        self
    }

    pub fn update(
        &mut self,
        current: usize,
        total: usize,
        current_file: Option<&PathBuf>,
    ) -> Result<(), OutputError> {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_update) >= self.update_interval || current == total {
            let progress_text = self
                .formatter
                .format_progress(current, total, current_file)?;
            if !progress_text.is_empty() {
                write!(self.writer, "{}", progress_text)
                    .map_err(|e| OutputError::WriteError(e.to_string()))?;
                self.writer
                    .flush()
                    .map_err(|e| OutputError::WriteError(e.to_string()))?;
            }
            self.last_update = now;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), OutputError> {
        writeln!(self.writer).map_err(|e| OutputError::WriteError(e.to_string()))?;
        self.writer
            .flush()
            .map_err(|e| OutputError::WriteError(e.to_string()))?;
        Ok(())
    }
}

/// Output writer that handles different output formats and destinations
pub struct OutputWriter {
    formatter: Box<dyn OutputFormatter + Send + Sync>,
    writer: Box<dyn Write + Send + Sync>,
}

impl OutputWriter {
    pub fn new(format: OutputFormat, verbosity: VerbosityLevel) -> Self {
        let formatter: Box<dyn OutputFormatter + Send + Sync> = match format {
            OutputFormat::Human => Box::new(HumanFormatter::new(verbosity)),
            OutputFormat::Json => Box::new(JsonFormatter::new(true)),
            OutputFormat::Summary => Box::new(SummaryFormatter),
        };

        Self {
            formatter,
            writer: Box::new(io::stdout()),
        }
    }

    pub fn with_writer(mut self, writer: Box<dyn Write + Send + Sync>) -> Self {
        self.writer = writer;
        self
    }

    pub fn write_results(&mut self, results: &ValidationResults) -> Result<(), OutputError> {
        let output = self.formatter.format_results(results)?;
        write!(self.writer, "{}", output).map_err(|e| OutputError::WriteError(e.to_string()))?;
        self.writer
            .flush()
            .map_err(|e| OutputError::WriteError(e.to_string()))?;
        Ok(())
    }

    pub fn write_file_result(&mut self, result: &FileValidationResult) -> Result<(), OutputError> {
        let output = self.formatter.format_file_result(result)?;
        writeln!(self.writer, "{}", output).map_err(|e| OutputError::WriteError(e.to_string()))?;
        self.writer
            .flush()
            .map_err(|e| OutputError::WriteError(e.to_string()))?;
        Ok(())
    }

    pub fn write_summary(&mut self, results: &ValidationResults) -> Result<(), OutputError> {
        let output = self.formatter.format_summary(results)?;
        writeln!(self.writer, "{}", output).map_err(|e| OutputError::WriteError(e.to_string()))?;
        self.writer
            .flush()
            .map_err(|e| OutputError::WriteError(e.to_string()))?;
        Ok(())
    }
}

/// JSON serializable structures for machine-readable output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonValidationResults {
    pub summary: JsonSummary,
    pub files: Vec<JsonFileResult>,
    pub schemas: Vec<String>,
    pub performance: JsonPerformanceMetrics,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSummary {
    pub total_files: usize,
    pub valid_files: usize,
    pub invalid_files: usize,
    pub error_files: usize,
    pub skipped_files: usize,
    pub success_rate: f64,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonFileResult {
    pub path: String,
    pub status: String,
    pub schema_url: Option<String>,
    pub duration_ms: u64,
    pub error_details: Vec<String>,
    pub error_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonPerformanceMetrics {
    pub total_duration_ms: u64,
    pub discovery_duration_ms: u64,
    pub validation_duration_ms: u64,
    pub average_time_per_file_ms: u64,
    pub throughput_files_per_second: f64,
    pub peak_memory_mb: u64,
    pub cache_hit_rate: f64,
    pub concurrent_validations: usize,
    pub cache_stats: JsonCacheStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub schemas_loaded: usize,
    pub cache_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonProgress {
    pub current: usize,
    pub total: usize,
    pub percentage: f64,
    pub current_file: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Convert ValidationResults to JSON format
impl From<&ValidationResults> for JsonValidationResults {
    fn from(results: &ValidationResults) -> Self {
        Self {
            summary: JsonSummary::from(results),
            files: results
                .file_results
                .iter()
                .map(JsonFileResult::from)
                .collect(),
            schemas: results.schemas_used.clone(),
            performance: JsonPerformanceMetrics::from(&results.performance_metrics),
            timestamp: Utc::now(),
        }
    }
}

impl From<&ValidationResults> for JsonSummary {
    fn from(results: &ValidationResults) -> Self {
        Self {
            total_files: results.total_files,
            valid_files: results.valid_files,
            invalid_files: results.invalid_files,
            error_files: results.error_files,
            skipped_files: results.skipped_files,
            success_rate: results.success_rate(),
            total_duration_ms: results.total_duration.as_millis() as u64,
        }
    }
}

impl From<&FileValidationResult> for JsonFileResult {
    fn from(result: &FileValidationResult) -> Self {
        let (status, error_count) = match &result.status {
            ValidationStatus::Valid => ("valid".to_string(), None),
            ValidationStatus::Invalid { error_count } => {
                ("invalid".to_string(), Some(*error_count))
            }
            ValidationStatus::Error { message: _ } => ("error".to_string(), None),
            ValidationStatus::Skipped { reason: _ } => ("skipped".to_string(), None),
        };

        Self {
            path: result.path.to_string_lossy().to_string(),
            status,
            schema_url: result.schema_url.clone(),
            duration_ms: result.duration.as_millis() as u64,
            error_details: result.error_details.clone(),
            error_count,
        }
    }
}

impl From<&PerformanceMetrics> for JsonPerformanceMetrics {
    fn from(metrics: &PerformanceMetrics) -> Self {
        Self {
            total_duration_ms: metrics.total_duration.as_millis() as u64,
            discovery_duration_ms: metrics.discovery_duration.as_millis() as u64,
            validation_duration_ms: metrics.validation_duration.as_millis() as u64,
            average_time_per_file_ms: metrics.average_time_per_file.as_millis() as u64,
            throughput_files_per_second: metrics.throughput_files_per_second,
            peak_memory_mb: metrics.peak_memory_mb,
            cache_hit_rate: metrics.cache_hit_rate,
            concurrent_validations: metrics.concurrent_validations,
            cache_stats: JsonCacheStats::from(&metrics.schema_cache_stats),
        }
    }
}

impl From<&crate::validator::SchemaCacheStats> for JsonCacheStats {
    fn from(stats: &crate::validator::SchemaCacheStats) -> Self {
        Self {
            hits: stats.hits,
            misses: stats.misses,
            schemas_loaded: stats.schemas_loaded,
            cache_size_bytes: stats.cache_size_bytes,
        }
    }
}

/// Errors that can occur during output formatting
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Write error: {0}")]
    WriteError(String),

    #[error("Format error: {0}")]
    FormatError(String),
}

/// Factory for creating output formatters
pub struct OutputFormatterFactory;

impl OutputFormatterFactory {
    pub fn create_formatter(
        format: OutputFormat,
        verbosity: VerbosityLevel,
    ) -> Box<dyn OutputFormatter + Send + Sync> {
        match format {
            OutputFormat::Human => Box::new(HumanFormatter::new(verbosity)),
            OutputFormat::Json => Box::new(JsonFormatter::new(true)),
            OutputFormat::Summary => Box::new(SummaryFormatter),
        }
    }

    pub fn create_progress_indicator(
        format: OutputFormat,
        verbosity: VerbosityLevel,
    ) -> ProgressIndicator {
        let formatter = Self::create_formatter(format, verbosity);
        ProgressIndicator::new(formatter)
    }

    pub fn create_output_writer(format: OutputFormat, verbosity: VerbosityLevel) -> OutputWriter {
        OutputWriter::new(format, verbosity)
    }
}
