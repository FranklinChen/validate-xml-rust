use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::fs;

/// Test fixture paths
pub struct TestFixtures {
    pub fixtures_dir: PathBuf,
}

impl TestFixtures {
    pub fn new() -> Self {
        let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");

        Self { fixtures_dir }
    }

    pub fn xml_valid_dir(&self) -> PathBuf {
        self.fixtures_dir.join("xml").join("valid")
    }

    pub fn xml_invalid_dir(&self) -> PathBuf {
        self.fixtures_dir.join("xml").join("invalid")
    }

    pub fn xml_malformed_dir(&self) -> PathBuf {
        self.fixtures_dir.join("xml").join("malformed")
    }

    pub fn schemas_local_dir(&self) -> PathBuf {
        self.fixtures_dir.join("schemas").join("local")
    }

    pub fn configs_dir(&self) -> PathBuf {
        self.fixtures_dir.join("configs")
    }

    pub fn simple_schema(&self) -> PathBuf {
        self.schemas_local_dir().join("simple.xsd")
    }

    pub fn complex_schema(&self) -> PathBuf {
        self.schemas_local_dir().join("complex.xsd")
    }

    pub fn strict_schema(&self) -> PathBuf {
        self.schemas_local_dir().join("strict.xsd")
    }

    pub fn simple_valid_xml(&self) -> PathBuf {
        self.xml_valid_dir().join("simple_valid.xml")
    }

    pub fn simple_invalid_xml(&self) -> PathBuf {
        self.xml_invalid_dir().join("simple_invalid.xml")
    }

    pub fn malformed_xml(&self) -> PathBuf {
        self.xml_malformed_dir().join("not_well_formed.xml")
    }
}

/// Create a temporary directory with test XML files
pub async fn create_temp_xml_files() -> std::io::Result<TempDir> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // Create directory structure
    fs::create_dir_all(root.join("project1")).await?;
    fs::create_dir_all(root.join("project2/schemas")).await?;
    fs::create_dir_all(root.join("ignored")).await?;

    // Create XML files with schema references
    fs::write(
        root.join("document1.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/schema1 http://example.com/schema1.xsd">
    <element>content</element>
</root>"#,
    )
    .await?;

    fs::write(
        root.join("project1/document2.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<data xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="local-schema.xsd">
    <item>value</item>
</data>"#,
    )
    .await?;

    fs::write(
        root.join("project2/document3.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:schemaLocation="http://example.com/config https://schemas.example.com/config.xsd">
    <setting>enabled</setting>
</config>"#,
    )
    .await?;

    // Create some non-XML files
    fs::write(root.join("readme.txt"), "This is a readme file").await?;
    fs::write(root.join("project1/config.json"), r#"{"key": "value"}"#).await?;

    // Create files in ignored directory
    fs::write(
        root.join("ignored/ignored.xml"),
        r#"<?xml version="1.0"?><ignored/>"#,
    )
    .await?;

    Ok(temp_dir)
}

/// Performance measurement utilities
pub struct PerformanceTimer {
    start: std::time::Instant,
}

impl PerformanceTimer {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.start.elapsed()
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.elapsed().as_millis()
    }
}

/// Memory usage measurement
pub fn get_memory_usage() -> Option<usize> {
    // This is a simplified version - in a real implementation,
    // you might use system-specific APIs or crates like `sysinfo`
    None
}

/// Assert that a duration is within expected bounds
pub fn assert_duration_within_bounds(
    actual: std::time::Duration,
    min: std::time::Duration,
    max: std::time::Duration,
) {
    assert!(
        actual >= min && actual <= max,
        "Duration {:?} not within bounds [{:?}, {:?}]",
        actual,
        min,
        max
    );
}

/// Async test utilities
pub async fn wait_for_condition<F, Fut>(mut condition: F, timeout: std::time::Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if condition().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    false
}

/// File system test utilities
pub async fn create_test_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, content).await
}

pub async fn file_exists(path: &Path) -> bool {
    fs::metadata(path).await.is_ok()
}

/// Schema content constants for testing
pub const SIMPLE_XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

pub const VALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>Hello World</root>"#;

pub const INVALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root><invalid>content</invalid></root>"#;

pub const MALFORMED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root><unclosed>"#;
