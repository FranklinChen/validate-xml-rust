//! Simple Output and Reporting
//!
//! This module provides output formatting for validation results.

use atty;
use std::time::Duration;

use crate::cli::VerbosityLevel;
use crate::validator::{
    FileValidationResult, PerformanceMetrics, ValidationResults, ValidationStatus,
};

/// Simple output formatter for human-readable results
pub struct Output {
    verbosity: VerbosityLevel,
    show_colors: bool,
}

impl Output {
    pub fn new(verbosity: VerbosityLevel) -> Self {
        Self {
            verbosity,
            show_colors: atty::is(atty::Stream::Stdout),
        }
    }

    fn colorize(&self, text: &str, color: &str) -> String {
        if self.show_colors {
            format!("\x1b[{}m{}\x1b[0m", color, text)
        } else {
            text.to_string()
        }
    }

    pub fn format_results(&self, results: &ValidationResults) -> String {
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
            VerbosityLevel::Normal | VerbosityLevel::Verbose | VerbosityLevel::Debug => {
                output.push_str(&self.format_summary(results));
                output.push('\n');

                if self.verbosity >= VerbosityLevel::Verbose {
                    for file_result in &results.file_results {
                        if !file_result.status.is_valid() {
                            output.push_str(&self.format_file_result(file_result));
                            output.push('\n');
                        }
                    }
                }

                if self.verbosity == VerbosityLevel::Debug {
                    output.push_str(&self.format_debug_info(results));
                }
            }
        }

        output
    }

    pub fn format_file_result(&self, result: &FileValidationResult) -> String {
        let path_display = result.path.display();
        let duration_str = format_duration(result.duration);

        match &result.status {
            ValidationStatus::Valid => {
                format!(
                    "{}  {} ({})",
                    self.colorize("✓ VALID", "32"),
                    path_display,
                    duration_str
                )
            }
            ValidationStatus::Invalid { error_count } => {
                let mut output = format!(
                    "{}  {} ({}) - {} error{}",
                    self.colorize("✗ INVALID", "31"),
                    path_display,
                    duration_str,
                    error_count,
                    if *error_count == 1 { "" } else { "s" }
                );

                if self.verbosity >= VerbosityLevel::Verbose {
                    for error_detail in &result.error_details {
                        output.push_str(&format!("\n    {}", error_detail));
                    }
                }
                output
            }
            ValidationStatus::Error { message } => {
                format!(
                    "{}  {} ({}) - {}",
                    self.colorize("⚠ ERROR", "33"),
                    path_display,
                    duration_str,
                    message
                )
            }
            ValidationStatus::Skipped { reason } => {
                format!(
                    "{}  {} ({}) - {}",
                    self.colorize("- SKIPPED", "36"),
                    path_display,
                    duration_str,
                    reason
                )
            }
        }
    }

    fn format_summary(&self, results: &ValidationResults) -> String {
        let mut output = String::new();
        output.push_str("Validation Summary:\n");
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
            format_duration(results.total_duration)
        ));

        if self.verbosity >= VerbosityLevel::Verbose {
            output.push_str(&self.format_performance_metrics(&results.performance_metrics));
        }

        output
    }

    fn format_performance_metrics(&self, metrics: &PerformanceMetrics) -> String {
        let mut output = String::new();
        output.push_str("\nPerformance Metrics:\n");
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
        }
        output
    }

    fn format_debug_info(&self, results: &ValidationResults) -> String {
        let mut output = String::new();
        output.push_str("\nDebug Information:\n");
        output.push_str(&format!("  Schemas used: {}\n", results.schemas_used.len()));
        for (i, schema) in results.schemas_used.iter().enumerate() {
            output.push_str(&format!("    {}: {}\n", i + 1, schema));
        }
        output
    }
}

fn format_duration(duration: Duration) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator::SchemaCacheStats;

    fn create_test_results() -> ValidationResults {
        ValidationResults::with_metrics(
            vec![],
            PerformanceMetrics {
                total_duration: Duration::from_millis(100),
                discovery_duration: Duration::ZERO,
                schema_loading_duration: Duration::ZERO,
                validation_duration: Duration::ZERO,
                average_time_per_file: Duration::ZERO,
                throughput_files_per_second: 0.0,
                peak_memory_mb: 0,
                cache_hit_rate: 0.0,
                concurrent_validations: 1,
                schema_cache_stats: SchemaCacheStats {
                    hits: 0,
                    misses: 0,
                    schemas_loaded: 0,
                    cache_size_bytes: 0,
                },
            },
        )
    }

    #[test]
    fn test_output_summary() {
        let output = Output::new(VerbosityLevel::Normal);
        let results = create_test_results();
        let formatted = output.format_results(&results);
        assert!(formatted.contains("Validation Summary:"));
    }
}
