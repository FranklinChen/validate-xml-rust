use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// High-performance XML validation tool
#[derive(Parser, Debug, Clone)]
#[command(name = "validate-xml")]
#[command(about = "Validate XML files against their schemas with high performance and caching")]
#[command(long_about = "
A high-performance XML validation tool that validates XML files against XML Schema (XSD) definitions.
Features concurrent validation, schema caching, and support for both local and remote schemas.

EXAMPLES:
    validate-xml /path/to/xml/files
    validate-xml --extensions xml,xsd /path/to/files
    validate-xml --threads 8 --verbose /path/to/files
    validate-xml --config myconfig.toml /path/to/files
    validate-xml --cache-dir /tmp/schemas --cache-ttl 48 /path/to/files
")]
#[command(version)]
pub struct Cli {
    /// Directory to scan for XML files
    #[arg(help = "Directory containing XML files to validate")]
    pub directory: PathBuf,

    /// File extensions to process (comma-separated)
    #[arg(
        short = 'e',
        long = "extensions",
        value_name = "EXT1,EXT2",
        default_value = "xml",
        help = "File extensions to process (comma-separated, e.g., 'xml,cmdi')"
    )]
    pub extensions: String,

    /// Number of concurrent validation threads
    #[arg(
        short = 't',
        long = "threads",
        value_name = "N",
        help = "Number of concurrent validation threads (default: number of CPU cores)"
    )]
    pub threads: Option<usize>,

    /// Enable verbose output
    #[arg(
        short = 'v',
        long = "verbose",
        help = "Enable verbose output with detailed validation information"
    )]
    pub verbose: bool,

    /// Enable quiet mode (errors only)
    #[arg(
        short = 'q',
        long = "quiet",
        help = "Quiet mode - only show errors and final summary",
        conflicts_with = "verbose"
    )]
    pub quiet: bool,

    /// Configuration file path
    #[arg(
        short = 'c',
        long = "config",
        value_name = "FILE",
        help = "Path to configuration file (TOML format)"
    )]
    pub config: Option<PathBuf>,

    /// Cache directory for schemas
    #[arg(
        long = "cache-dir",
        value_name = "DIR",
        help = "Directory for caching downloaded schemas"
    )]
    pub cache_dir: Option<PathBuf>,

    /// Cache TTL in hours
    #[arg(
        long = "cache-ttl",
        value_name = "HOURS",
        default_value = "24",
        help = "Time-to-live for cached schemas in hours"
    )]
    pub cache_ttl: u64,

    /// HTTP request timeout in seconds
    #[arg(
        long = "timeout",
        value_name = "SECONDS",
        default_value = "30",
        help = "HTTP request timeout in seconds for downloading remote schemas"
    )]
    pub timeout: u64,

    /// Number of retry attempts for failed downloads
    #[arg(
        long = "retry-attempts",
        value_name = "N",
        default_value = "3",
        help = "Number of retry attempts for failed schema downloads"
    )]
    pub retry_attempts: u32,

    /// Include file patterns (glob syntax)
    #[arg(
        long = "include",
        value_name = "PATTERN",
        help = "Include files matching this glob pattern (can be used multiple times)",
        action = clap::ArgAction::Append
    )]
    pub include_patterns: Vec<String>,

    /// Exclude file patterns (glob syntax)
    #[arg(
        long = "exclude",
        value_name = "PATTERN",
        help = "Exclude files matching this glob pattern (can be used multiple times)",
        action = clap::ArgAction::Append
    )]
    pub exclude_patterns: Vec<String>,

    /// Output format
    #[arg(
        short = 'f',
        long = "format",
        value_enum,
        default_value = "human",
        help = "Output format for validation results"
    )]
    pub output_format: OutputFormat,

    /// Show progress indicators
    #[arg(
        long = "progress",
        help = "Show progress indicators for long-running operations"
    )]
    pub progress: bool,

    /// Fail fast on first validation error
    #[arg(
        long = "fail-fast",
        help = "Stop validation on first error encountered"
    )]
    pub fail_fast: bool,

    /// Maximum cache size in MB
    #[arg(
        long = "max-cache-size",
        value_name = "MB",
        default_value = "100",
        help = "Maximum cache size in megabytes"
    )]
    pub max_cache_size: u64,
}

/// Output format options
#[derive(ValueEnum, Debug, Clone, PartialEq)]
pub enum OutputFormat {
    /// Human-readable output
    Human,
    /// JSON output for machine processing
    Json,
    /// Compact summary output
    Summary,
}

impl Cli {
    /// Parse command line arguments
    pub fn parse_args() -> Self {
        Self::parse()
    }

