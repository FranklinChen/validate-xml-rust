use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

// Error handling modules
mod error;
mod error_reporter;

// CLI module
mod cli;

// Configuration module
mod config;

// Cache module
mod cache;

// HTTP client module
mod http_client;

// Schema loader module
mod schema_loader;

// LibXML2 wrapper module
mod libxml2;

// File discovery module
mod file_discovery;

// Validation engine module
mod validator;

// Output and reporting module
mod output;

pub use cache::*;
pub use cli::*;
pub use config::{Config, ConfigError as ConfigurationError, ConfigManager};
pub use error::ValidationError;
pub use error_reporter::{ErrorReporter, VerbosityLevel};
pub use file_discovery::*;
pub use http_client::{AsyncHttpClient, HttpClientConfig};
pub use libxml2::*;
pub use output::*;
pub use schema_loader::*;
pub use validator::*;

// CLI interface now implemented with Clap in cli.rs module

/// Return the first Schema URL found, if any.
/// This is the legacy synchronous version - use extract_schema_url_async for new code.
#[allow(dead_code)]
fn extract_schema_url(path: &Path) -> Option<String> {
    // Use the async version in a blocking context for backward compatibility
    let rt = tokio::runtime::Handle::current();
    rt.block_on(extract_schema_url_async(path)).ok()
}

// Temporarily commented out - will be replaced with async cache in task 5
/*
/// Cache schema into memory after downloading from Web once and stashing into memory.
///
/// Panics on I/O error.
#[cached(sync_writes = true)]
fn get_schema(url: String) -> XmlSchemaPtr {
    lazy_static! {
        static ref CLIENT: Client = Client::new();
    }

    // DEBUG to show that download happens only once.
    println!("Downloading now {url}...");

    let response = CLIENT.get(url.as_str()).send().unwrap().bytes().unwrap();

    unsafe {
        let schema_parser_ctxt =
            xmlSchemaNewMemParserCtxt(response.as_ptr() as *const c_char, response.len() as i32);

        // Use default callbacks rather than overriding.
        //xmlSchemaSetParserErrors();

        let schema = xmlSchemaParse(schema_parser_ctxt);
        xmlSchemaFreeParserCtxt(schema_parser_ctxt);

        XmlSchemaPtr(schema)
    }
}
*/

// Temporarily commented out - will be replaced with async validation in task 10
/*
/// Copy the behavior of [`xmllint`](https://github.com/GNOME/libxml2/blob/master/xmllint.c)
fn validate(path_buf: PathBuf) {
    let url = extract_schema_url(path_buf.as_path()).unwrap();
    let schema = get_schema(url);

    let path_str = path_buf.to_str().unwrap();
    let c_path = CString::new(path_str).unwrap();

    unsafe {
        // Have to create new validation context for each parse.
        let schema_valid_ctxt = xmlSchemaNewValidCtxt(schema.0);

        // TODO better error message with integrated path using callback.
        //xmlSchemaSetValidErrors();

        // This reads the file and validates it.
        let result = xmlSchemaValidateFile(schema_valid_ctxt, c_path.as_ptr(), 0);
        if result == 0 {
            eprintln!("{path_str} validates");
        } else if result > 0 {
            // Note: the message is output after the validation messages.
            eprintln!("{path_str} fails to validate");
        } else {
            eprintln!("{path_str} validation generated an internal error");
        }

        xmlSchemaFreeValidCtxt(schema_valid_ctxt);
    }
}
*/

