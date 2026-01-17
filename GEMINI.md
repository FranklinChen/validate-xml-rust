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
*   **Parsing (`xmlSchemaParse`)**: NOT thread-safe. Serialized via `ParsedSchemaCache`.
*   **Validation (`xmlSchemaValidateFile`)**: IS thread-safe. Runs in parallel.

### FFI Wrappers (`src/libxml2.rs`)
*   **Safety**: Wrap raw pointers in `XmlSchemaPtr` (RAII) immediately to prevent leaks/segfaults.
