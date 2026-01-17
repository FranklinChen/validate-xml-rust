use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Verbosity levels for output
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum VerbosityLevel {
    /// Only show critical errors
    Quiet,
    /// Show standard information
    #[default]
    Normal,
    /// Show detailed information
    Verbose,
    /// Show all available debugging information
    Debug,
}

/// Main application configuration derived from CLI
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub path: PathBuf,
    pub extensions: Vec<String>,
    pub threads: usize,
    pub verbose: bool,
    pub quiet: bool,
    pub cache_dir: PathBuf,
    pub cache_ttl_hours: u64,
    pub timeout_seconds: u64,
    pub retry_attempts: u32,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub progress: bool,
    pub fail_fast: bool,
    pub max_cache_size_mb: u64,
}

impl Config {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            path: cli.path.clone(),
            extensions: cli.get_extensions(),
            threads: cli.get_thread_count(),
            verbose: cli.verbose,
            quiet: cli.quiet,
            cache_dir: cli.get_cache_dir(),
            cache_ttl_hours: cli.cache_ttl,
            timeout_seconds: cli.timeout,
            retry_attempts: cli.retry_attempts,
            include_patterns: cli.include_patterns.clone(),
            exclude_patterns: cli.exclude_patterns.clone(),
            progress: cli.progress || (atty::is(atty::Stream::Stderr) && !cli.quiet),
            fail_fast: cli.fail_fast,
            max_cache_size_mb: cli.max_cache_size,
        }
    }

    pub fn verbosity(&self) -> VerbosityLevel {
        if self.quiet {
            VerbosityLevel::Quiet
        } else if self.verbose {
            VerbosityLevel::Verbose
        } else {
            VerbosityLevel::Normal
        }
    }
}

/// High-performance XML validation tool
#[derive(Parser, Debug, Clone)]
#[command(name = "validate-xml")]
#[command(about = "Validate XML files against their schemas with high performance and caching")]
#[command(version)]
pub struct Cli {
    /// Path to scan for XML files (directory or file)
    #[arg(help = "Directory or file to validate")]
    pub path: PathBuf,

    /// File extensions to process (comma-separated)
    #[arg(
        short = 'e',
        long = "extensions",
        default_value = "xml",
        help = "File extensions to process (e.g., 'xml,cmdi')"
    )]
    pub extensions: String,

    /// Number of concurrent validation threads
    #[arg(
        short = 't',
        long = "threads",
        help = "Number of concurrent validation threads"
    )]
    pub threads: Option<usize>,

    /// Enable verbose output
    #[arg(short = 'v', long = "verbose", help = "Enable verbose output")]
    pub verbose: bool,

    /// Enable quiet mode (errors only)
    #[arg(
        short = 'q',
        long = "quiet",
        help = "Quiet mode",
        conflicts_with = "verbose"
    )]
    pub quiet: bool,

    /// Cache directory for schemas
    #[arg(long = "cache-dir")]
    pub cache_dir: Option<PathBuf>,

    /// Cache TTL in hours
    #[arg(long = "cache-ttl", default_value = "24")]
    pub cache_ttl: u64,

    /// HTTP request timeout in seconds
    #[arg(long = "timeout", default_value = "30")]
    pub timeout: u64,

    /// Number of retry attempts for failed downloads
    #[arg(long = "retry-attempts", default_value = "3")]
    pub retry_attempts: u32,

    /// Include file patterns (glob syntax)
    #[arg(long = "include", action = clap::ArgAction::Append)]
    pub include_patterns: Vec<String>,

    /// Exclude file patterns (glob syntax)
    #[arg(long = "exclude", action = clap::ArgAction::Append)]
    pub exclude_patterns: Vec<String>,

    /// Show progress indicators
    #[arg(long = "progress")]
    pub progress: bool,

    /// Fail fast on first validation error
    #[arg(long = "fail-fast")]
    pub fail_fast: bool,

    /// Maximum cache size in MB
    #[arg(long = "max-cache-size", default_value = "100")]
    pub max_cache_size: u64,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }

    pub fn get_extensions(&self) -> Vec<String> {
        self.extensions
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.path.exists() {
            return Err(format!("Path does not exist: {}", self.path.display()));
        }
        if let Some(threads) = self.threads
            && threads == 0
        {
            return Err("Number of threads must be greater than 0".to_string());
        }
        Ok(())
    }

    pub fn get_thread_count(&self) -> usize {
        self.threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
    }

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

    #[test]
    fn test_basic_cli_parsing() {
        let args = vec!["validate-xml", "/tmp"];
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(cli.path, PathBuf::from("/tmp"));
    }
}
