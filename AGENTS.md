# Repository guidance

This file provides guidance to coding agents working in this repository.

## Project Overview

**validate-xml** is a high-performance CLI tool for validating XML files against XML Schemas (XSD), built with the latest stable Rust toolchain (edition 2024). Hardened libxml2 is the single authoritative validator. The crate uses concurrent async processing and two-tier caching (memory + disk).

## Build & Development Commands

```bash
cargo build --release             # Build release binary
cargo test                        # Run all tests (unit + integration)
cargo test <test_name>            # Run a single test by name (substring match)
cargo test --lib                  # Run only library unit tests (skip integration)
cargo test --test integration_tests  # Run only integration tests
cargo bench                       # Run benchmarks (divan harness)
cargo fmt                         # Format code
cargo clippy                      # Lint
cargo install --path .            # Install binary to ~/.cargo/bin
```

Integration tests (`tests/integration_tests.rs`) invoke the binary Cargo builds for the active test profile through `CARGO_BIN_EXE_validate-xml`; no separate release build is required. Use `cargo test --lib` for a faster unit-test-only loop.

The `samples/pom.xml` file exercises the remote schema fetch path (Apache Maven POM schema) and is useful for smoke-testing HTTP + disk cache behavior end-to-end.

## Architecture

The validation pipeline flows: CLI parsing → file discovery → schema extraction → schema loading (with caching) → parallel validation → output formatting.

### Key modules in `src/`:

- **main.rs** — Entry point, sets up tokio runtime, progress bars (indicatif), exit codes (0=success, 1=config error, 2=system error, 3=validation failures)
- **lib.rs** — Re-exports the public library surface (see `src/lib.rs`); the crate builds as both a binary and a library.
- **validator.rs** — Core engine. Hybrid async/sync: async I/O for file/network ops, while backend compilation/validation runs through bounded `tokio::task::spawn_blocking` calls. Timeout reports a deadline failure but cannot interrupt an active FFI call; its permit remains held until return. Fail-fast stops scheduling and drains admitted work.
- **backend.rs** — `CompiledSchema` typestate boundary and hardened libxml2 validation. The FFI serializes initialization/schema compilation and creates a distinct validation context for every call.
- **cache.rs** — Two-tier caching: L1 in-memory (`moka::Cache` for `CompiledSchema`, with thundering-herd protection, bounded capacity, and TTL) and L2 disk (`cacache` with TTL and size eviction).
- **schema_loader.rs** — Extracts schema URLs from XML and loads the referenced XSDs. Several non-obvious design points:
  - **Streaming extraction**: uses `quick-xml`'s namespace-aware `NsReader` to walk only the prolog + root element's start tag, then stops. The read runs on a tokio blocking thread over a sync `std::io::BufReader`, so the rest of the XML body never enters memory regardless of file size.
  - **Schema-URL sources**: `xsi:schemaLocation` (whitespace-separated `(namespace, location)+` pairs retained for root-namespace matching), `xsi:noNamespaceSchemaLocation`, and `<?xml-model href=... ?>` processing instructions.
  - **Namespace-URI attribute resolution**: xsi attributes are matched by namespace URI (`http://www.w3.org/2001/XMLSchema-instance`), not by literal `xsi:` prefix, so documents that bind xsi to a non-standard prefix still work.
  - **Odd-token `schemaLocation`** surfaces as `ValidationError::XmlParsing`, not silently dropped.
  - **`SchemaReference` is an enum** (`Local(PathBuf)` / `Remote(String)`). It was previously a struct with a `url: String` field that, for `Local` refs, held the *unresolved* raw reference and was used as the L1 cache key — two XML files in different directories both referencing `"schema.xsd"` would silently share a cache entry. `SchemaReference::cache_key()` provides a namespaced location identity; `SchemaLoader::cache_key()` additionally versions local schemas with the complete graph digest used by both cache tiers.
  - **Loading paths**: local files via `tokio::fs`, remote via `AsyncHttpClient`, both routed through the two-tier cache.
  - **Root-element probe**: fetched XSD bytes are checked (root element must be `{http://www.w3.org/2001/XMLSchema}schema`) before being cached, so HTML/404 responses don't poison the disk cache.
- **http_client.rs** — Async HTTP client (reqwest) with exponential backoff retries.
- **file_discovery.rs** — Recursive `ignore` traversal on Tokio's blocking pool with `globset`-based pattern matching.
- **cli.rs** — Clap derive-based argument parsing. Key flags: `--schema` (override XSD), `--threads`, `--cache-ttl`, `--extensions`. `--schema` populates `ValidationConfig.schema_override` and **short-circuits `schema_loader.rs`'s extraction path entirely** — useful to remember when a validation issue might lie in extraction vs. validation itself.
- **error.rs** — Error hierarchy using `thiserror`. Main types: `ValidationError`, `ConfigError`, `CacheError`, `NetworkError`.
- **output.rs** — Color-coded TTY-aware output formatting with verbosity levels.

### Design decisions

- **Compiled-schema boundary** keeps fetched bytes distinct from schemas accepted by libxml2. Local schemas receive a filesystem base URI for relative includes/imports; remote schemas remain memory-only so libxml2 cannot bypass the HTTP client. The external thread-safety stress repository supports the shared-schema/per-call-context pattern but is not a general security proof.
- **Hybrid async/sync** separates I/O-bound work (Tokio) from CPU-bound validation. `spawn_blocking` prevents synchronous parsing and validation from occupying Tokio worker threads.
- **Globset replaced regex** for file pattern matching (more intuitive, better performance).
- **Two-tier cache separation**: L1 stores `CompiledSchema` (work reuse, moka single-flight); L2 stores raw schema bytes on disk (cacache, survives process restarts). A canonical path plus SHA-256 content digest versions local keys.

## Error model

Errors flow through `ValidationError` (`src/error.rs`), a `thiserror` enum that distinguishes configuration, network, cache, schema-parsing (XSD side), XML-parsing (input-document side), and validation-failure cases. `SchemaParsing { url, details }` is for XSD content; `XmlParsing { file, details }` is for the XML input file being validated — don't conflate them (the old code stuffed XML file paths into `SchemaParsing.url`, which was a leaky abstraction). Exit-code mapping in `main.rs` depends on aggregate counters (`error_files`, `invalid_files`), not on the variant itself — add new error categories by extending the enum rather than stringly-typed messages, so the exit-code contract in the README stays stable.
