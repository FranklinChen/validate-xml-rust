# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**validate-xml** is a high-performance CLI tool for validating XML files against XML Schemas (XSD), built in Rust. It uses the xmloxide crate (pure Rust, no system dependencies), concurrent async processing, and two-tier caching (memory + disk).

## Build & Development Commands

```bash
cargo build --release        # Build release binary
cargo test                   # Run all tests (unit + integration)
cargo test <test_name>       # Run a single test by name
cargo bench                  # Run benchmarks (divan harness)
cargo fmt                    # Format code
cargo clippy                 # Lint
cargo install --path .       # Install binary to ~/.cargo/bin
```

Integration tests (`tests/integration_tests.rs`) invoke the compiled binary at `./target/release/validate-xml`, so `cargo build --release` must run before `cargo test`.

## Architecture

The validation pipeline flows: CLI parsing → file discovery → schema extraction → schema loading (with caching) → parallel validation → output formatting.

### Key modules in `src/`:

- **main.rs** — Entry point, sets up tokio runtime, progress bars (indicatif), exit codes (0=success, 1=config error, 2=system error, 3=validation failures)
- **validator.rs** — Core engine. Hybrid async/sync: async I/O for file/network ops, sync xmloxide calls (`parse_xsd`, `validate_xsd`, `Document::parse_file`) for CPU-bound validation. Uses `Arc<Semaphore>` for bounded concurrency.
- **cache.rs** — Two-tier caching: L1 in-memory (`moka::Cache` for parsed `XsdSchema`, with thundering herd protection) and L2 disk (`cacache` with TTL expiration).
- **schema_loader.rs** — Extracts schema URLs from XML via regex (`xsi:schemaLocation`, `xsi:noNamespaceSchemaLocation`). Handles local and remote schemas.
- **http_client.rs** — Async HTTP client (reqwest) with exponential backoff retries.
- **file_discovery.rs** — Recursive async directory traversal with `globset`-based pattern matching.
- **cli.rs** — Clap derive-based argument parsing. Key flags: `--schema` (override XSD), `--threads`, `--cache-ttl`, `--extensions`.
- **error.rs** — Error hierarchy using `thiserror`. Main types: `ValidationError`, `ConfigError`, `CacheError`, `NetworkError`.
- **output.rs** — Color-coded TTY-aware output formatting with verbosity levels.

### Design decisions

- **xmloxide over libxml2 FFI** for pure Rust safety, no system dependencies, and simpler builds.
- **Hybrid async/sync** separates I/O-bound work (tokio) from CPU-bound validation (direct sync calls).
- **Globset replaced regex** for file pattern matching (more intuitive, better performance).
