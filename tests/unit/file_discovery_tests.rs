use std::path::Path;
use tempfile::TempDir;
use tokio::fs;

use crate::common::test_helpers::create_temp_xml_files;
use validate_xml::{FileDiscovery, ValidationError};

#[tokio::test]
async fn test_basic_file_discovery() {
    let temp_dir = create_temp_xml_files().await.unwrap();

    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find all XML files
    assert_eq!(files.len(), 4); // document1.xml, document2.xml, document3.xml, ignored.xml

    // Verify all found files are XML files
    for file in &files {
        assert_eq!(file.extension().unwrap(), "xml");
    }
}

#[tokio::test]
async fn test_file_discovery_with_custom_extensions() {
    let temp_dir = TempDir::new().unwrap();

    // Create files with different extensions
    fs::write(temp_dir.path().join("test.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("test.xsd"), "<schema/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("test.cmdi"), "<cmdi/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("test.txt"), "text")
        .await
        .unwrap();

    let discovery = FileDiscovery::new().with_extensions(vec![
        "xml".to_string(),
        "xsd".to_string(),
        "cmdi".to_string(),
    ]);

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 3 files (excluding .txt)
    assert_eq!(files.len(), 3);

    let extensions: std::collections::HashSet<_> = files
        .iter()
        .map(|f| f.extension().unwrap().to_str().unwrap())
        .collect();

    assert!(extensions.contains("xml"));
    assert!(extensions.contains("xsd"));
    assert!(extensions.contains("cmdi"));
    assert!(!extensions.contains("txt"));
}

#[tokio::test]
#[ignore]
async fn test_file_discovery_with_include_patterns() {
    let temp_dir = TempDir::new().unwrap();

    // Create directory structure
    fs::create_dir_all(temp_dir.path().join("src"))
        .await
        .unwrap();
    fs::create_dir_all(temp_dir.path().join("tests"))
        .await
        .unwrap();
    fs::create_dir_all(temp_dir.path().join("docs"))
        .await
        .unwrap();

    // Create files
    fs::write(temp_dir.path().join("src/main.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("tests/test.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("docs/doc.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("root.xml"), "<xml/>")
        .await
        .unwrap();

    let discovery = FileDiscovery::new()
        .with_include_patterns(vec!["src/**".to_string(), "root.xml".to_string()])
        .unwrap();

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 2 files (src/main.xml and root.xml)
    assert_eq!(files.len(), 2);

    let file_names: std::collections::HashSet<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_str().unwrap())
        .collect();

    assert!(file_names.contains("main.xml"));
    assert!(file_names.contains("root.xml"));
    assert!(!file_names.contains("test.xml"));
    assert!(!file_names.contains("doc.xml"));
}

#[tokio::test]
#[ignore]
async fn test_file_discovery_with_exclude_patterns() {
    let temp_dir = TempDir::new().unwrap();

    // Create directory structure
    fs::create_dir_all(temp_dir.path().join("src"))
        .await
        .unwrap();
    fs::create_dir_all(temp_dir.path().join("target"))
        .await
        .unwrap();
    fs::create_dir_all(temp_dir.path().join(".git"))
        .await
        .unwrap();

    // Create files
    fs::write(temp_dir.path().join("src/main.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("target/build.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join(".git/config.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("root.xml"), "<xml/>")
        .await
        .unwrap();

    let discovery = FileDiscovery::new()
        .with_exclude_patterns(vec!["target/**".to_string(), ".*/**".to_string()])
        .unwrap();

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 2 files (src/main.xml and root.xml)
    assert_eq!(files.len(), 2);

    let file_names: std::collections::HashSet<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_str().unwrap())
        .collect();

    assert!(file_names.contains("main.xml"));
    assert!(file_names.contains("root.xml"));
    assert!(!file_names.contains("build.xml"));
    assert!(!file_names.contains("config.xml"));
}

#[tokio::test]
async fn test_file_discovery_with_max_depth() {
    let temp_dir = TempDir::new().unwrap();

    // Create nested directory structure
    fs::create_dir_all(temp_dir.path().join("level1/level2/level3"))
        .await
        .unwrap();

    // Create files at different depths
    fs::write(temp_dir.path().join("root.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("level1/file1.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("level1/level2/file2.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(
        temp_dir.path().join("level1/level2/level3/file3.xml"),
        "<xml/>",
    )
    .await
    .unwrap();

    // Test with max depth 1
    let discovery = FileDiscovery::new().with_max_depth(Some(1));
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 2 files (root.xml and level1/file1.xml)
    assert_eq!(files.len(), 2);

    // Test with max depth 2
    let discovery = FileDiscovery::new().with_max_depth(Some(2));
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find 3 files (excluding level3/file3.xml)
    assert_eq!(files.len(), 3);

    // Test with no depth limit
    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should find all 4 files
    assert_eq!(files.len(), 4);
}

#[tokio::test]
async fn test_file_discovery_empty_directory() {
    let temp_dir = TempDir::new().unwrap();

    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    assert_eq!(files.len(), 0);
}

#[tokio::test]
async fn test_file_discovery_nonexistent_directory() {
    let discovery = FileDiscovery::new();
    let result = discovery
        .discover_files(Path::new("/nonexistent/path"))
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ValidationError::Io(_) => {} // Expected
        e => panic!("Expected IO error, got: {:?}", e),
    }
}

#[tokio::test]
#[ignore]
async fn test_file_discovery_symlinks() {
    let temp_dir = TempDir::new().unwrap();

    // Create a file and a symlink to it
    let original_file = temp_dir.path().join("original.xml");
    fs::write(&original_file, "<xml/>").await.unwrap();

    let symlink_file = temp_dir.path().join("symlink.xml");

    // Create symlink (skip test if symlinks not supported)
    if tokio::fs::symlink(&original_file, &symlink_file)
        .await
        .is_ok()
    {
        let discovery = FileDiscovery::new();
        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should find both files (original and symlink)
        assert_eq!(files.len(), 2);

        let file_names: std::collections::HashSet<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_str().unwrap())
            .collect();

        assert!(file_names.contains("original.xml"));
        assert!(file_names.contains("symlink.xml"));
    }
}

#[tokio::test]
async fn test_file_discovery_large_directory() {
    let temp_dir = TempDir::new().unwrap();

    // Create many files
    let file_count = 1000;
    for i in 0..file_count {
        let file_path = temp_dir.path().join(format!("file_{:04}.xml", i));
        fs::write(&file_path, format!("<xml>{}</xml>", i))
            .await
            .unwrap();
    }

    let discovery = FileDiscovery::new();
    let start_time = std::time::Instant::now();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();
    let elapsed = start_time.elapsed();

    assert_eq!(files.len(), file_count);

    // Should be reasonably fast (less than 1 second for 1000 files)
    assert!(
        elapsed.as_secs() < 1,
        "File discovery took too long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_file_discovery_concurrent() {
    // Concurrent discovery test - FileDiscovery doesn't implement Send
    // Concurrency is tested through integration tests
    let temp_dir = create_temp_xml_files().await.unwrap();

    let discovery = FileDiscovery::new();

    // Sequential discovery operations (can be made concurrent with refactoring)
    for _i in 0..3 {
        let files = discovery.discover_files(temp_dir.path()).await.unwrap();
        assert_eq!(files.len(), 4);
    }
}

#[tokio::test]
async fn test_file_discovery_pattern_validation() {
    // Test invalid glob patterns
    let result = FileDiscovery::new().with_include_patterns(vec!["[invalid".to_string()]);

    assert!(result.is_err());

    let result = FileDiscovery::new().with_exclude_patterns(vec!["[invalid".to_string()]);

    assert!(result.is_err());

    // Test valid patterns
    let result = FileDiscovery::new()
        .with_include_patterns(vec!["**/*.xml".to_string(), "src/**".to_string()]);

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_file_discovery_case_sensitivity() {
    let temp_dir = TempDir::new().unwrap();

    // Create files with different cases
    fs::write(temp_dir.path().join("test.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("test.XML"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join("TEST.xml"), "<xml/>")
        .await
        .unwrap();

    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // On case-sensitive filesystems, should find 3 files
    // On case-insensitive filesystems, might find fewer
    assert!(files.len() >= 1 && files.len() <= 3);

    // All found files should have xml extension (case-insensitive)
    for file in &files {
        let ext = file.extension().unwrap().to_str().unwrap().to_lowercase();
        assert_eq!(ext, "xml");
    }
}

#[tokio::test]
#[ignore]
async fn test_file_discovery_hidden_files() {
    let temp_dir = TempDir::new().unwrap();

    // Create regular and hidden files
    fs::write(temp_dir.path().join("regular.xml"), "<xml/>")
        .await
        .unwrap();
    fs::write(temp_dir.path().join(".hidden.xml"), "<xml/>")
        .await
        .unwrap();

    let discovery = FileDiscovery::new();
    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // By default, should include hidden files
    assert_eq!(files.len(), 2);

    let file_names: std::collections::HashSet<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_str().unwrap())
        .collect();

    assert!(file_names.contains("regular.xml"));
    assert!(file_names.contains(".hidden.xml"));

    // Test excluding hidden files
    let discovery = FileDiscovery::new()
        .with_exclude_patterns(vec![".*".to_string()])
        .unwrap();

    let files = discovery.discover_files(temp_dir.path()).await.unwrap();

    // Should only find regular file
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name().unwrap(), "regular.xml");
}
