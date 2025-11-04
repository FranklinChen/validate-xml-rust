# validate-xml User Guide

**Version**: 0.2.0
**Last Updated**: 2025-11-01
**License**: MIT

## Table of Contents

1. [Installation](#installation)
2. [Quick Start](#quick-start)
3. [Basic Usage](#basic-usage)
4. [Advanced Features](#advanced-features)
5. [Configuration](#configuration)
6. [Troubleshooting](#troubleshooting)
7. [Performance Tips](#performance-tips)
8. [Integration](#integration)

---

## Installation

### From Source (Development)

```bash
git clone https://github.com/your-org/validate-xml.git
cd validate-xml
cargo build --release
./target/release/validate-xml --version
```

### From Cargo

```bash
cargo install validate-xml
```

### Docker

```bash
docker build -t validate-xml .
docker run validate-xml --help
```

---

## Quick Start

### Validate a Single XML File

```bash
validate-xml /path/to/file.xml
```

Output:
```
✓ file.xml: VALID
All 1 files passed validation.
```

### Validate All XML Files in a Directory

```bash
validate-xml /path/to/xml/files/
```

### Validate with Specific Extension

```bash
validate-xml /path/to/files --extension=cmdi
```

### See Results in JSON Format

```bash
validate-xml /path/to/files --output=json | jq .
```

---

## Basic Usage

### Command Syntax

```bash
validate-xml <DIRECTORY> [OPTIONS]
```

### Common Options

| Option | Description | Example |
|--------|-------------|---------|
| `--extension=EXT` | File extension to validate (default: cmdi) | `--extension=xml` |
| `--concurrency=N` | Number of parallel validators (default: all cores) | `--concurrency=4` |
| `--verbose` / `-v` | Show progress during validation | `-v` |
| `--output=FORMAT` | Output format: `text` (default) or `json` | `--output=json` |
| `--help` | Display help information | `--help` |
| `--version` | Display version information | `--version` |

### Basic Examples

**Validate all XML files with progress:**
```bash
validate-xml /data/xml/files -v
```

**Validate with limited parallelism:**
```bash
validate-xml /data/xml/files --concurrency=2
```

**Validate specific file type:**
```bash
validate-xml /data --extension=xsd
```

**Get JSON output for processing:**
```bash
validate-xml /data --output=json > results.json
```

---

## Advanced Features

### 1. Remote Schema Caching

The tool automatically caches schemas from remote URLs to improve performance:

```bash
# First run: Downloads schemas
validate-xml /data/xml/files

# Subsequent runs: Reuses cached schemas (much faster)
validate-xml /data/xml/files
```

**Cache Location**: `~/.validate-xml/cache/` (configurable)

### 2. Concurrent Validation

Automatically parallelizes validation across all CPU cores:

```bash
# On 8-core system, validates 8 files simultaneously
validate-xml /data/xml/files
```

**Concurrency Control**:
```bash
# Limit to 4 concurrent validators
validate-xml /data/xml/files --concurrency=4
```

### 3. Progress Reporting

Track progress on large batch jobs:

```bash
validate-xml /data/xml/files --verbose
# Output:
# Processed 100/50000 files (0.2%)...
# Processed 1000/50000 files (2.0%)...
```

### 4. Error Details

Validation errors include:
- Filename
- Line number where error occurred
- Column number
- Error message

```
✗ document.xml (line 42, col 15):
  Element 'name': Missing required attribute 'id'
```

### 5. JSON Output for Integration

Machine-readable output suitable for CI/CD:

```bash
validate-xml /data --output=json
```

JSON Structure:
```json
{
  "summary": {
    "total_files": 100,
    "valid_files": 98,
    "invalid_files": 2,
    "elapsed_seconds": 2.5
  },
  "results": [
    {
      "file": "document.xml",
      "status": "valid"
    },
    {
      "file": "invalid.xml",
      "status": "invalid",
      "errors": [
        {
          "line": 42,
          "column": 15,
          "message": "Missing required attribute 'id'"
        }
      ]
    }
  ]
}
```

---

## Configuration

### Configuration File

Create `~/.validate-xml/config.toml`:

```toml
# Cache configuration
[cache]
directory = "~/.validate-xml/cache"
ttl_hours = 24                    # Cache validity in hours
max_size_mb = 100                 # Maximum cache size
max_memory_entries = 1000         # Max schemas in memory
memory_ttl_seconds = 3600         # In-memory cache TTL

# Validation rules
[validation]
require_schema = true             # Require schema declaration
fail_on_warning = false           # Fail on XSD warnings

# Network settings
[network]
timeout_seconds = 30              # HTTP request timeout
retry_attempts = 3                # Retry failed downloads

# File discovery
[file_discovery]
recursive = true                  # Recursively search subdirectories
skip_hidden = true                # Skip hidden files
```

### Environment Variables

Override config file settings:

```bash
# Cache directory
export VALIDATE_XML_CACHE_DIR=/tmp/cache

# HTTP timeout
export VALIDATE_XML_TIMEOUT_SECONDS=60

# Concurrency
export VALIDATE_XML_CONCURRENCY=8
```

---

## Troubleshooting

### "No files found"

**Problem**: Directory contains no files matching the extension.

**Solution**:
```bash
# Check file extensions in directory
ls /path/to/files

# Specify correct extension
validate-xml /path/to/files --extension=xml
```

### "Schema not found"

**Problem**: XML file references a schema that doesn't exist or is unreachable.

**Solution**:
```bash
# Check schema URL in XML file
grep -i "schema" file.xml

# Verify URL is accessible
curl -I http://schema.example.com/schema.xsd

# If network issue, increase timeout
VALIDATE_XML_TIMEOUT_SECONDS=60 validate-xml /path/to/files
```

### "Too slow" or "High memory usage"

**Problem**: Validation takes too long or uses too much memory.

**Solution**:
```bash
# Reduce parallelism
validate-xml /data --concurrency=2

# Clear cache to free memory
rm -rf ~/.validate-xml/cache

# Validate in batches
validate-xml /data/batch1
validate-xml /data/batch2
```

### "Permission denied"

**Problem**: Cannot read files in directory.

**Solution**:
```bash
# Check directory permissions
ls -ld /path/to/files

# Fix permissions
chmod -R 755 /path/to/files
```

---

## Performance Tips

### 1. Use Concurrency Wisely

- **8+ cores**: Use default (all cores)
- **4 cores**: Use `--concurrency=4`
- **2 cores**: Use `--concurrency=2`
- **Memory-constrained**: Use `--concurrency=1` or `--concurrency=2`

### 2. Leverage Schema Caching

```bash
# First run: Caches all schemas (slower)
validate-xml /data/xml/files  # ~30 seconds for 20k files

# Second run: Uses cached schemas (faster)
validate-xml /data/xml/files  # ~5 seconds for 20k files
```

**Cache Hit Rate**: Run `--verbose` to see schema hit statistics.

### 3. Batch Large Jobs

For very large directories (100k+ files), validate in batches:

```bash
# Process in chunks
validate-xml /data/2023
validate-xml /data/2024

# Or split by type
validate-xml /data --extension=xml
validate-xml /data --extension=cmdi
```

### 4. Monitor Resource Usage

```bash
# Show progress with resource info
validate-xml /data --verbose

# Monitor with system tools
# macOS:
top -l 1 | grep validate-xml

# Linux:
watch -n 1 'ps aux | grep validate-xml'
```

### 5. Optimize Configuration

For throughput optimization:

```toml
[validation]
# Skip expensive schema validation (if only checking format)
require_schema = false

[network]
# Adjust based on network reliability
timeout_seconds = 60
retry_attempts = 5
```

---

## Integration

### CI/CD Pipeline (GitHub Actions)

```yaml
name: XML Validation

on: [push, pull_request]

jobs:
  validate-xml:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Install validate-xml
        run: cargo install validate-xml

      - name: Validate XML files
        run: validate-xml ./data --output=json > validation-results.json

      - name: Check validation results
        run: |
          INVALID=$(jq '.summary.invalid_files' validation-results.json)
          if [ "$INVALID" -gt 0 ]; then
            echo "❌ $INVALID files failed validation"
            exit 1
          fi
          echo "✓ All files passed validation"
```

### Shell Script Integration

```bash
#!/bin/bash
# Validate and process results

RESULTS=$(validate-xml /data --output=json)

INVALID=$(echo "$RESULTS" | jq '.summary.invalid_files')

if [ "$INVALID" -gt 0 ]; then
    echo "❌ Validation failed: $INVALID files invalid"
    echo "$RESULTS" | jq '.results[] | select(.status == "invalid")'
    exit 1
fi

echo "✓ Validation successful"
exit 0
```

### Docker Integration

```dockerfile
FROM rust:latest

WORKDIR /app
RUN cargo install validate-xml

COPY data/ /data/

CMD ["validate-xml", "/data", "--output=json"]
```

---

## FAQ

**Q: What XML versions are supported?**
A: Standard XML 1.0 with XSD schema validation (XML 1.1 support planned).

**Q: Can I validate against remote schemas?**
A: Yes, XML files can reference schemas via HTTP(S) URLs. They are automatically cached.

**Q: What's the performance compared to other tools?**
A: validate-xml is 10-100x faster than sequential validation due to concurrency and caching.

**Q: How much disk space does the cache use?**
A: Configurable (default 100MB). Automatic LRU eviction when full.

**Q: Can I use this as a library?**
A: Yes! Use the `validate-xml` crate in Cargo.toml for programmatic access.

**Q: What's the exit code behavior?**
A: Exit 0 = all valid, Exit 1 = validation failed, Exit 2+ = system error.

---

## Support

- **Documentation**: https://docs.example.com/validate-xml
- **Issues**: https://github.com/your-org/validate-xml/issues
- **Discussions**: https://github.com/your-org/validate-xml/discussions
- **Email**: support@example.com

---

## License

MIT License - See LICENSE file for details
