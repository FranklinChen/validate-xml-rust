# validate-xml

[![CI](https://github.com/FranklinChen/validate-xml-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/FranklinChen/validate-xml-rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust stable](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)

`validate-xml` validates an XML file or directory tree against XML Schema (XSD) using the system libxml2 library. It can discover schema hints from documents, fetch and cache standalone remote schemas, reuse compiled schemas, and validate files concurrently.

## Requirements

- Rust 1.98 or newer. [`rust-toolchain.toml`](rust-toolchain.toml) tracks the latest stable toolchain.
- libxml2 development files.
- `pkg-config` on Linux and macOS.

Install the native dependencies before building:

```bash
# Debian or Ubuntu
sudo apt-get install libxml2-dev pkg-config

# macOS with Homebrew
brew install libxml2 pkg-config
export PKG_CONFIG_PATH="$(brew --prefix libxml2)/lib/pkgconfig"

# Windows with vcpkg
vcpkg install libxml2:x64-windows
```

The Windows build also needs the vcpkg `x64-windows` library directory in `LIB` and its binary directory in `PATH`; see [the CI workflow](.github/workflows/ci.yml) for the exact setup.

## Install

```bash
git clone https://github.com/FranklinChen/validate-xml-rust.git
cd validate-xml-rust
cargo install --locked --path .
```

## Usage

Validate every `.xml` file below a directory:

```bash
validate-xml /path/to/documents
```

Validate one file against an explicit local schema:

```bash
validate-xml --schema /path/to/schema.xsd /path/to/document.xml
```

Select more extensions and filesystem paths with globs:

```bash
validate-xml \
  --extensions xml,cmdi \
  --include '**/records/**' \
  --exclude '**/fixtures/**' \
  /path/to/documents
```

Show individual invalid/error results and performance information:

```bash
validate-xml --verbose /path/to/documents
```

`samples/pom.xml` exercises the standalone remote-schema path:

```bash
validate-xml samples/pom.xml
```

That smoke test requires network access on the first run; subsequent runs can use the disk cache until its TTL expires.

## Schema selection

Unless `--schema` is supplied, the validator reads schema hints from the XML prolog and document element:

- `xsi:schemaLocation` namespace/location pairs are retained, and the location matching the document element's namespace is selected.
- `xsi:noNamespaceSchemaLocation` is used for a document element without a namespace.
- `<?xml-model href="…"?>` is a fallback when no applicable `xsi` hint exists.

Relative local schema paths are resolved against the XML document. Local XSDs receive a filesystem base URI, so relative `xs:include` and `xs:import` references work. The complete local composition graph contributes to the cache identity, so changing an included schema invalidates the compiled entry.

HTTP and HTTPS schema hints are normalized and downloaded by the application's HTTP client. A remote schema is compiled from memory without an HTTP base URI. Remote `xs:include`, `xs:import`, `xs:redefine`, or `xs:override` composition is therefore rejected instead of allowing libxml2 to fetch nested resources outside the application's network and cache policy.

## Command-line reference

```text
validate-xml [OPTIONS] <PATH>
```

| Option | Default | Meaning |
| --- | --- | --- |
| `<PATH>` | required | XML file or directory tree to validate |
| `-e, --extensions <LIST>` | `xml` | Comma-separated file extensions |
| `-t, --threads <N>` | available parallelism | Maximum concurrent validations and blocking FFI operations |
| `-v, --verbose` | off | Include individual failures and performance information |
| `-q, --quiet` | off | Print only nonzero error/invalid counts |
| `--cache-dir <PATH>` | platform cache directory | Raw-schema disk-cache directory |
| `--cache-ttl <HOURS>` | `24` | Raw disk-cache TTL |
| `--timeout <SECONDS>` | `30` | Per-file validation deadline and HTTP request timeout |
| `--retry-attempts <N>` | `3` | Retry attempts for failed schema downloads |
| `--include <GLOB>` | none | Include matching paths; repeat for multiple patterns |
| `--exclude <GLOB>` | none | Exclude matching paths; repeat for multiple patterns |
| `--progress` | automatic on an interactive terminal | Show a progress indicator |
| `--fail-fast` | off | Stop scheduling after the first invalid/error result and drain admitted work |
| `--max-cache-size <MB>` | `100` | Indexed raw-schema disk budget; evict oldest entries first |
| `--schema <PATH>` | none | Use one local XSD for every input and skip hint extraction |

`--verbose` and `--quiet` are mutually exclusive. Zero threads, timeout, cache TTL, or cache size are rejected.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Every processed file is valid, or no invalid/error result occurred |
| `1` | CLI, configuration, initialization, or top-level workflow error |
| `2` | At least one file encountered a system, network, parsing, cache, or deadline error |
| `3` | At least one file was well-formed but violated its schema |

Files without an applicable schema hint are reported as skipped and do not by themselves produce a nonzero exit code.

## Execution and cache model

The pipeline is:

```text
CLI → file discovery → schema-hint extraction → schema loading → compilation → validation → reporting
```

- Directory traversal uses `ignore` on Tokio's blocking pool and applies `globset` include/exclude patterns.
- File, HTTP, and cache I/O is asynchronous where appropriate. XML parsing and libxml2 work run on blocking workers.
- A semaphore bounds actual blocking schema-compilation and validation operations to `--threads`.
- A deadline cannot interrupt a libxml2 call that has already started. A timed-out call retains its semaphore permit until it returns, preventing later work from exceeding the configured FFI concurrency.
- Fail-fast stops admitting new files after the first invalid/error result but drains work already admitted.
- A `CompiledSchema` can only be constructed after libxml2 accepts the XSD. This keeps untrusted raw bytes distinct from compiled schemas in the type system.

Caching has three layers of responsibility:

- A bounded, TTL-aware `moka` cache stores compiled `CompiledSchema` values and single-flights concurrent loads of the same key.
- A second bounded, TTL-aware `moka` cache stores raw schema bytes in memory.
- `cacache` stores raw schema bytes across processes. Application metadata provides expiry and oldest-first indexed-data eviction.

Local cache keys contain the canonical path plus a SHA-256 digest of the root XSD and every recursively referenced local schema. Remote cache keys use the normalized URL. Cache removal and clearing invalidate compiled and raw entries.

## libxml2 boundary and trust model

The FFI is confined to [`src/backend.rs`](src/backend.rs). It uses checked buffer lengths, RAII wrappers, per-context structured error callbacks, and a distinct validation context per call. Schema compilation is serialized; successfully compiled schemas are immutable and shared. Caller-owned local schema documents remain alive for the compiled schema's lifetime and are freed after the schema.

Input documents are parsed with `XML_PARSE_NONET` and without entity substitution. The validator never calls libxml2's global cleanup function. Unix builds select libxml2 through `pkg-config`; macOS CI additionally verifies that the release binary links Homebrew's library instead of Apple's system copy.

These controls narrow the unsafe and I/O boundaries; they do not make arbitrary hostile XML or XSD risk-free. Keep libxml2 patched and use the tool only with documents and schemas from sources appropriate to your threat model. The separate [`libxml2-thread-safety-test`](https://github.com/FranklinChen/libxml2-thread-safety-test) stress-tests the shared-schema/per-call-context pattern, but is not a general security proof.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
cargo build --locked --release --all-features
cargo bench --locked --all-features --no-run
```

The integration tests use Cargo's profile-correct `CARGO_BIN_EXE_validate-xml` binary. Run `cargo test --lib` for a faster unit-only loop. Divan benchmarks exercise the same libxml2 compile/validate path as the application; this repository does not publish historical microbenchmark numbers as current guarantees.

See [`AGENTS.md`](AGENTS.md) for repository architecture notes and [`CHANGELOG.md`](CHANGELOG.md) for release history.

## License

[MIT](LICENSE)
