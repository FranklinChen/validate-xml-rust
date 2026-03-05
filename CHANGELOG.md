# Changelog

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
