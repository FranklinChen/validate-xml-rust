use std::path::Path;
use tempfile::TempDir;
use tokio::fs;
use validate_xml::{FileDiscovery, SchemaExtractor, ValidationError};

async fn create_test_xml_files() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Create directory structure
    fs::create_dir_all(root.join("project1")).await.unwrap();
    fs::create_dir_all(root.join("project2/schemas"))
        .await
        .unwrap();
    fs::create_dir_all(root.join("ignored")).await.unwrap();

    // Create XML files with schema references
    fs::write(
        root.join("document1.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/schema1 http://example.com/schema1.xsd">
    <element>content</element>
</root>"#,
    )
    .await
    .unwrap();

    fs::write(
        root.join("project1/document2.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<data xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="local-schema.xsd">
    <item>value</item>
</data>"#,
    )
    .await
    .unwrap();

    fs::write(
        root.join("project2/document3.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:schemaLocation="http://example.com/config https://schemas.example.com/config.xsd">
    <setting>enabled</setting>
</config>"#,
    )
    .await
    .unwrap();

    // Create some non-XML files
    fs::write(root.join("readme.txt"), "This is a readme file")
        .await
        .unwrap();
    fs::write(root.join("project1/config.json"), r#"{"key": "value"}"#)
        .await
        .unwrap();

    // Create files in ignored directory
    fs::write(
        root.join("ignored/ignored.xml"),
        r#"<?xml version="1.0"?><ignored/>"#,
    )
    .await
    .unwrap();

    temp_dir
}

#[tokio::test]
async fn test_file_discovery_with_schema_extraction() {
    let temp_dir = create_test_xml_files().await;

    // Test basic file discovery
    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 4 XML files (including ignored.xml)
    assert_eq!(files.len(), 4);

    // Verify all found files are XML files
    for file in &files {
        assert!(file.extension().unwrap() == "xml");
    }

    // Test schema extraction from discovered files
    let extractor = SchemaExtractor::new().unwrap();
    let mut schema_urls = Vec::new();

    for file in &files {
        if let Ok(refs) = extractor.extract_schema_urls(file).await {
            for schema_ref in refs {
                schema_urls.push(schema_ref.url);
            }
        }
    }

    // Should find 3 schema references (ignored.xml has no schema)
    assert_eq!(schema_urls.len(), 3);
    assert!(schema_urls.contains(&"http://example.com/schema1.xsd".to_string()));
    assert!(schema_urls.contains(&"local-schema.xsd".to_string()));
    assert!(schema_urls.contains(&"https://schemas.example.com/config.xsd".to_string()));
}

#[tokio::test]
async fn test_file_discovery_with_patterns() {
    let temp_dir = create_test_xml_files().await;

    // Test with include patterns
    let discovery = FileDiscovery::new()
        .with_include_patterns(vec!["**/project1/**".to_string()])
        .unwrap();

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should only find files in project1 directory
    assert_eq!(files.len(), 1);
    assert!(files[0].to_string_lossy().contains("project1"));
    assert!(files[0].file_name().unwrap() == "document2.xml");
}

#[tokio::test]
async fn test_file_discovery_with_exclude_patterns() {
    let temp_dir = create_test_xml_files().await;

    // Test with exclude patterns
    let discovery = FileDiscovery::new()
        .with_exclude_patterns(vec!["**/ignored/**".to_string()])
        .unwrap();

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 3 XML files (excluding ignored.xml)
    assert_eq!(files.len(), 3);

    // Verify ignored.xml is not included
    for file in &files {
        assert!(!file.to_string_lossy().contains("ignored"));
    }
}

#[tokio::test]
async fn test_file_discovery_with_multiple_extensions() {
    let temp_dir = create_test_xml_files().await;

    // Add some XSD files
    fs::write(
        temp_dir.path().join("schema1.xsd"),
        r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
</xs:schema>"#,
    )
    .await
    .unwrap();

    fs::write(
        temp_dir.path().join("project2/schemas/config.xsd"),
        r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
</xs:schema>"#,
    )
    .await
    .unwrap();

    // Test discovery with multiple extensions
    let discovery =
        FileDiscovery::new().with_extensions(vec!["xml".to_string(), "xsd".to_string()]);

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 6 files (4 XML + 2 XSD)
    assert_eq!(files.len(), 6);

    // Verify we have both XML and XSD files
    let xml_count = files
        .iter()
        .filter(|f| f.extension().unwrap() == "xml")
        .count();
    let xsd_count = files
        .iter()
        .filter(|f| f.extension().unwrap() == "xsd")
        .count();

    assert_eq!(xml_count, 4);
    assert_eq!(xsd_count, 2);
}

#[tokio::test]
async fn test_file_discovery_with_depth_limit() {
    let temp_dir = create_test_xml_files().await;

    // Test with depth limit of 0 (only root directory)
    let discovery = FileDiscovery::new().with_max_depth(Some(0));
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should only find document1.xml in root
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap(), "document1.xml");

    // Test with depth limit of 1 (root + one level)
    let discovery = FileDiscovery::new().with_max_depth(Some(1));
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 3 files (document1.xml, document2.xml, document3.xml)
    // but not ignored.xml which is at depth 1 but in ignored directory
    assert_eq!(files.len(), 4); // All files since they're all at depth 0 or 1
}

#[tokio::test]
async fn test_async_file_operations() {
    let temp_dir = create_test_xml_files().await;

    // Test that file discovery is truly async by processing multiple directories concurrently
    let discovery = FileDiscovery::new();

    // Create paths with proper lifetimes
    let project1_path = temp_dir.path().join("project1");
    let project2_path = temp_dir.path().join("project2");

    // Create multiple discovery tasks
    let tasks = vec![
        discovery.discover_files(temp_dir.path()),
        discovery.discover_files(&project1_path),
        discovery.discover_files(&project2_path),
    ];

    // Run all tasks concurrently
    let results = futures::future::try_join_all(tasks).await.unwrap();

    // Verify results
    assert_eq!(results[0].len(), 4); // All XML files
    assert_eq!(results[1].len(), 1); // Only document2.xml
    assert_eq!(results[2].len(), 1); // Only document3.xml
}

#[tokio::test]
async fn test_error_handling() {
    let discovery = FileDiscovery::new();

    // Test with non-existent directory
    let result = discovery
        .discover_files(Path::new("/non/existent/path"))
        .await;
    assert!(result.is_err());

    match result.unwrap_err() {
        ValidationError::Io(_) => {} // Expected
        _ => panic!("Expected IO error"),
    }
}
