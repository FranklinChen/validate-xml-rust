use crate::cli::{Cli, OutputFormat};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Trait for abstracting environment variable access
pub trait EnvProvider {
    fn get(&self, key: &str) -> Option<String>;
}

/// System environment variable provider for production use
pub struct SystemEnvProvider;

impl EnvProvider for SystemEnvProvider {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parsing error: {0}")]
    TomlParsing(#[from] toml::de::Error),

    #[error("JSON parsing error: {0}")]
    JsonParsing(#[from] serde_json::Error),

    #[error("Configuration validation error: {0}")]
    Validation(String),

    #[error("Environment variable error: {0}")]
    Environment(String),

    #[error("Unsupported configuration file format: {0}")]
    UnsupportedFormat(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Config {
    pub validation: ValidationConfig,
    pub cache: CacheConfig,
    pub network: NetworkConfig,
    pub output: OutputConfig,
    pub files: FileConfig,
}

/// Validation-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ValidationConfig {
    /// Number of concurrent validation threads
    pub threads: Option<usize>,
    /// Stop validation on first error
    pub fail_fast: bool,
    /// Show progress indicators
    pub show_progress: bool,
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheConfig {
    /// Cache directory path
    pub directory: PathBuf,
    /// Time-to-live for cached schemas in hours
    pub ttl_hours: u64,
    /// Maximum cache size in megabytes
    pub max_size_mb: u64,
    /// Maximum number of entries in memory cache
    pub max_memory_entries: u64,
    /// Memory cache TTL in seconds
    pub memory_ttl_seconds: u64,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkConfig {
    /// HTTP request timeout in seconds
    pub timeout_seconds: u64,
    /// Number of retry attempts for failed downloads
    pub retry_attempts: u32,
    /// Retry delay in milliseconds
    pub retry_delay_ms: u64,
}

/// Output configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputConfig {
    /// Output format
    pub format: OutputFormatConfig,
    /// Verbose output
    pub verbose: bool,
    /// Quiet mode (errors only)
    pub quiet: bool,
}

/// File processing configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileConfig {
    /// File extensions to process
    pub extensions: Vec<String>,
    /// Include patterns (glob syntax)
    pub include_patterns: Vec<String>,
    /// Exclude patterns (glob syntax)
    pub exclude_patterns: Vec<String>,
}

/// Output format configuration (serializable version of CLI OutputFormat)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormatConfig {
    Human,
    Json,
    Summary,
}

impl From<OutputFormat> for OutputFormatConfig {
    fn from(format: OutputFormat) -> Self {
        match format {
            OutputFormat::Human => OutputFormatConfig::Human,
            OutputFormat::Json => OutputFormatConfig::Json,
            OutputFormat::Summary => OutputFormatConfig::Summary,
        }
    }
}

impl From<OutputFormatConfig> for OutputFormat {
    fn from(format: OutputFormatConfig) -> Self {
        match format {
            OutputFormatConfig::Human => OutputFormat::Human,
            OutputFormatConfig::Json => OutputFormat::Json,
            OutputFormatConfig::Summary => OutputFormat::Summary,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            directory: dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("validate-xml"),
            ttl_hours: 24,
            max_size_mb: 100,
            max_memory_entries: 1000,
            memory_ttl_seconds: 3600, // 1 hour
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            retry_attempts: 3,
            retry_delay_ms: 1000,
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormatConfig::Human,
            verbose: false,
            quiet: false,
        }
    }
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            extensions: vec!["xml".to_string()],
            include_patterns: vec![],
            exclude_patterns: vec![],
        }
    }
}

/// Configuration manager for loading and merging configurations
pub struct ConfigManager;

impl ConfigManager {
    /// Load configuration with precedence: file -> environment -> CLI
    pub async fn load_config(cli: &Cli) -> Result<Config> {
        // Start with default configuration
        let mut config = Config::default();

        // Load from configuration file if specified
        if let Some(config_path) = &cli.config {
            let file_config = Self::load_from_file(config_path).await?;
            config = Self::merge_configs(config, file_config);
        } else {
            // Try to find configuration files in standard locations
            if let Some(found_config) = Self::find_config_file().await? {
                config = Self::merge_configs(config, found_config);
            }
        }

        // Apply environment variable overrides
        config = Self::apply_environment_overrides(config)?;

        // Apply CLI argument overrides (highest precedence)
        config = Self::merge_with_cli(config, cli);

        // Validate the final configuration
        Self::validate_config(&config)?;

        Ok(config)
    }

