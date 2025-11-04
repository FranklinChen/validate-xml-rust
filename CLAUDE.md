# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

validate-xml is a high-performance XML schema validator written in Rust. It validates thousands of XML files against XSD schemas using concurrent processing and intelligent two-tier caching (memory + disk). Built with libxml2 FFI bindings and async I/O throughout.

**Key Performance**: Validates 20,000 files in ~2 seconds (cached) or ~30 seconds (first run with schema downloads).

## Common Commands

### Building and Testing

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run all tests (deterministic, no network calls)
cargo test

# Run a specific test
cargo test test_name

# Run tests with output visible
cargo test -- --nocapture

# Run only library tests (fastest)
cargo test --lib

# Run ignored network tests (requires internet)
cargo test -- --ignored

# Run a single test file
cargo test --test http_client_test
```

### Running the Binary

```bash
# Run with development build
cargo run -- /path/to/xml/files

# Run with release build (much faster)
cargo run --release -- /path/to/xml/files

# With options
cargo run --release -- --verbose --extensions xml,cmdi /path/to/files

# With debug logging
RUST_LOG=debug cargo run -- /path/to/files
```

### Code Quality

```bash
# Format code
cargo fmt

# Check formatting without changes
cargo fmt --check

# Run clippy linter
cargo clippy

# Fix clippy warnings automatically
cargo clippy --fix
```

## Architecture

### Core Components

The codebase follows a modular async-first architecture with clear separation of concerns:

1. **File Discovery** (`file_discovery.rs`)
   - Recursively traverses directories to find XML files
   - Filters by extension using glob patterns
   - Single-threaded sequential operation

2. **Schema Loading** (`schema_loader.rs`)
   - Extracts schema URLs from XML using regex (xsi:schemaLocation, xsi:noNamespaceSchemaLocation)
   - Downloads remote schemas via async HTTP client
   - Validates schema content before caching
   - Integrates with two-tier cache system

3. **Two-Tier Caching** (`cache.rs`)
   - **L1 (Memory)**: moka cache for in-run reuse (microsecond lookups)
   - **L2 (Disk)**: cacache for cross-run persistence (millisecond lookups)
   - Thread-safe via Arc wrapping
   - Configurable TTL and size limits

4. **Validation Engine** (`validator.rs`)
   - **Hybrid architecture**: Async I/O orchestration + sync CPU-bound validation
   - Spawns concurrent async tasks (bounded by semaphore)
   - Each task: load XML → fetch schema → validate via libxml2 (synchronous, thread-safe)
   - Collects results and statistics
   - Default concurrency = CPU core count

5. **libxml2 FFI** (`libxml2.rs`)
   - Safe Rust wrappers around unsafe C FFI calls
   - Memory management via RAII patterns
   - Schema parsing and XML validation
   - **CRITICAL Thread Safety**:
     - Schema parsing is NOT thread-safe (serialized via cache)
     - Validation IS thread-safe (parallel execution, no global locks)

6. **Error Handling** (`error.rs`, `error_reporter.rs`)
   - Structured error types using thiserror
   - Context-rich error messages with recovery hints
   - Line/column precision for validation errors
   - Both human-readable and JSON output formats

7. **Configuration** (`config.rs`)
   - Environment variable support via `EnvProvider` trait pattern
   - File-based config (TOML/JSON)
   - CLI argument merging (CLI > env > file > defaults)
   - **IMPORTANT**: Uses dependency injection for testability

### Data Flow

```
CLI Args → Config Merge → File Discovery → Schema Extraction
                                              ↓
                                         Schema Cache Check
                                         (L1 → L2 → HTTP)
                                              ↓
                                    Concurrent Validation Tasks
                                    (bounded by semaphore)
                                              ↓
                                    Error Aggregation → Output
                                    (Text or JSON format)
```

### Key Design Patterns

1. **Async-First**: All I/O operations use tokio async runtime
2. **Dependency Injection**: Config system uses `EnvProvider` trait for testability
3. **Two-Tier Caching**: Memory (fast) + Disk (persistent) for optimal performance
4. **Bounded Concurrency**: Semaphore limits prevent resource exhaustion
5. **RAII for FFI**: Proper cleanup of libxml2 resources via Drop trait

## Testing Philosophy

### Test Structure

The project has **214+ passing tests** organized as:
- **115 unit tests** in `src/` modules (fast, no I/O)
- **99 integration tests** in `tests/` (slower, includes I/O simulation)
- **24 ignored tests** (network-dependent, run explicitly with `--ignored`)

### Critical Testing Rules

1. **No Unsafe Code in Tests**: All environment variable manipulation must use `MockEnvProvider` pattern (see `src/config.rs` tests)

2. **No Real Network Calls**: Tests making HTTP requests to external services (httpbin.org) must be marked `#[ignore]`
   ```rust
   #[tokio::test]
   #[ignore] // Requires internet connectivity - run with: cargo test -- --ignored
   async fn test_network_operation() { ... }
   ```

