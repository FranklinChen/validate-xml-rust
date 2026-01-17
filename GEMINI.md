# Developer Instructions: validate-xml

This document provides strict guidelines for developing and maintaining the `validate-xml` codebase.

## 🚨 Pre-commit Checklist

Before marking any task as "done", you **MUST** run and pass:
1.  `cargo fmt` (standard formatting)
2.  `cargo clippy` (linting and best practices)
3.  `cargo test` (verification)

## Core Mandates

*   **Minimalist Architecture**: Keep the codebase lean. Avoid redundant abstraction layers (factories, traits for simple tasks).
*   **CLI-First**: Configuration is handled via CLI flags in `src/cli.rs`. No hidden config files.
*   **Simple Output**: Use `src/output.rs` for standard human-readable and summary output. No JSON.
*   **Parse Once, Validate Many**: Use `ParsedSchemaCache` in `src/cache.rs` to ensure any XSD is parsed exactly once.
*   **Non-Blocking Validation**: CPU-intensive FFI calls (`xmlSchemaParse`, `xmlSchemaValidateFile`) **MUST** run in `tokio::task::spawn_blocking` (or bounded parallel tasks).
*   **Quality Control**: Rigorously enforce code quality. All changes MUST pass `cargo fmt` and `cargo clippy` without warnings.

## Architecture Reference

### 1. File Discovery (`src/file_discovery.rs`)
*   **Role**: Async recursive directory scanning with glob pattern support.

### 2. Schema Loading (`src/schema_loader.rs`)
*   **Role**: Extract URL -> Download/Read -> Cache Raw Bytes.

### 3. Validation Engine (`src/validator.rs`)
*   **Role**: Orchestrate the pipeline (Discovery -> Loading -> Validation).

### 4. Caching (`src/cache.rs`)
*   **Role**: Two-tier caching (Memory + Disk) for persistent schema storage.

## Implementation Details

### Thread Safety (Critical)

Our architecture handles `libxml2`'s legacy global state through a hybrid strategy:

*   **Serialized Operations**: Library initialization (`xmlInitParser`) and schema parsing (`xmlSchemaParse`) are **NOT** thread-safe. These are protected by a global `Mutex` (`LIBXML2_GLOBAL_LOCK`) in `src/libxml2.rs`.
*   **Parallel Operations**: Schema validation (`xmlSchemaValidateFile`) **IS** thread-safe and runs in parallel across all CPU cores without locks.
*   **Thundering Herd Protection**: `ParsedSchemaCache` (moka) ensures each unique XSD is parsed exactly once, even under heavy concurrent load.

### Platform Support

The project supports macOS, Linux, and Windows:

*   **Windows**: Uses `vcpkg` for `libxml2`. Linking is directed to `libxml2.lib`.
*   **Unix (macOS/Linux)**: Uses system libraries. Linking is directed to `libxml2` (usually `libxml2.so` or `libxml2.dylib`).
*   **FFI Linking**: Handled via `#[cfg_attr]` in `src/libxml2.rs` to ensure correct library resolution at compile time.

### FFI Wrappers (`src/libxml2.rs`)

*   **Safety**: Wrap raw pointers in `XmlSchemaPtr` (RAII) immediately to prevent leaks/segfaults.
*   **Locking**: All non-thread-safe FFI calls MUST be wrapped in the global mutex.
