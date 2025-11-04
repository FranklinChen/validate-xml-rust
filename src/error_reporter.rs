use crate::error::{ConfigError, ValidationError};
use std::path::PathBuf;

/// Verbosity levels for error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbosityLevel {
    /// Only show critical errors
    Quiet,
    /// Show standard error information
    Normal,
    /// Show detailed error information with context
    Verbose,
    /// Show all available debugging information
    Debug,
}

/// Error reporter with configurable verbosity
pub struct ErrorReporter {
    verbosity: VerbosityLevel,
    show_timestamps: bool,
    show_error_codes: bool,
}

impl ErrorReporter {
    /// Create a new error reporter with specified verbosity
    pub fn new(verbosity: VerbosityLevel) -> Self {
        Self {
            verbosity,
            show_timestamps: false,
            show_error_codes: false,
        }
    }

    /// Create a new error reporter with additional options
    pub fn with_options(
        verbosity: VerbosityLevel,
        show_timestamps: bool,
        show_error_codes: bool,
    ) -> Self {
        Self {
            verbosity,
            show_timestamps,
            show_error_codes,
        }
    }

    /// Report a validation error with appropriate verbosity
    pub fn report_validation_error(&self, error: &ValidationError) {
        match self.verbosity {
            VerbosityLevel::Quiet => {
                if self.is_critical_error(error) {
                    eprintln!("{}", self.format_error_brief(error));
                }
            }
            VerbosityLevel::Normal => {
                eprintln!("{}", self.format_error_normal(error));
            }
            VerbosityLevel::Verbose => {
                eprintln!("{}", self.format_error_verbose(error));
            }
            VerbosityLevel::Debug => {
                eprintln!("{}", self.format_error_debug(error));
            }
        }
    }

    /// Report a configuration error
    pub fn report_config_error(&self, error: &ConfigError) {
        let formatted = match self.verbosity {
            VerbosityLevel::Quiet => format!("Config error: {}", error),
            VerbosityLevel::Normal | VerbosityLevel::Verbose => {
                format!(
                    "Configuration Error: {}\n{}",
                    error,
                    self.get_config_help(error)
                )
            }
            VerbosityLevel::Debug => {
                format!(
                    "Configuration Error: {}\nDebug: {:?}\n{}",
                    error,
                    error,
                    self.get_config_help(error)
                )
            }
        };
        eprintln!("{}", formatted);
    }

    /// Report a summary of validation results
    pub fn report_summary(&self, results: &ValidationSummary) {
        match self.verbosity {
            VerbosityLevel::Quiet => {
                if results.error_count > 0 {
                    eprintln!("Errors: {}", results.error_count);
                }
            }
            VerbosityLevel::Normal => {
                eprintln!("Validation Summary:");
                eprintln!("  Total files: {}", results.total_files);
                eprintln!("  Valid: {}", results.valid_count);
                eprintln!("  Invalid: {}", results.invalid_count);
                eprintln!("  Errors: {}", results.error_count);
            }
            VerbosityLevel::Verbose | VerbosityLevel::Debug => {
                eprintln!("Validation Summary:");
                eprintln!("  Total files processed: {}", results.total_files);
                eprintln!("  Valid files: {}", results.valid_count);
                eprintln!("  Invalid files: {}", results.invalid_count);
                eprintln!("  Files with errors: {}", results.error_count);
                eprintln!("  Duration: {:?}", results.duration);
                eprintln!("  Schemas cached: {}", results.schemas_cached);

                if self.verbosity == VerbosityLevel::Debug {
                    eprintln!("  Memory usage: {} MB", results.memory_usage_mb);
                    eprintln!("  Cache hits: {}", results.cache_hits);
                    eprintln!("  Cache misses: {}", results.cache_misses);
                }
            }
        }
    }

    /// Report progress for long-running operations
    pub fn report_progress(&self, current: usize, total: usize, current_file: Option<&PathBuf>) {
        if self.verbosity == VerbosityLevel::Quiet {
            return;
        }

        let percentage = (current as f64 / total as f64 * 100.0) as u32;

        match self.verbosity {
            VerbosityLevel::Normal => {
                eprint!("\rProgress: {}/{} ({}%)", current, total, percentage);
            }
            VerbosityLevel::Verbose | VerbosityLevel::Debug => {
                if let Some(file) = current_file {
                    eprint!(
                        "\rProgress: {}/{} ({}%) - Processing: {}",
                        current,
                        total,
                        percentage,
                        file.display()
                    );
                } else {
                    eprint!("\rProgress: {}/{} ({}%)", current, total, percentage);
                }
            }
            VerbosityLevel::Quiet => {}
        }

        if current == total {
            eprintln!(); // New line when complete
        }
    }

    /// Check if an error is considered critical
    fn is_critical_error(&self, error: &ValidationError) -> bool {
        matches!(
            error,
            ValidationError::Config(_)
                | ValidationError::LibXml2Internal { .. }
                | ValidationError::ResourceExhaustion { .. }
        )
    }

    /// Format error for brief output (quiet mode)
    fn format_error_brief(&self, error: &ValidationError) -> String {
        match error {
            ValidationError::ValidationFailed { file, .. } => {
                format!("INVALID: {}", file.display())
            }
            ValidationError::SchemaNotFound { url } => {
                format!("SCHEMA NOT FOUND: {}", url)
            }
            _ => format!("ERROR: {}", error),
        }
    }