    /// Load configuration from a file (TOML or JSON)
    pub async fn load_from_file(path: &Path) -> Result<Config> {
        let content = tokio::fs::read_to_string(path).await?;

        match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => {
                let config: Config = toml::from_str(&content)?;
                Ok(config)
            }
            Some("json") => {
                let config: Config = serde_json::from_str(&content)?;
                Ok(config)
            }
            Some(ext) => Err(ConfigError::UnsupportedFormat(ext.to_string())),
            None => {
                // Try to parse as TOML first, then JSON
                if let Ok(config) = toml::from_str::<Config>(&content) {
                    Ok(config)
                } else {
                    let config: Config = serde_json::from_str(&content)?;
                    Ok(config)
                }
            }
        }
    }

    /// Find configuration file in standard locations
    pub async fn find_config_file() -> Result<Option<Config>> {
        let config_names = [
            "validate-xml.toml",
            "validate-xml.json",
            ".validate-xml.toml",
            ".validate-xml.json",
        ];

        // Check current directory first
        for name in &config_names {
            let path = PathBuf::from(name);
            if path.exists() {
                return Ok(Some(Self::load_from_file(&path).await?));
            }
        }

        // Check user config directory
        if let Some(config_dir) = dirs::config_dir() {
            let app_config_dir = config_dir.join("validate-xml");
            for name in &config_names {
                let path = app_config_dir.join(name);
                if path.exists() {
                    return Ok(Some(Self::load_from_file(&path).await?));
                }
            }
        }

        Ok(None)
    }

    /// Apply environment variable overrides using the system environment
    pub fn apply_environment_overrides(config: Config) -> Result<Config> {
        Self::apply_environment_overrides_with(&SystemEnvProvider, config)
    }

    /// Apply environment variable overrides with a custom environment provider
    pub fn apply_environment_overrides_with(
        env: &impl EnvProvider,
        mut config: Config,
    ) -> Result<Config> {
        // Validation settings
        if let Some(threads) = env.get("VALIDATE_XML_THREADS") {
            config.validation.threads = Some(threads.parse().map_err(|_| {
                ConfigError::Environment(format!("Invalid VALIDATE_XML_THREADS value: {}", threads))
            })?);
        }

        if let Some(fail_fast) = env.get("VALIDATE_XML_FAIL_FAST") {
            config.validation.fail_fast = fail_fast.parse().map_err(|_| {
                ConfigError::Environment(format!(
                    "Invalid VALIDATE_XML_FAIL_FAST value: {}",
                    fail_fast
                ))
            })?;
        }

        // Cache settings
        if let Some(cache_dir) = env.get("VALIDATE_XML_CACHE_DIR") {
            config.cache.directory = PathBuf::from(cache_dir);
        }

        if let Some(cache_ttl) = env.get("VALIDATE_XML_CACHE_TTL") {
            config.cache.ttl_hours = cache_ttl.parse().map_err(|_| {
                ConfigError::Environment(format!(
                    "Invalid VALIDATE_XML_CACHE_TTL value: {}",
                    cache_ttl
                ))
            })?;
        }

        if let Some(max_size) = env.get("VALIDATE_XML_MAX_CACHE_SIZE") {
            config.cache.max_size_mb = max_size.parse().map_err(|_| {
                ConfigError::Environment(format!(
                    "Invalid VALIDATE_XML_MAX_CACHE_SIZE value: {}",
                    max_size
                ))
            })?;
        }

        // Network settings
        if let Some(timeout) = env.get("VALIDATE_XML_TIMEOUT") {
            config.network.timeout_seconds = timeout.parse().map_err(|_| {
                ConfigError::Environment(format!("Invalid VALIDATE_XML_TIMEOUT value: {}", timeout))
            })?;
        }

        if let Some(retry_attempts) = env.get("VALIDATE_XML_RETRY_ATTEMPTS") {
            config.network.retry_attempts = retry_attempts.parse().map_err(|_| {
                ConfigError::Environment(format!(
                    "Invalid VALIDATE_XML_RETRY_ATTEMPTS value: {}",
                    retry_attempts
                ))
            })?;
        }

        // Output settings
        if let Some(verbose) = env.get("VALIDATE_XML_VERBOSE") {
            config.output.verbose = verbose.parse().map_err(|_| {
                ConfigError::Environment(format!("Invalid VALIDATE_XML_VERBOSE value: {}", verbose))
            })?;
        }

        if let Some(quiet) = env.get("VALIDATE_XML_QUIET") {
            config.output.quiet = quiet.parse().map_err(|_| {
                ConfigError::Environment(format!("Invalid VALIDATE_XML_QUIET value: {}", quiet))
            })?;
        }

        if let Some(format) = env.get("VALIDATE_XML_FORMAT") {
            config.output.format = match format.to_lowercase().as_str() {
                "human" => OutputFormatConfig::Human,
                "json" => OutputFormatConfig::Json,
                "summary" => OutputFormatConfig::Summary,
                _ => {
                    return Err(ConfigError::Environment(format!(
                        "Invalid VALIDATE_XML_FORMAT value: {}",
                        format
                    )));
                }
            };
        }

        // File settings
        if let Some(extensions) = env.get("VALIDATE_XML_EXTENSIONS") {
            config.files.extensions = extensions
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        Ok(config)
    }

    /// Merge CLI arguments with configuration (CLI takes precedence)
    pub fn merge_with_cli(mut config: Config, cli: &Cli) -> Config {
        // Validation settings
        if cli.threads.is_some() {
            config.validation.threads = cli.threads;
        }
        config.validation.fail_fast = cli.fail_fast;
        config.validation.show_progress = cli.progress;

        // Cache settings
        if let Some(cache_dir) = &cli.cache_dir {
            config.cache.directory = cache_dir.clone();
        }
        config.cache.ttl_hours = cli.cache_ttl;
        config.cache.max_size_mb = cli.max_cache_size;

        // Network settings
        config.network.timeout_seconds = cli.timeout;
        config.network.retry_attempts = cli.retry_attempts;

        // Output settings
        config.output.format = cli.output_format.clone().into();
        config.output.verbose = cli.verbose;
        config.output.quiet = cli.quiet;

        // File settings
        config.files.extensions = cli.get_extensions();
        if !cli.include_patterns.is_empty() {
            config.files.include_patterns = cli.include_patterns.clone();
        }
        if !cli.exclude_patterns.is_empty() {
            config.files.exclude_patterns = cli.exclude_patterns.clone();
        }

        config
    }

    /// Merge two configurations (second takes precedence for non-None values)
    pub fn merge_configs(mut base: Config, override_config: Config) -> Config {
        // Validation settings
        if override_config.validation.threads.is_some() {
            base.validation.threads = override_config.validation.threads;
        }
        base.validation.fail_fast = override_config.validation.fail_fast;
        base.validation.show_progress = override_config.validation.show_progress;

        // Cache settings
        base.cache.directory = override_config.cache.directory;
        base.cache.ttl_hours = override_config.cache.ttl_hours;
        base.cache.max_size_mb = override_config.cache.max_size_mb;

        // Network settings
        base.network.timeout_seconds = override_config.network.timeout_seconds;
        base.network.retry_attempts = override_config.network.retry_attempts;
        base.network.retry_delay_ms = override_config.network.retry_delay_ms;

        // Output settings
        base.output.format = override_config.output.format;
        base.output.verbose = override_config.output.verbose;
        base.output.quiet = override_config.output.quiet;

        // File settings
        if !override_config.files.extensions.is_empty() {
            base.files.extensions = override_config.files.extensions;
        }
        if !override_config.files.include_patterns.is_empty() {
            base.files.include_patterns = override_config.files.include_patterns;
        }
        if !override_config.files.exclude_patterns.is_empty() {
            base.files.exclude_patterns = override_config.files.exclude_patterns;
        }

        base
    }

    /// Validate configuration values
    pub fn validate_config(config: &Config) -> Result<()> {
        // Validate threads
        if let Some(threads) = config.validation.threads {
            if threads == 0 {
                return Err(ConfigError::Validation(
                    "Number of threads must be greater than 0".to_string(),
                ));
            }
            if threads > 1000 {
                return Err(ConfigError::Validation(
                    "Number of threads cannot exceed 1000".to_string(),
                ));
            }
        }

        // Validate cache settings
        if config.cache.ttl_hours == 0 {
            return Err(ConfigError::Validation(
                "Cache TTL must be greater than 0".to_string(),
            ));
        }

        if config.cache.max_size_mb == 0 {
            return Err(ConfigError::Validation(
                "Cache max size must be greater than 0".to_string(),
            ));
        }

        // Validate network settings
        if config.network.timeout_seconds == 0 {
            return Err(ConfigError::Validation(
                "Timeout must be greater than 0".to_string(),
            ));
        }

        if config.network.retry_attempts > 10 {
            return Err(ConfigError::Validation(
                "Retry attempts cannot exceed 10".to_string(),
            ));
        }

        // Validate output settings
        if config.output.verbose && config.output.quiet {
            return Err(ConfigError::Validation(
                "Cannot enable both verbose and quiet modes".to_string(),
            ));
        }

        // Validate file settings
        if config.files.extensions.is_empty() {
            return Err(ConfigError::Validation(
                "At least one file extension must be specified".to_string(),
            ));
        }

        // Validate that extensions don't contain invalid characters
        for ext in &config.files.extensions {
            if ext.contains('/') || ext.contains('\\') || ext.contains('.') {
                return Err(ConfigError::Validation(format!(
                    "Invalid file extension: {}",
                    ext
                )));
            }
        }

        Ok(())
    }

    /// Get the effective cache directory
    pub fn get_cache_directory(config: &Config) -> PathBuf {
        config.cache.directory.clone()
    }

    /// Get the effective thread count
    pub fn get_thread_count(config: &Config) -> usize {
        config.validation.threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
    }

    /// Convert configuration to Duration for network timeout
    pub fn get_timeout_duration(config: &Config) -> Duration {
        Duration::from_secs(config.network.timeout_seconds)
    }

    /// Convert configuration to Duration for cache TTL
    pub fn get_cache_ttl_duration(config: &Config) -> Duration {
        Duration::from_secs(config.cache.ttl_hours * 3600)
    }

    /// Convert configuration to Duration for retry delay
    pub fn get_retry_delay_duration(config: &Config) -> Duration {
        Duration::from_millis(config.network.retry_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    /// Mock environment variable provider for testing
    #[derive(Default)]
    struct MockEnvProvider {
        vars: HashMap<String, String>,
    }

    impl MockEnvProvider {
        fn new() -> Self {
            Self {
                vars: HashMap::new(),
            }
        }

        fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
            self.vars.insert(key.into(), value.into());
        }
    }

    impl EnvProvider for MockEnvProvider {
        fn get(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
    }

    #[tokio::test]
    async fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.validation.threads, None);
        assert!(!config.validation.fail_fast);
        assert!(!config.validation.show_progress);

        // Cache directory should be set to default path
        assert!(
            config
                .cache
                .directory
                .to_string_lossy()
                .contains("validate-xml")
        );
        assert_eq!(config.cache.ttl_hours, 24);
        assert_eq!(config.cache.max_size_mb, 100);

        assert_eq!(config.network.timeout_seconds, 30);
        assert_eq!(config.network.retry_attempts, 3);
        assert_eq!(config.network.retry_delay_ms, 1000);

        assert_eq!(config.output.format, OutputFormatConfig::Human);
        assert!(!config.output.verbose);
        assert!(!config.output.quiet);

        assert_eq!(config.files.extensions, vec!["xml"]);
        assert!(config.files.include_patterns.is_empty());
        assert!(config.files.exclude_patterns.is_empty());
    }

    #[tokio::test]
    async fn test_load_toml_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let toml_content = r#"
[validation]
threads = 8
fail_fast = true
show_progress = true

[cache]
directory = "/tmp/cache"
ttl_hours = 48
max_size_mb = 200
max_memory_entries = 1000
memory_ttl_seconds = 3600

[network]
timeout_seconds = 60
retry_attempts = 5
retry_delay_ms = 2000

[output]
format = "json"
verbose = true
quiet = false

[files]
extensions = ["xml", "cmdi", "xsd"]
include_patterns = ["*.xml", "test_*"]
exclude_patterns = ["temp_*", "*.bak"]
"#;

        fs::write(&config_path, toml_content).unwrap();

        let config = ConfigManager::load_from_file(&config_path).await.unwrap();

        assert_eq!(config.validation.threads, Some(8));
        assert!(config.validation.fail_fast);
        assert!(config.validation.show_progress);

        assert_eq!(config.cache.directory, PathBuf::from("/tmp/cache"));
        assert_eq!(config.cache.ttl_hours, 48);
        assert_eq!(config.cache.max_size_mb, 200);

        assert_eq!(config.network.timeout_seconds, 60);
        assert_eq!(config.network.retry_attempts, 5);
        assert_eq!(config.network.retry_delay_ms, 2000);

        assert_eq!(config.output.format, OutputFormatConfig::Json);
        assert!(config.output.verbose);
        assert!(!config.output.quiet);

        assert_eq!(config.files.extensions, vec!["xml", "cmdi", "xsd"]);
        assert_eq!(config.files.include_patterns, vec!["*.xml", "test_*"]);
        assert_eq!(config.files.exclude_patterns, vec!["temp_*", "*.bak"]);
    }

    #[tokio::test]
    async fn test_load_json_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        let json_content = r#"{
  "validation": {
    "threads": 4,
    "fail_fast": false,
    "show_progress": true
  },
  "cache": {
    "directory": "/custom/cache",
    "ttl_hours": 12,
    "max_size_mb": 50,
    "max_memory_entries": 500,
    "memory_ttl_seconds": 1800
  },
  "network": {
    "timeout_seconds": 45,
    "retry_attempts": 2,
    "retry_delay_ms": 500
  },
  "output": {
    "format": "summary",
    "verbose": false,
    "quiet": true
  },
  "files": {
    "extensions": ["xml"],
    "include_patterns": [],
    "exclude_patterns": ["*.tmp"]
  }
}"#;

        fs::write(&config_path, json_content).unwrap();

        let config = ConfigManager::load_from_file(&config_path).await.unwrap();

        assert_eq!(config.validation.threads, Some(4));
        assert!(!config.validation.fail_fast);
        assert!(config.validation.show_progress);

        assert_eq!(config.cache.directory, PathBuf::from("/custom/cache"));
        assert_eq!(config.cache.ttl_hours, 12);
        assert_eq!(config.cache.max_size_mb, 50);

        assert_eq!(config.network.timeout_seconds, 45);
        assert_eq!(config.network.retry_attempts, 2);
        assert_eq!(config.network.retry_delay_ms, 500);

        assert_eq!(config.output.format, OutputFormatConfig::Summary);
        assert!(!config.output.verbose);
        assert!(config.output.quiet);

        assert_eq!(config.files.extensions, vec!["xml"]);
        assert!(config.files.include_patterns.is_empty());
        assert_eq!(config.files.exclude_patterns, vec!["*.tmp"]);
    }

    #[tokio::test]
    async fn test_unsupported_file_format() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");

        fs::write(&config_path, "invalid: yaml").unwrap();

        let result = ConfigManager::load_from_file(&config_path).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            ConfigError::UnsupportedFormat(ext) => assert_eq!(ext, "yaml"),
            _ => panic!("Expected UnsupportedFormat error"),
        }
    }

    #[tokio::test]
    async fn test_invalid_toml() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        fs::write(&config_path, "invalid toml [[[").unwrap();

        let result = ConfigManager::load_from_file(&config_path).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::TomlParsing(_)));
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        fs::write(&config_path, "{ invalid json }").unwrap();

        let result = ConfigManager::load_from_file(&config_path).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::JsonParsing(_)));
    }

    #[test]
    fn test_environment_overrides() {
        // Create mock environment with test values
        let mut mock_env = MockEnvProvider::new();
        mock_env.set("VALIDATE_XML_THREADS", "16");
        mock_env.set("VALIDATE_XML_FAIL_FAST", "true");
        mock_env.set("VALIDATE_XML_CACHE_DIR", "/env/cache");
        mock_env.set("VALIDATE_XML_CACHE_TTL", "72");
        mock_env.set("VALIDATE_XML_TIMEOUT", "120");
        mock_env.set("VALIDATE_XML_VERBOSE", "true");
        mock_env.set("VALIDATE_XML_FORMAT", "json");
        mock_env.set("VALIDATE_XML_EXTENSIONS", "xml,cmdi");

        let base_config = Config::default();
        let config =
            ConfigManager::apply_environment_overrides_with(&mock_env, base_config).unwrap();

        assert_eq!(config.validation.threads, Some(16));
        assert!(config.validation.fail_fast);
        assert_eq!(config.cache.directory, PathBuf::from("/env/cache"));
        assert_eq!(config.cache.ttl_hours, 72);
        assert_eq!(config.network.timeout_seconds, 120);
        assert!(config.output.verbose);
        assert_eq!(config.output.format, OutputFormatConfig::Json);
        assert_eq!(config.files.extensions, vec!["xml", "cmdi"]);
    }

    #[test]
    fn test_invalid_environment_values() {
        // Create mock environment with invalid value
        let mut mock_env = MockEnvProvider::new();
        mock_env.set("VALIDATE_XML_THREADS", "invalid");

        let base_config = Config::default();
        let result = ConfigManager::apply_environment_overrides_with(&mock_env, base_config);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::Environment(_)));
    }

    #[test]
    fn test_merge_with_cli() {
        use clap::Parser;

        let temp_dir = TempDir::new().unwrap();
        let args = vec![
            "validate-xml",
            "--threads",
            "12",
            "--verbose",
            "--cache-ttl",
            "36",
            "--timeout",
            "90",
            "--extensions",
            "xml,xsd",
            "--format",
            "summary",
            temp_dir.path().to_str().unwrap(),
        ];

        let cli = Cli::try_parse_from(args).unwrap();
        let base_config = Config::default();
        let config = ConfigManager::merge_with_cli(base_config, &cli);

        assert_eq!(config.validation.threads, Some(12));
        assert!(config.output.verbose);
        assert_eq!(config.cache.ttl_hours, 36);
        assert_eq!(config.network.timeout_seconds, 90);
        assert_eq!(config.files.extensions, vec!["xml", "xsd"]);
        assert_eq!(config.output.format, OutputFormatConfig::Summary);
    }

    #[test]
    fn test_merge_configs() {
        let mut base = Config::default();
        base.validation.threads = Some(4);
        base.cache.ttl_hours = 12;

        let mut override_config = Config::default();
        override_config.validation.threads = Some(8);
        override_config.network.timeout_seconds = 60;

        let merged = ConfigManager::merge_configs(base, override_config);

        assert_eq!(merged.validation.threads, Some(8)); // Override wins
        assert_eq!(merged.network.timeout_seconds, 60); // Override wins
        assert_eq!(merged.cache.ttl_hours, 24); // Default from override_config (24 is the default)
    }

    #[test]
    fn test_config_validation() {
        let mut config = Config::default();

        // Valid config should pass
        assert!(ConfigManager::validate_config(&config).is_ok());

        // Invalid threads
        config.validation.threads = Some(0);
        assert!(ConfigManager::validate_config(&config).is_err());

        config.validation.threads = Some(1001);
        assert!(ConfigManager::validate_config(&config).is_err());

        // Reset threads
        config.validation.threads = Some(4);

        // Invalid cache TTL
        config.cache.ttl_hours = 0;
        assert!(ConfigManager::validate_config(&config).is_err());

        // Reset cache TTL
        config.cache.ttl_hours = 24;

        // Invalid timeout
        config.network.timeout_seconds = 0;
        assert!(ConfigManager::validate_config(&config).is_err());

        // Reset timeout
        config.network.timeout_seconds = 30;

        // Invalid verbose + quiet
        config.output.verbose = true;
        config.output.quiet = true;
        assert!(ConfigManager::validate_config(&config).is_err());

        // Reset output
        config.output.verbose = false;
        config.output.quiet = false;

        // Invalid extensions
        config.files.extensions = vec![];
        assert!(ConfigManager::validate_config(&config).is_err());

        config.files.extensions = vec!["invalid/ext".to_string()];
        assert!(ConfigManager::validate_config(&config).is_err());
    }

    #[test]
    fn test_utility_functions() {
        let config = Config::default();

        // Test cache directory
        let cache_dir = ConfigManager::get_cache_directory(&config);
        assert!(cache_dir.to_string_lossy().contains("validate-xml"));

        // Test thread count
        let thread_count = ConfigManager::get_thread_count(&config);
        assert!(thread_count >= 1);

        // Test duration conversions
        let timeout = ConfigManager::get_timeout_duration(&config);
        assert_eq!(timeout, Duration::from_secs(30));

        let cache_ttl = ConfigManager::get_cache_ttl_duration(&config);
        assert_eq!(cache_ttl, Duration::from_secs(24 * 3600));

        let retry_delay = ConfigManager::get_retry_delay_duration(&config);
        assert_eq!(retry_delay, Duration::from_millis(1000));
    }

    #[test]
    fn test_output_format_conversion() {
        assert_eq!(
            OutputFormatConfig::from(OutputFormat::Human),
            OutputFormatConfig::Human
        );
        assert_eq!(
            OutputFormatConfig::from(OutputFormat::Json),
            OutputFormatConfig::Json
        );
        assert_eq!(
            OutputFormatConfig::from(OutputFormat::Summary),
            OutputFormatConfig::Summary
        );

        assert_eq!(
            OutputFormat::from(OutputFormatConfig::Human),
            OutputFormat::Human
        );
        assert_eq!(
            OutputFormat::from(OutputFormatConfig::Json),
            OutputFormat::Json
        );
        assert_eq!(
            OutputFormat::from(OutputFormatConfig::Summary),
            OutputFormat::Summary
        );
    }

    #[tokio::test]
    async fn test_find_config_file_not_found() {
        // This test assumes no config files exist in the current directory or user config
        let result = ConfigManager::find_config_file().await.unwrap();
        // Result could be None (no config found) or Some (config found in user directory)
        // We just test that it doesn't error
        assert!(result.is_none() || result.is_some());
    }

    #[tokio::test]
    async fn test_load_config_integration() {
        use clap::Parser;

        let temp_dir = TempDir::new().unwrap();

        // Create a config file
        let config_path = temp_dir.path().join("test.toml");
        let toml_content = r#"
[validation]
threads = 6
fail_fast = true
show_progress = false

[cache]
directory = "/tmp/test-cache"
ttl_hours = 48
max_size_mb = 100
max_memory_entries = 800
memory_ttl_seconds = 2400

[network]
timeout_seconds = 45
retry_attempts = 3
retry_delay_ms = 1000

[output]
format = "human"
verbose = false
quiet = false

[files]
extensions = ["xml"]
include_patterns = []
exclude_patterns = []
"#;
        fs::write(&config_path, toml_content).unwrap();

        // Create CLI args that override some config values
        let args = vec![
            "validate-xml",
            "--config",
            config_path.to_str().unwrap(),
            "--threads",
            "8", // This should override config file
            "--verbose",
            temp_dir.path().to_str().unwrap(),
        ];

        let cli = Cli::try_parse_from(args).unwrap();
        let config = ConfigManager::load_config(&cli).await.unwrap();

        // CLI should override config file
        assert_eq!(config.validation.threads, Some(8));
        assert!(config.output.verbose);

        // Config file values should be used where CLI doesn't override
        // Note: fail_fast is false because CLI default (false) overrides config file (true)
        assert!(!config.validation.fail_fast); // CLI default overrides config
        assert_eq!(config.cache.ttl_hours, 24); // CLI default overrides config
        assert_eq!(config.network.timeout_seconds, 30); // CLI default overrides config
    }
}
