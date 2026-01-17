# validate-xml: High-Performance XML Schema Validator

[![CI](https://github.com/FranklinChen/validate-xml-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/FranklinChen/validate-xml-rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.70+](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)

A blazingly fast CLI tool for validating XML files against XML Schemas, built in Rust with a focus on concurrent processing, intelligent caching, and low memory overhead.

**Validate 20,000 files in seconds** with automatic schema caching, concurrent validation, and comprehensive error reporting.

---

## Features

✨ **Core Capabilities**
- **Concurrent Validation**: Uses all available CPU cores for parallel XML/XSD validation
- **Schema Caching**: Two-tier caching (L1 memory, L2 disk) prevents redundant downloads
- **Batch Processing**: Validate entire directory trees (100,000+ files) without memory exhaustion
- **Output**: Text (human-readable) or Compact Summary
- **Smart Error Reporting**: Line/column numbers, clear error messages, detailed diagnostics

⚡ **Performance**
- **C FFI**: Direct bindings to libxml2 for native XML/XSD validation
- **Async I/O**: Tokio-based async operations for files and HTTP downloads
- **In-Memory Caching**: First-run download + cross-run disk cache for schema reuse
- **Bounded Memory**: Concurrent validation with configurable limits

🏗️ **Architecture**
- **Hybrid Async/Sync**: Async I/O (files, HTTP, caching) + sync CPU-bound validation (libxml2)
- **True Parallel Validation**: No global locks - 10x throughput on multi-core CPUs
- **Parse Once, Validate Many**: Schemas are parsed once and shared safely across threads
- **Modular Design**: Clean separation of concerns (discovery, loading, validation, reporting)
- **Non-Blocking**: CPU-intensive tasks are offloaded to `spawn_blocking` to keep the async runtime responsive.

---

## Prerequisites

- **Rust**: 1.70+ (stable toolchain) with Cargo
- **libxml2**: System library for XML validation
  - macOS: `brew install libxml2`
  - Ubuntu/Debian: `sudo apt-get install libxml2-dev`
  - CentOS/RHEL: `sudo yum install libxml2-devel`
  - Windows: `vcpkg install libxml2:x64-windows`

---

## Installation

### From Source

```bash
git clone https://github.com/franklinchen/validate-xml-rust.git
cd validate-xml-rust
cargo install --path .
```

This installs the `validate-xml` binary to `~/.cargo/bin`. Add `~/.cargo/bin` to your `$PATH` if not already present.

---

## Quick Start

### Basic Usage

Validate all XML files in a directory:

```bash
# Validate all .xml files (recursive)
validate-xml /path/to/xml/files

# Validate files with custom extensions
validate-xml --extensions xml,xsd /path/to/files

# Validate with verbose output and progress bar
validate-xml --verbose /path/to/files
```

### Output Formats

Standard output includes validation status per file (in verbose mode) and a final summary.

```
Validation Summary:
  Total files: 20000
  Valid: 19950
  Invalid: 50
  Errors: 0
  Skipped: 0
  Success rate: 99.8%
  Duration: 4.20s
```

### Error Message Format

Validation errors are reported with precise location information for easy IDE integration:

```
path/to/file.xml:42:15: Missing required element 'id'
path/to/file.xml:87:3: Element 'invalid' not allowed here
path/to/file.xml:120:1: Schema error: Could not locate schema resource
```

### Remote Schema Example

A robust way to test remote schema validation is using an Apache Maven POM file. A sample is provided in `samples/pom.xml`:

```bash
# Validate the provided sample which uses a remote Apache Maven schema
validate-xml samples/pom.xml
```

---

## Command-Line Reference

### Basic Syntax

```bash
validate-xml [OPTIONS] <DIRECTORY>
```

### Options

| Option | Default | Description |
|--------|---------|-------------|
| `--extensions <EXT>` | `xml` | XML file extension to match (comma-separated) |
| `--threads <N>` | CPU cores | Max concurrent validation threads |
| `--cache-dir <PATH>` | *Platform specific* | Schema cache directory |
| `--cache-ttl <HOURS>` | `24` | Schema cache TTL in hours |
| `--verbose` | - | Show detailed output |
| `--quiet` | - | Suppress non-error output |
| `--progress` | Auto | Show progress bar |
| `--fail-fast` | - | Stop validation on first error |
| `--help` | - | Show help message |
| `--version` | - | Show version information |

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | All files valid |
| `1` | Configuration or CLI error |
| `2` | Errors occurred during validation (system/network) |
| `3` | Invalid files found (schema violations) |

---

## How It Works

### Architecture

The validator consists of four main components:

**1. File Discovery**
- Recursively traverses directory tree and filters by extension.

**2. Schema Loading**
- Extracts schema URLs (xsi:schemaLocation, xsi:noNamespaceSchemaLocation).
- Downloads remote schemas (HTTP/HTTPS) and caches raw bytes to memory and disk.
- **Parse Once**: Parsed schema structures are cached in memory and shared safely across threads.

**3. Concurrent Validation**
- Spawns async tasks bounded by `--threads`.
- Heavy CPU tasks (parsing, validation) are offloaded to `spawn_blocking`.
- **Thread Safety**: Uses a global lock for non-thread-safe library initialization and schema parsing, while allowing **full parallel execution** for XML validation.

**4. Error Reporting**
- Aggregates and formats errors with line/column information.

### Caching Strategy

- **L1 Parsed Cache**: In-memory `moka` cache storing compiled `XmlSchemaPtr`. Ensures we parse any XSD exactly once.
- **L2 Raw Cache**: Disk-backed `cacache` for persistent cross-run storage of schema bytes.

---

## Performance Characteristics

### Benchmarks (divan)

Micro-benchmarks measuring the core validation engine (on Apple M1 Max):

| Operation | Median Time | Throughput |
|-----------|-------------|------------|
| Schema Parsing | ~6.0 µs | 166,000/sec |
| Valid XML Validation | ~17.2 µs | 58,000/sec |
| Invalid XML Validation | ~17.6 µs | 56,000/sec |

*Note: Validation includes reading the XML file and checking against the cached schema.*

---

## Development

### Building from Source

```bash
# Clone repository
git clone https://github.com/franklinchen/validate-xml-rust.git
cd validate-xml-rust

# Build (release, optimized)
cargo build --release

# Run tests
cargo test

# Run benchmarks
cargo bench
```

### Testing & Quality

Before submitting changes, ensure you run:
- `cargo fmt`
- `cargo clippy`
- `cargo test`

---

## License

MIT License - See LICENSE file for details

---

## Acknowledgements

Google Gemini was used as an aid in improving this project, particularly in streamlining the architecture and test suite.