#[tokio::main]
async fn main() -> Result<(), ValidationError> {
    // Parse command line arguments using Clap
    let cli = Cli::parse_args();

    // Validate CLI arguments
    if let Err(error) = cli.validate() {
        eprintln!("Error: {}", error);
        std::process::exit(1);
    }

    // Load configuration with precedence: file -> env -> CLI
    let config = match ConfigManager::load_config(&cli).await {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Configuration error: {}", error);
            std::process::exit(1);
        }
    };

    // Initialize libxml2 wrapper (this will remain)
    let _libxml2_wrapper = LibXml2Wrapper::new();

    // Display configuration in verbose mode
    if config.output.verbose && !config.output.quiet {
        println!("XML Validator - Configuration Management System implemented");
        println!("Configuration:");
        println!("  Directory: {}", cli.directory.display());
        println!("  Extensions: {:?}", config.files.extensions);
        println!("  Threads: {}", ConfigManager::get_thread_count(&config));
        println!(
            "  Cache directory: {}",
            ConfigManager::get_cache_directory(&config).display()
        );
        println!("  Cache TTL: {} hours", config.cache.ttl_hours);
        println!("  Cache max size: {} MB", config.cache.max_size_mb);
        println!("  Timeout: {} seconds", config.network.timeout_seconds);
        println!("  Retry attempts: {}", config.network.retry_attempts);
        println!("  Retry delay: {} ms", config.network.retry_delay_ms);
        println!("  Output format: {:?}", config.output.format);
        println!("  Fail fast: {}", config.validation.fail_fast);
        println!("  Show progress: {}", config.validation.show_progress);

        if !config.files.include_patterns.is_empty() {
            println!("  Include patterns: {:?}", config.files.include_patterns);
        }
        if !config.files.exclude_patterns.is_empty() {
            println!("  Exclude patterns: {:?}", config.files.exclude_patterns);
        }

        if let Some(config_path) = &cli.config {
            println!("  Config file: {}", config_path.display());
        }
    } else if !config.output.quiet {
        println!("XML Validator - Configuration Management System implemented");
        println!("Run with --verbose for detailed configuration");
    }

    // Initialize async validation engine (Task 10 implementation)
    let schema_cache = Arc::new(SchemaCache::new(config.cache.clone()));

    // Convert NetworkConfig to HttpClientConfig
    let http_config = HttpClientConfig {
        timeout_seconds: config.network.timeout_seconds,
        retry_attempts: config.network.retry_attempts,
        retry_delay_ms: config.network.retry_delay_ms,
        max_retry_delay_ms: 30000, // Default value
        user_agent: format!("validate-xml/{}", env!("CARGO_PKG_VERSION")),
    };
    let http_client = AsyncHttpClient::new(http_config)?;

    let validation_config = ValidationConfig {
        max_concurrent_validations: ConfigManager::get_thread_count(&config),
        validation_timeout: Duration::from_secs(config.network.timeout_seconds),
        fail_fast: config.validation.fail_fast,
        show_progress: config.validation.show_progress,
        collect_metrics: true,
        progress_update_interval_ms: 100,
    };

    let validation_engine = ValidationEngine::new(schema_cache, http_client, validation_config)?;

    // Initialize file discovery
    let file_discovery = FileDiscovery::new()
        .with_extensions(config.files.extensions.clone())
        .with_include_patterns(config.files.include_patterns.clone())?
        .with_exclude_patterns(config.files.exclude_patterns.clone())?;

    if !config.output.quiet {
        println!(
            "Starting comprehensive validation of directory: {}",
            cli.directory.display()
        );
        if config.validation.show_progress {
            println!("Progress tracking enabled");
        }
    }

    // Create progress callback if progress is enabled
    let progress_callback = if config.validation.show_progress && !config.output.quiet {
        let verbosity = if config.output.quiet {
            VerbosityLevel::Quiet
        } else if config.output.verbose {
            VerbosityLevel::Verbose
        } else {
            VerbosityLevel::Normal
        };

        // Create formatter for progress updates
        let formatter = OutputFormatterFactory::create_formatter(
            config.output.format.clone().into(),
            verbosity,
        );

        Some(Arc::new(move |progress: ValidationProgress| {
            match progress.phase {
                ValidationPhase::Discovery => {
                    if !matches!(verbosity, VerbosityLevel::Quiet) {
                        eprint!("\rDiscovering XML files...");
                        let _ = std::io::stderr().flush();
                    }
                }
                ValidationPhase::SchemaLoading => {
                    if !matches!(verbosity, VerbosityLevel::Quiet) {
                        eprint!("\rLoading schemas...");
                        let _ = std::io::stderr().flush();
                    }
                }
                ValidationPhase::Validation => {
                    // Use the enhanced formatter for progress
                    if let Ok(progress_text) = formatter.format_progress(
                        progress.completed,
                        progress.total,
                        progress.current_file.as_ref(),
                    ) && !progress_text.is_empty()
                    {
                        eprint!("{}", progress_text);
                        let _ = std::io::stderr().flush();
                    }
                }
                ValidationPhase::Aggregation => {
                    if !matches!(verbosity, VerbosityLevel::Quiet) {
                        eprint!("\rAggregating results...");
                        let _ = std::io::stderr().flush();
                    }
                }
                ValidationPhase::Complete => {
                    if !matches!(verbosity, VerbosityLevel::Quiet) {
                        eprintln!(); // New line after progress
                    }
                }
            }
        }) as ProgressCallback)
    } else {
        None
    };

    // Perform comprehensive async validation with progress tracking
    let results = validation_engine
        .run_comprehensive_validation(&cli.directory, &file_discovery, progress_callback)
        .await?;

    // Report results using enhanced output system
    let verbosity = if config.output.quiet {
        VerbosityLevel::Quiet
    } else if config.output.verbose {
        VerbosityLevel::Verbose
    } else {
        VerbosityLevel::Normal
    };

    // Create output writer with appropriate format
    let mut output_writer = OutputWriter::new(config.output.format.clone().into(), verbosity);

    // Write results using the enhanced output system
    if let Err(e) = output_writer.write_results(&results) {
        eprintln!("Error writing output: {}", e);
        std::process::exit(1);
    }

    // Exit with appropriate code
    if results.has_errors() && config.validation.fail_fast {
        std::process::exit(1);
    } else if results.error_files > 0 {
        std::process::exit(2);
    } else if results.invalid_files > 0 {
        std::process::exit(3);
    }

    Ok(())
}
