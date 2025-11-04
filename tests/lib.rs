//! Comprehensive test suite for XML Validator CLI Tool
//!
//! This test suite validates the core functionality of the validate-xml tool:
//! - Unit tests for individual modules (cache, config, error handling)
//! - Integration tests for end-to-end validation workflows
//! - Performance benchmarks for throughput and caching efficiency
//! - Mock implementations for testing external dependencies (HTTP, file I/O)
//!
//! ## Running Tests
//!
//! Run all tests:
//! ```bash
//! cargo test --all
//! ```
//!
//! Run specific test module:
//! ```bash
//! cargo test --lib unit::cache_tests
//! ```
//!
//! Run with output:
//! ```bash
//! cargo test -- --nocapture
//! ```

// Common test utilities and helpers
pub mod common;

// Unit tests for individual modules (currently being refactored for API compatibility)
pub mod unit;

// Re-export commonly used test utilities
pub use common::mocks::*;
pub use common::test_helpers::*;