    /// Format error for normal output
    fn format_error_normal(&self, error: &ValidationError) -> String {
        let timestamp = if self.show_timestamps {
            format!("[{}] ", chrono::Utc::now().format("%H:%M:%S"))
        } else {
            String::new()
        };

        format!("{}{}", timestamp, error)
    }

    /// Format error for verbose output
    fn format_error_verbose(&self, error: &ValidationError) -> String {
        let mut output = self.format_error_normal(error);

        // Add context and suggestions based on error type
        match error {
            ValidationError::Http(http_err) => {
                output.push_str("\nSuggestion: Check network connectivity and URL validity");
                if self.show_error_codes {
                    output.push_str(&format!("\nHTTP Error Details: {:?}", http_err));
                }
            }
            ValidationError::SchemaNotFound { url } => {
                output.push_str(&format!(
                    "\nSuggestion: Verify the schema URL is correct and accessible: {}",
                    url
                ));
            }
            ValidationError::ValidationFailed { file, details } => {
                output.push_str(&format!("\nFile: {}", file.display()));
                output.push_str(&format!("\nDetails: {}", details));
                output.push_str("\nSuggestion: Check XML syntax and schema compliance");
            }
            ValidationError::Cache(_cache_err) => {
                output.push_str("\nSuggestion: Try clearing the cache or check disk space");
            }
            _ => {}
        }

        output
    }

    /// Format error for debug output
    fn format_error_debug(&self, error: &ValidationError) -> String {
        let mut output = self.format_error_verbose(error);
        output.push_str(&format!("\nDebug Info: {:?}", error));

        // Add stack trace context if available
        output.push_str("\nError Chain:");
        let mut current_error: &dyn std::error::Error = error;
        let mut level = 0;
        while let Some(source) = current_error.source() {
            output.push_str(&format!("\n  {}: {}", level + 1, source));
            current_error = source;
            level += 1;
        }

        output
    }

    /// Get helpful suggestions for configuration errors
    fn get_config_help(&self, error: &ConfigError) -> String {
        match error {
            ConfigError::FileNotFound { path } => {
                format!("Try creating a configuration file at: {}", path.display())
            }
            ConfigError::InvalidFormat { .. } => {
                "Check the configuration file syntax (TOML/JSON format expected)".to_string()
            }
            ConfigError::MissingField { field } => {
                format!("Add the required field '{}' to your configuration", field)
            }
            ConfigError::InvalidValue {
                field,
                value,
                reason,
            } => {
                format!(
                    "Fix the value for '{}': current='{}', reason: {}",
                    field, value, reason
                )
            }
            ConfigError::MergeConflict { .. } => {
                "Resolve conflicting configuration values between file, environment, and CLI"
                    .to_string()
            }
        }
    }
}

/// Summary of validation results for reporting
#[derive(Debug, Clone)]
pub struct ValidationSummary {
    pub total_files: usize,
    pub valid_count: usize,
    pub invalid_count: usize,
    pub error_count: usize,
    pub duration: std::time::Duration,
    pub schemas_cached: usize,
    pub memory_usage_mb: u64,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

impl ValidationSummary {
    /// Create a new validation summary
    pub fn new() -> Self {
        Self {
            total_files: 0,
            valid_count: 0,
            invalid_count: 0,
            error_count: 0,
            duration: std::time::Duration::new(0, 0),
            schemas_cached: 0,
            memory_usage_mb: 0,
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// Check if validation was successful (no errors)
    pub fn is_successful(&self) -> bool {
        self.error_count == 0
    }

    /// Get success rate as percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.valid_count as f64 / self.total_files as f64) * 100.0
        }
    }
}

impl Default for ValidationSummary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_error_reporter_verbosity_levels() {
        let reporter_quiet = ErrorReporter::new(VerbosityLevel::Quiet);
        let reporter_normal = ErrorReporter::new(VerbosityLevel::Normal);
        let reporter_verbose = ErrorReporter::new(VerbosityLevel::Verbose);
        let reporter_debug = ErrorReporter::new(VerbosityLevel::Debug);

        assert_eq!(reporter_quiet.verbosity, VerbosityLevel::Quiet);
        assert_eq!(reporter_normal.verbosity, VerbosityLevel::Normal);
        assert_eq!(reporter_verbose.verbosity, VerbosityLevel::Verbose);
        assert_eq!(reporter_debug.verbosity, VerbosityLevel::Debug);
    }

    #[test]
    fn test_validation_summary_success_rate() {
        let mut summary = ValidationSummary::new();
        summary.total_files = 10;
        summary.valid_count = 8;
        summary.invalid_count = 1;
        summary.error_count = 1;

        assert_eq!(summary.success_rate(), 80.0);
        assert!(!summary.is_successful());

        summary.error_count = 0;
        assert!(summary.is_successful());
    }

    #[test]
    fn test_critical_error_detection() {
        let reporter = ErrorReporter::new(VerbosityLevel::Quiet);

        let config_error = ValidationError::Config("test".to_string());
        let validation_error = ValidationError::ValidationFailed {
            file: PathBuf::from("test.xml"),
            details: "test".to_string(),
        };

        assert!(reporter.is_critical_error(&config_error));
        assert!(!reporter.is_critical_error(&validation_error));
    }
}