    /// Get file extensions as a vector
    pub fn get_extensions(&self) -> Vec<String> {
        self.extensions
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Validate CLI arguments
    pub fn validate(&self) -> Result<(), String> {
        // Validate directory exists
        if !self.directory.exists() {
            return Err(format!(
                "Directory does not exist: {}",
                self.directory.display()
            ));
        }

        if !self.directory.is_dir() {
            return Err(format!(
                "Path is not a directory: {}",
                self.directory.display()
            ));
        }

        // Validate threads
        if let Some(threads) = self.threads {
            if threads == 0 {
                return Err("Number of threads must be greater than 0".to_string());
            }
            if threads > 1000 {
                return Err("Number of threads cannot exceed 1000".to_string());
            }
        }

        // Validate cache TTL
        if self.cache_ttl == 0 {
            return Err("Cache TTL must be greater than 0".to_string());
        }

        // Validate timeout
        if self.timeout == 0 {
            return Err("Timeout must be greater than 0".to_string());
        }

        // Validate extensions
        let extensions = self.get_extensions();
        if extensions.is_empty() {
            return Err("At least one file extension must be specified".to_string());
        }

        // Validate that extensions don't contain invalid characters
        for ext in &extensions {
            if ext.contains('/') || ext.contains('\\') || ext.contains('.') {
                return Err(format!("Invalid file extension: {}", ext));
            }
        }

        // Validate config file exists if specified
        if let Some(config_path) = &self.config
            && !config_path.exists()
        {
            return Err(format!(
                "Configuration file does not exist: {}",
                config_path.display()
            ));
        }

        // Validate cache directory is writable if specified
        if let Some(cache_dir) = &self.cache_dir
            && cache_dir.exists()
            && !cache_dir.is_dir()
        {
            return Err(format!(
                "Cache path is not a directory: {}",
                cache_dir.display()
            ));
        }

        Ok(())
    }

    /// Get the number of threads to use (default to number of CPU cores)
    pub fn get_thread_count(&self) -> usize {
        self.threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
    }

    /// Check if verbose mode is enabled
    pub fn is_verbose(&self) -> bool {
        self.verbose && !self.quiet
    }

    /// Check if quiet mode is enabled
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Get cache directory with default
    pub fn get_cache_dir(&self) -> PathBuf {
        self.cache_dir.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("validate-xml")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_basic_cli_parsing() {
        let args = vec!["validate-xml", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.directory, PathBuf::from("/tmp"));
        assert_eq!(cli.extensions, "xml");
        assert!(!cli.verbose);
        assert!(!cli.quiet);
        assert_eq!(cli.cache_ttl, 24);
        assert_eq!(cli.timeout, 30);
        assert_eq!(cli.retry_attempts, 3);
    }

    #[test]
    fn test_all_options() {
        let temp_dir = TempDir::new().unwrap();
        let config_file = temp_dir.path().join("config.toml");
        fs::write(&config_file, "# test config").unwrap();

        let args = vec![
            "validate-xml",
            "--extensions",
            "xml,cmdi,xsd",
            "--threads",
            "8",
            "--verbose",
            "--config",
            config_file.to_str().unwrap(),
            "--cache-dir",
            "/tmp/cache",
            "--cache-ttl",
            "48",
            "--timeout",
            "60",
            "--retry-attempts",
            "5",
            "--include",
            "*.xml",
            "--include",
            "test_*.cmdi",
            "--exclude",
            "temp_*",
            "--format",
            "json",
            "--progress",
            "--fail-fast",
            "--max-cache-size",
            "200",
            temp_dir.path().to_str().unwrap(),
        ];

        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.get_extensions(), vec!["xml", "cmdi", "xsd"]);
        assert_eq!(cli.threads, Some(8));
        assert!(cli.verbose);
        assert!(!cli.quiet);
        assert_eq!(cli.config, Some(config_file));
        assert_eq!(cli.cache_dir, Some(PathBuf::from("/tmp/cache")));
        assert_eq!(cli.cache_ttl, 48);
        assert_eq!(cli.timeout, 60);
        assert_eq!(cli.retry_attempts, 5);
        assert_eq!(cli.include_patterns, vec!["*.xml", "test_*.cmdi"]);
        assert_eq!(cli.exclude_patterns, vec!["temp_*"]);
        assert_eq!(cli.output_format, OutputFormat::Json);
        assert!(cli.progress);
        assert!(cli.fail_fast);
        assert_eq!(cli.max_cache_size, 200);
    }

    #[test]
    fn test_conflicting_verbose_quiet() {
        let args = vec!["validate-xml", "--verbose", "--quiet", "/tmp"];
        let result = Cli::try_parse_from(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_extensions() {
        let args = vec!["validate-xml", "--extensions", "xml,cmdi, xsd ,txt", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.get_extensions(), vec!["xml", "cmdi", "xsd", "txt"]);
    }

    #[test]
    fn test_get_extensions_empty() {
        let args = vec!["validate-xml", "--extensions", "", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert!(cli.get_extensions().is_empty());
    }

    #[test]
    fn test_get_thread_count_default() {
        let args = vec!["validate-xml", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();

        let thread_count = cli.get_thread_count();
        assert!(thread_count >= 1);
    }

    #[test]
    fn test_get_thread_count_specified() {
        let args = vec!["validate-xml", "--threads", "16", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.get_thread_count(), 16);
    }

    #[test]
    fn test_validation_nonexistent_directory() {
        let args = vec!["validate-xml", "/nonexistent/directory"];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Directory does not exist"));
    }

    #[test]
    fn test_validation_file_as_directory() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test_file.txt");
        fs::write(&file_path, "test").unwrap();

        let args = vec!["validate-xml", file_path.to_str().unwrap()];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path is not a directory"));
    }

    #[test]
    fn test_validation_zero_threads() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--threads",
            "0",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Number of threads must be greater than 0")
        );
    }

    #[test]
    fn test_validation_too_many_threads() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--threads",
            "1001",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Number of threads cannot exceed 1000")
        );
    }