3. **Deterministic Tests Only**: Never use:
   - `tokio::time::sleep()` without proper synchronization
   - `tokio::spawn()` without waiting for completion
   - Real system time for timing assertions

4. **Race Condition Prevention**: When testing concurrent code, use proper synchronization:
   ```rust
   // BAD: Race condition
   tokio::spawn(async move { /* ... */ });
   tokio::time::sleep(Duration::from_millis(50)).await; // Hope it finishes

   // GOOD: Proper synchronization
   let handle = tokio::spawn(async move { /* ... */ });
   handle.await.unwrap(); // Wait for completion
   ```

### Running Flaky/Network Tests

Network tests are ignored by default to ensure CI reliability:
```bash
# Run only network tests
cargo test -- --ignored

# Run all tests including network tests
cargo test -- --include-ignored
```

## Environment Variables

The config system supports environment variable overrides:

```bash
# Cache configuration
export VALIDATE_XML_CACHE_DIR=/custom/cache
export VALIDATE_XML_CACHE_TTL=48

# Validation settings
export VALIDATE_XML_THREADS=4
export VALIDATE_XML_TIMEOUT=120

# Output settings
export VALIDATE_XML_VERBOSE=true
export VALIDATE_XML_FORMAT=json
```

## libxml2 FFI Critical Notes

When working with `libxml2.rs`:

1. **Memory Safety**: All pointers must be checked for null before dereferencing
2. **Cleanup**: Schema contexts must be freed via `xmlSchemaFree` in Drop implementations
3. **Thread Safety** (see ARCHITECTURE_CHANGES.md for details):
   - **Schema parsing** (`xmlSchemaParse`): NOT thread-safe, serialized via cache
   - **Validation** (`xmlSchemaValidateFile`): IS thread-safe, runs in parallel
   - Arc-wrapped schemas enable safe sharing across tasks
   - Each validation creates its own context (per-task isolation)
4. **Error Handling**: libxml2 prints errors to stderr - this is expected in tests (e.g., "Schemas parser error" messages)

Example safe pattern:
```rust
impl Drop for SchemaContext {
    fn drop(&mut self) {
        unsafe {
            if !self.schema.is_null() {
                xmlSchemaFree(self.schema);
            }
        }
    }
}
```

## Dependency Injection Pattern

For testability, the config system uses trait-based dependency injection:

```rust
// Production: uses real environment variables
ConfigManager::apply_environment_overrides(config)

// Testing: uses mock provider (no unsafe code)
let mut mock_env = MockEnvProvider::new();
mock_env.set("VALIDATE_XML_THREADS", "16");
ConfigManager::apply_environment_overrides_with(&mock_env, config)
```

**Never** use `std::env::set_var` or `std::env::remove_var` in tests - always use `MockEnvProvider`.

## Performance Considerations

1. **Schema Caching**: First run downloads schemas (~30s for 20k files), subsequent runs use cache (~2s)
2. **Concurrency**: Default = CPU cores, but can be limited for memory-constrained systems
3. **Memory**: Bounded by L1 cache size (default 100 entries) and concurrent task count
4. **Network**: HTTP client uses connection pooling and retry logic with exponential backoff

## Common Gotchas

1. **libxml2 Errors to stderr**: The message "Schemas parser error : The XML document 'in_memory_buffer' is not a schema document" is EXPECTED in test output - it's from tests validating error handling

2. **Timing Tests**: Any test using `tokio::time::sleep()` is likely flaky - refactor to use proper synchronization

3. **Environment Pollution**: Tests must not modify global environment state - use `MockEnvProvider` pattern

4. **Ignored Tests**: Running full test suite may show "24 ignored" - this is correct (network tests)

## Code Generation and AI Assistance

This project was collaboratively developed with Claude Code. When making changes:

1. Maintain the existing architecture patterns (async-first, dependency injection, trait-based abstractions)
2. Add tests for all new functionality (aim for 100% coverage)
3. Update documentation strings for public APIs
4. Run full test suite before committing: `cargo test && cargo clippy`
5. For network-dependent code, mark tests with `#[ignore]` and document why
