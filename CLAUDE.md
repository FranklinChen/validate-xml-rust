# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**validate-xml** is a high-performance CLI tool for validating XML files against XML Schemas (XSD), built in Rust (edition 2024, MSRV 1.81). It uses the xmloxide crate (pure Rust, no system dependencies), concurrent async processing, and two-tier caching (memory + disk). The crate is a dual bin+lib: integration tests shell out to the compiled binary, while the library surface (`src/lib.rs`) is also re-exported for programmatic use.

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

Integration tests (`tests/integration_tests.rs`) invoke the compiled binary at `./target/release/validate-xml`, so `cargo build --release` must run before `cargo test` (or before `cargo test --test integration_tests`). A quick feedback loop that skips the build: `cargo test --lib`.

The `samples/pom.xml` file exercises the remote schema fetch path (Apache Maven POM schema) and is useful for smoke-testing HTTP + disk cache behavior end-to-end.

## Architecture

The validation pipeline flows: CLI parsing → file discovery → schema extraction → schema loading (with caching) → parallel validation → output formatting.

### Key modules in `src/`:

- **main.rs** — Entry point, sets up tokio runtime, progress bars (indicatif), exit codes (0=success, 1=config error, 2=system error, 3=validation failures)
- **lib.rs** — Re-exports the public library surface (see `src/lib.rs`); the crate builds as both a binary and a library.
- **validator.rs** — Core engine. Hybrid async/sync: async I/O for file/network ops, and `parse_xsd`/`validate_xsd` CPU work is currently wrapped in `tokio::task::spawn_blocking` (see the two call sites inside `validate_single_file_internal`) with an `Arc<Semaphore>` bounding concurrency. Note: the module-level doc comment claims "no spawn_blocking overhead" — this reflects the original design intent for xmloxide's thread-safe sync API but does not match current code. Reconcile the comment or the implementation when touching this file.
- **cache.rs** — Two-tier caching: L1 in-memory (`moka::Cache` for parsed `XsdSchema`, with thundering herd protection) and L2 disk (`cacache` with TTL expiration).
- **schema_loader.rs** — Extracts schema URLs from XML and loads the referenced XSDs. Several non-obvious design points:
  - **Streaming extraction**: uses `quick-xml`'s namespace-aware `NsReader` to walk only the prolog + root element's start tag, then stops. The read runs on a tokio blocking thread over a sync `std::io::BufReader`, so the rest of the XML body never enters memory regardless of file size.
  - **Schema-URL sources**: `xsi:schemaLocation` (whitespace-separated `(namespace, location)+` pairs — locations emitted, namespaces discarded), `xsi:noNamespaceSchemaLocation`, and `<?xml-model href=... ?>` processing instructions.
  - **Namespace-URI attribute resolution**: xsi attributes are matched by namespace URI (`http://www.w3.org/2001/XMLSchema-instance`), not by literal `xsi:` prefix, so documents that bind xsi to a non-standard prefix still work.
  - **Odd-token `schemaLocation`** surfaces as `ValidationError::XmlParsing`, not silently dropped.
  - **`SchemaReference` is an enum** (`Local(PathBuf)` / `Remote(String)`). It was previously a struct with a `url: String` field that, for `Local` refs, held the *unresolved* raw reference and was used as the L1 cache key — two XML files in different directories both referencing `"schema.xsd"` would silently share a cache entry. Cache keys now come from `SchemaReference::cache_key()`, which namespaces local paths with a `local:` prefix and uses the fully resolved path.
  - **Loading paths**: local files via `tokio::fs`, remote via `AsyncHttpClient`, both routed through the two-tier cache.
  - **Root-element probe**: fetched XSD bytes are checked (root element must be `{http://www.w3.org/2001/XMLSchema}schema`) before being cached, so HTML/404 responses don't poison the disk cache.
- **http_client.rs** — Async HTTP client (reqwest) with exponential backoff retries.
- **file_discovery.rs** — Recursive async directory traversal with `globset`-based pattern matching.
- **cli.rs** — Clap derive-based argument parsing. Key flags: `--schema` (override XSD), `--threads`, `--cache-ttl`, `--extensions`. `--schema` populates `ValidationConfig.schema_override` and **short-circuits `schema_loader.rs`'s extraction path entirely** — useful to remember when a validation issue might lie in extraction vs. validation itself.
- **error.rs** — Error hierarchy using `thiserror`. Main types: `ValidationError`, `ConfigError`, `CacheError`, `NetworkError`.
- **output.rs** — Color-coded TTY-aware output formatting with verbosity levels.

### Design decisions

- **xmloxide over libxml2 FFI** for pure Rust safety, no system dependencies, and simpler builds (see `CHANGELOG.md` 0.3.0 for the FFI-removal migration, and 0.4.0 for the `SchemaReference` enum refactor + the streaming schema-URL extractor rewrite). `CHANGELOG.md` is the canonical record of why each module looks the way it does — consult it before reverse-engineering history from `git log`.
- **Hybrid async/sync** separates I/O-bound work (tokio) from CPU-bound validation. xmloxide is thread-safe so a `spawn_blocking` hop is not strictly required; the code currently uses it anyway — treat this as a tunable, not an invariant.
- **Globset replaced regex** for file pattern matching (more intuitive, better performance).
- **Two-tier cache separation**: L1 stores parsed `XsdSchema` (work reuse, moka with thundering-herd protection); L2 stores raw schema bytes on disk (cacache, survives process restarts). L1 rebuilds from L2 on cold start.

## Error model

Errors flow through `ValidationError` (`src/error.rs`), a `thiserror` enum that distinguishes configuration, network, cache, schema-parsing (XSD side), XML-parsing (input-document side), and validation-failure cases. `SchemaParsing { url, details }` is for XSD content; `XmlParsing { file, details }` is for the XML input file being validated — don't conflate them (the old code stuffed XML file paths into `SchemaParsing.url`, which was a leaky abstraction). Exit-code mapping in `main.rs` depends on aggregate counters (`error_files`, `invalid_files`), not on the variant itself — add new error categories by extending the enum rather than stringly-typed messages, so the exit-code contract in the README stays stable.