    #[test]
    fn test_validation_zero_cache_ttl() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--cache-ttl",
            "0",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cache TTL must be greater than 0")
        );
    }

    #[test]
    fn test_validation_zero_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--timeout",
            "0",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Timeout must be greater than 0")
        );
    }

    #[test]
    fn test_validation_invalid_extensions() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--extensions",
            "xml,invalid/ext",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid file extension"));
    }

    #[test]
    fn test_validation_nonexistent_config() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--config",
            "/nonexistent/config.toml",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Configuration file does not exist")
        );
    }

    #[test]
    fn test_is_verbose_and_quiet() {
        let temp_dir = TempDir::new().unwrap();

        // Test verbose mode
        let args = vec![
            "validate-xml",
            "--verbose",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.is_verbose());
        assert!(!cli.is_quiet());

        // Test quiet mode
        let args = vec!["validate-xml", "--quiet", temp_dir.path().to_str().unwrap()];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(!cli.is_verbose());
        assert!(cli.is_quiet());

        // Test default (neither verbose nor quiet)
        let args = vec!["validate-xml", temp_dir.path().to_str().unwrap()];
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(!cli.is_verbose());
        assert!(!cli.is_quiet());
    }

    #[test]
    fn test_get_cache_dir_default() {
        let temp_dir = TempDir::new().unwrap();
        let args = vec!["validate-xml", temp_dir.path().to_str().unwrap()];
        let cli = Cli::try_parse_from(args).unwrap();

        let cache_dir = cli.get_cache_dir();
        assert!(cache_dir.to_string_lossy().contains("validate-xml"));
    }

    #[test]
    fn test_get_cache_dir_specified() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = "/tmp/custom-cache";
        let args = vec![
            "validate-xml",
            "--cache-dir",
            cache_path,
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        assert_eq!(cli.get_cache_dir(), PathBuf::from(cache_path));
    }

    #[test]
    fn test_output_format_parsing() {
        let temp_dir = TempDir::new().unwrap();

        // Test human format (default)
        let args = vec!["validate-xml", temp_dir.path().to_str().unwrap()];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.output_format, OutputFormat::Human);

        // Test JSON format
        let args = vec![
            "validate-xml",
            "--format",
            "json",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.output_format, OutputFormat::Json);

        // Test summary format
        let args = vec![
            "validate-xml",
            "--format",
            "summary",
            temp_dir.path().to_str().unwrap(),
        ];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.output_format, OutputFormat::Summary);
    }

    #[test]
    fn test_help_text_generation() {
        // This test ensures that help text can be generated without panicking
        let result = Cli::try_parse_from(vec!["validate-xml", "--help"]);
        assert!(result.is_err()); // --help causes clap to exit with help text

        // The error should be a help display error, not a parsing error
        match result {
            Err(e) => assert_eq!(e.kind(), clap::error::ErrorKind::DisplayHelp),
            Ok(_) => panic!("Expected help error"),
        }
    }

    #[test]
    fn test_version_display() {
        // This test ensures that version can be displayed without panicking
        let result = Cli::try_parse_from(vec!["validate-xml", "--version"]);
        assert!(result.is_err()); // --version causes clap to exit with version

        // The error should be a version display error
        match result {
            Err(e) => assert_eq!(e.kind(), clap::error::ErrorKind::DisplayVersion),
            Ok(_) => panic!("Expected version error"),
        }
    }
}
