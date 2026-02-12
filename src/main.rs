use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::Duration;

use validate_xml::*;

#[tokio::main]
async fn main() -> Result<(), ValidationError> {
    let cli = Cli::parse_args();

    if let Err(error) = cli.validate() {
        eprintln!("Error: {}", error);
        std::process::exit(1);
    }

    let config = Config::from_cli(&cli);
    let _libxml2_wrapper = LibXml2Wrapper::new();

    if config.verbose && !config.quiet {
        println!("XML Validator");
        println!("Configuration:");
        println!("  Path: {}", config.path.display());
        println!("  Extensions: {:?}", config.extensions);
        println!("  Threads: {}", config.threads);
        println!("  Cache directory: {}", config.cache_dir.display());
        println!("  Timeout: {} seconds", config.timeout_seconds);
    }

    let schema_cache = Arc::new(SchemaCache::new(CacheConfig {
        directory: config.cache_dir.clone(),
        ttl_hours: config.cache_ttl_hours,
        max_size_mb: config.max_cache_size_mb,
        max_memory_entries: 1000,
        memory_ttl_seconds: 3600,
    }));

    let http_client = AsyncHttpClient::new(HttpClientConfig {
        timeout_seconds: config.timeout_seconds,
        retry_attempts: config.retry_attempts,
        retry_delay_ms: 1000,
        max_retry_delay_ms: 30000,
        user_agent: format!("validate-xml/{}", env!("CARGO_PKG_VERSION")),
    })?;

    let validation_engine = ValidationEngine::new(
        schema_cache,
        http_client,
        ValidationConfig {
            max_concurrent_validations: config.threads,
            validation_timeout: Duration::from_secs(config.timeout_seconds),
            fail_fast: config.fail_fast,
            show_progress: config.progress,
            collect_metrics: true,
            schema_override: config.schema.clone(),
        },
    )?;

    let file_discovery = FileDiscovery::new().with_extensions(config.extensions.clone());
    let file_discovery = file_discovery.with_include_patterns(config.include_patterns.clone())?;
    let file_discovery = file_discovery.with_exclude_patterns(config.exclude_patterns.clone())?;

    if !config.quiet {
        println!("Scanning: {}", config.path.display());
    }

    let pb = if config.progress && !config.quiet {
        let pb = ProgressBar::new(0);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("█░"));
        Some(pb)
    } else {
        None
    };

    let pb_clone = pb.clone();
    let progress_callback = pb_clone.map(|pb| {
        Arc::new(move |progress: ValidationProgress| match progress.phase {
            ValidationPhase::Discovery => {
                pb.set_message("Discovering files...");
            }
            ValidationPhase::SchemaLoading => {
                pb.set_message("Loading schemas...");
            }
            ValidationPhase::Validation => {
                if pb.length() == Some(0) && progress.total > 0 {
                    pb.set_length(progress.total as u64);
                }
                pb.set_position(progress.completed as u64);
                if let Some(file) = progress.current_file {
                    let filename = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    pb.set_message(format!("Validating: {}", filename));
                }
            }
            ValidationPhase::Aggregation => {
                pb.set_message("Finalizing...");
            }
            ValidationPhase::Complete => {
                pb.finish_and_clear();
            }
        }) as ProgressCallback
    });

    let results = validation_engine
        .run_comprehensive_validation(&config.path, &file_discovery, progress_callback)
        .await?;

    if !config.quiet {
        let output_formatter = Output::new(config.verbosity());
        println!("{}", output_formatter.format_results(&results));
    }

    if results.has_errors() && config.fail_fast {
        std::process::exit(1);
    } else if results.error_files > 0 {
        std::process::exit(2);
    } else if results.invalid_files > 0 {
        std::process::exit(3);
    }

    Ok(())
}
