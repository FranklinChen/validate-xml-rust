# Changelog

## [0.4.0] - 2026-04-15

### Breaking Changes

- **`SchemaReference` is now an enum**, not a struct (fixes GitHub issue #235
  and a latent cache-key bug). Was `struct SchemaReference { url: String,
  source_type: SchemaSourceType }` + separate `enum SchemaSourceType`; is now
  `enum SchemaReference { Local(PathBuf), Remote(String) }` with a
  `cache_key()` method and `Display` impl.
  - The removed `url: String` field previously held the *unresolved* raw
    reference for `Local` variants (e.g. `"schema.xsd"`), which was used
    as the L1 parsed-schema cache key — two XML files in different
    directories each referencing `"schema.xsd"` would silently share a
    cache entry despite resolving to different files on disk. Cache keys
    now come from `SchemaReference::cache_key()`, which namespaces local
    paths with a `local:` prefix and uses the fully resolved path.
  - `SchemaSourceType` is gone; callers pattern-match directly on
    `SchemaReference`.
  - `extract_schema_url_async` is removed (no non-test callers).

### Added

- **Streaming XML pull parser for schema-URL extraction** (quick-xml
  `NsReader`). The extractor walks only the prolog + root element's start
  tag, stopping immediately afterward. Fixes all four failure modes in
  GitHub issue #235:
  - `xsi:schemaLocation` inside a leading XML comment is now correctly
    ignored.
  - Multi-pair `xsi:schemaLocation="ns1 loc1 ns2 loc2 ..."` now emits one
    `SchemaReference` per (ns, location) pair.
  - Multi-line `xsi:schemaLocation` attribute values are now handled.
  - `<?xml-model href="..." ?>` processing instructions are now honored.
- **Namespace-URI-based attribute resolution**: xsi attributes are matched
  by namespace URI (`http://www.w3.org/2001/XMLSchema-instance`), not by
  literal `xsi:` prefix, so documents that bind xsi to a non-standard
  prefix are handled correctly.
- **New `ValidationError::XmlParsing { file, details }` variant** for
  errors while parsing the XML *input* file (distinct from
  `SchemaParsing`, which is for XSD content). The old code stuffed XML
  file paths into `SchemaParsing.url`, a leaky abstraction.
- **Root-element probe for fetched XSD bytes**: before caching, the loader
  confirms the root element is `{http://www.w3.org/2001/XMLSchema}schema`
  via quick-xml, avoiding poisoning the disk cache with HTML/404
  responses. Replaces a substring-based `contains("<xs:schema")` check.
- **Odd-token `xsi:schemaLocation` now surfaces as `XmlParsing` error**
  instead of silently dropping the orphan token.

### Changed

- **Bounded file I/O**: extractor now uses a sync `std::io::BufReader`
  inside `tokio::task::spawn_blocking`. Because the pull parser stops at
  the root element's start tag, only prolog + root-tag bytes are actually
  read off disk — large XML bodies are never loaded into memory.
- **CLAUDE.md updated** with the new module design, error model
  distinction (XmlParsing vs SchemaParsing), and cache-key rationale.
- All `schema_loader.rs` tests converted to return
  `Result<(), ValidationError>` and propagate via `?` rather than using
  `.unwrap()` / `.expect()`, per the project's no-panic rule.

### Removed

- `regex` dependency (no longer used after extractor rewrite).
- `SchemaSourceType` enum (collapsed into `SchemaReference`).
- `extract_schema_url_async` convenience function (no non-test callers).
- 60+ lines of hand-rolled PI pseudo-attribute parsing (`split_pi_target`,
  `find_pseudo_attr`, `parse_xml_model_href`) — replaced by quick-xml's
  `BytesPI::target()` and `BytesPI::attributes()`.

### Added (deps)

- `quick-xml = "0.37"` for the streaming pull parser.

### Known Limitations

- `SchemaLoader::load_schema_for_file` still loads only the *first*
  schema reference. True multi-schema composition would require upstream
  `xmloxide` changes: as of 0.3.x it has no schema-merge API and no
  `xs:import`/`xs:include` support, and `validate_xsd` takes a single
  schema. Until that lands, extra references are dropped.

## [0.3.0] - 2026-03-04

### Breaking Changes

- **Replaced libxml2 C FFI with xmloxide (pure Rust)**
  - Removed `LibXml2Wrapper`, `XmlSchemaPtr`, `LibXml2Error`, `XmlValidationError` — no wrapper types needed
  - Removed `xml_validator.rs` / `libxml2.rs` module entirely — `validator.rs` calls xmloxide directly
  - Removed `ValidationError::LibXml2Internal` / `XmlValidationInternal` — errors use existing `SchemaParsing` and `ValidationFailed` variants
  - Removed custom `ValidationResult` enum — xmloxide's `ValidationResult` used directly
  - Removed `validate_memory` method (was unimplemented stub)

### Added

- Pure Rust XML/XSD validation via the [xmloxide](https://crates.io/crates/xmloxide) crate (v0.1.1)
- File validation tests in `xml_validator` module

### Removed

- **System dependency on libxml2** — no more `brew install libxml2` or `apt-get install libxml2-dev`
- All `unsafe` code — no more C FFI bindings, raw pointers, or manual memory management
- Global mutex (`LIBXML2_GLOBAL_LOCK`) — xmloxide is natively thread-safe
- `libc` dependency
- `once_cell` dependency
- `rayon` dev-dependency

### Changed

- MSRV raised from 1.70 to 1.81 (xmloxide requirement)
- CI workflow no longer installs system libxml2 on any platform
- Schema parsing no longer requires serialization — xmloxide's `parse_xsd` is thread-safe
- Validation errors now include line/column information from xmloxide's `ValidationError`

## [0.2.1] - Previous release

- Schema override functionality (`--schema` flag)
- globset-based file pattern matching
- libxml2 FFI-based validation
