use crate::error::{Result, ValidationError};
use globset::{GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Async file discovery engine that replaces ignore::Walk with async alternatives
#[derive(Debug, Clone)]
pub struct FileDiscovery {
    /// File extensions to include (e.g., ["xml", "xsd"])
    extensions: Vec<String>,
    /// Include patterns set
    include_set: Option<GlobSet>,
    /// Exclude patterns set
    exclude_set: Option<GlobSet>,
    /// Maximum depth for directory traversal (None = unlimited)
    max_depth: Option<usize>,
    /// Follow symbolic links
    follow_symlinks: bool,
}

impl FileDiscovery {
    /// Create a new FileDiscovery instance
    pub fn new() -> Self {
        Self {
            extensions: vec!["xml".to_string()],
            include_set: None,
            exclude_set: None,
            max_depth: None,
            follow_symlinks: false,
        }
    }

    /// Set file extensions to discover
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Add include patterns
    pub fn with_include_patterns(mut self, patterns: Vec<String>) -> Result<Self> {
        if patterns.is_empty() {
            self.include_set = None;
            return Ok(self);
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = globset::GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| {
                    ValidationError::Config(format!("Invalid glob pattern '{}': {}", pattern, e))
                })?;
            builder.add(glob);
        }

        self.include_set = Some(builder.build().map_err(|e| {
            ValidationError::Config(format!("Failed to build include glob set: {}", e))
        })?);
        Ok(self)
    }

    /// Add exclude patterns
    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Result<Self> {
        if patterns.is_empty() {
            self.exclude_set = None;
            return Ok(self);
        }

        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = globset::GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| {
                    ValidationError::Config(format!("Invalid glob pattern '{}': {}", pattern, e))
                })?;
            builder.add(glob);
        }

        self.exclude_set = Some(builder.build().map_err(|e| {
            ValidationError::Config(format!("Failed to build exclude glob set: {}", e))
        })?);
        Ok(self)
    }

    /// Set maximum traversal depth
    pub fn with_max_depth(mut self, depth: Option<usize>) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set whether to follow symbolic links
    pub fn with_follow_symlinks(mut self, follow: bool) -> Self {
        self.follow_symlinks = follow;
        self
    }

    /// Discover files asynchronously in the given path (file or directory)
    pub async fn discover_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let metadata = fs::metadata(path).await.map_err(ValidationError::from)?;

        if metadata.is_file() {
            // If it's a file, just return it if it matches patterns
            // Note: we check should_process but relax extension check if it's explicitly provided?
            // Actually, keep should_process strict.
            if self.should_process(path) {
                return Ok(vec![path.to_path_buf()]);
            } else {
                return Ok(Vec::new());
            }
        }

        let mut files = Vec::new();

        // Start with depth -1 so that files in root directory are at depth 0
        let mut read_dir = fs::read_dir(path).await.map_err(ValidationError::from)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(ValidationError::from)? {
            let entry_path = entry.path();

            // Handle symlinks
            if entry_path.is_symlink() && !self.follow_symlinks {
                continue;
            }

            // Process each entry at depth 0
            if let Err(e) = self
                .discover_files_recursive(&entry_path, 0, &mut files)
                .await
            {
                // Log error but continue processing other files
                eprintln!("Warning: Error processing {}: {}", entry_path.display(), e);
            }
        }

        Ok(files)
    }

    /// Recursive helper for discovering files
    fn discover_files_recursive<'a>(
        &'a self,
        path: &'a Path,
        depth: usize,
        files: &'a mut Vec<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            // Check depth limit - allow processing at current depth, but don't go deeper
            if let Some(max_depth) = self.max_depth
                && depth > max_depth
            {
                return Ok(());
            }

            let metadata = fs::metadata(path).await.map_err(ValidationError::from)?;

            if metadata.is_file() {
                if self.should_process(path) {
                    files.push(path.to_path_buf());
                }
            } else if metadata.is_dir() {
                // Only recurse into directories if we can still go deeper
                if let Some(max_depth) = self.max_depth
                    && depth >= max_depth
                {
                    return Ok(());
                }

                let mut read_dir = fs::read_dir(path).await.map_err(ValidationError::from)?;

                while let Some(entry) =
                    read_dir.next_entry().await.map_err(ValidationError::from)?
                {
                    let entry_path = entry.path();

                    // Handle symlinks
                    if entry_path.is_symlink() && !self.follow_symlinks {
                        continue;
                    }

                    // Recursively process subdirectories and files
                    if let Err(e) = self
                        .discover_files_recursive(&entry_path, depth + 1, files)
                        .await
                    {
                        // Log error but continue processing other files
                        eprintln!("Warning: Error processing {}: {}", entry_path.display(), e);
                    }
                }
            }

            Ok(())
        })
    }

    /// Check if a file should be processed based on extensions and patterns
    pub fn should_process(&self, path: &Path) -> bool {
        // Check extension
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            if !self.extensions.contains(&extension.to_lowercase()) {
                return false;
            }
        } else {
            // No extension - only process if "xml" is in extensions and no extension is acceptable
            return false;
        }

        // Check exclude patterns first
        if let Some(exclude_set) = &self.exclude_set
            && exclude_set.is_match(path) {
                return false;
            }

        // Check include patterns (if any are specified, at least one must match)
        if let Some(include_set) = &self.include_set {
            return include_set.is_match(path);
        }

        true
    }

    /// Get statistics about discovered files
    pub async fn get_discovery_stats(&self, root: &Path) -> Result<DiscoveryStats> {
        let files = self.discover_files(root).await?;
        Ok(DiscoveryStats {
            files_found: files.len(),
            errors: 0,
        })
    }
}

impl Default for FileDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about file discovery operation
#[derive(Debug, Default, Clone)]
pub struct DiscoveryStats {
    pub files_found: usize,
    pub errors: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;
    use tokio::fs;

    async fn create_test_directory() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create test directory structure
        fs::create_dir_all(root.join("subdir1")).await.unwrap();
        fs::create_dir_all(root.join("subdir2/nested"))
            .await
            .unwrap();

        // Create test files
        fs::write(root.join("file1.xml"), "<?xml version=\"1.0\"?>")
            .await
            .unwrap();
        fs::write(root.join("file2.xml"), "<?xml version=\"1.0\"?>")
            .await
            .unwrap();
        fs::write(root.join("file3.txt"), "text file")
            .await
            .unwrap();
        fs::write(root.join("subdir1/nested.xml"), "<?xml version=\"1.0\"?>")
            .await
            .unwrap();
        fs::write(
            root.join("subdir2/nested/deep.xml"),
            "<?xml version=\"1.0\"?>",
        )
        .await
        .unwrap();
        fs::write(root.join("subdir2/nested/other.xsd"), "schema")
            .await
            .unwrap();

        temp_dir
    }

    #[tokio::test]
    async fn test_discover_xml_files() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new();

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should find 4 XML files
        assert_eq!(files.len(), 4);

        let file_names: HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(file_names.contains("file1.xml"));
        assert!(file_names.contains("file2.xml"));
        assert!(file_names.contains("nested.xml"));
        assert!(file_names.contains("deep.xml"));
    }

    #[tokio::test]
    async fn test_discover_multiple_extensions() {
        let temp_dir = create_test_directory().await;
        let discovery =
            FileDiscovery::new().with_extensions(vec!["xml".to_string(), "xsd".to_string()]);

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should find 5 files (4 XML + 1 XSD)
        assert_eq!(files.len(), 5);
    }

    #[tokio::test]
    async fn test_max_depth_limit() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new().with_max_depth(Some(1));

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should find 3 files (2 in root + 1 in subdir1, but not the deep nested one)
        // Root (depth 0): file1.xml, file2.xml
        // Depth 1: subdir1/nested.xml
        // Depth 2: subdir2/nested/deep.xml (excluded by max_depth=1)
        assert_eq!(files.len(), 3);

        let file_names: HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(file_names.contains("file1.xml"));
        assert!(file_names.contains("file2.xml"));
        assert!(file_names.contains("nested.xml"));
        assert!(!file_names.contains("deep.xml")); // Too deep
    }

    #[tokio::test]
    async fn test_include_patterns() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new()
            .with_include_patterns(vec!["**/nested*".to_string()])
            .unwrap();

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should only find files matching the pattern
        // This should match: subdir1/nested.xml and subdir2/nested/deep.xml
        // But deep.xml doesn't have "nested" in its name, so only nested.xml should match
        assert_eq!(files.len(), 1); // Only nested.xml
    }

    #[tokio::test]
    async fn test_exclude_patterns() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new()
            .with_exclude_patterns(vec!["**/subdir2/**".to_string()])
            .unwrap();

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        // Should exclude files in subdir2
        assert_eq!(files.len(), 3); // All except deep.xml

        let file_names: HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(!file_names.contains("deep.xml"));
    }

    #[tokio::test]
    async fn test_should_process() {
        let discovery = FileDiscovery::new();

        assert!(discovery.should_process(Path::new("test.xml")));
        assert!(!discovery.should_process(Path::new("test.txt")));
        assert!(!discovery.should_process(Path::new("test"))); // No extension
    }

    #[tokio::test]
    async fn test_discovery_stats() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new();

        let stats = discovery
            .get_discovery_stats(temp_dir.path())
            .await
            .unwrap();

        assert_eq!(stats.files_found, 4); // 4 XML files
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn test_discover_files_recursive() {
        let temp_dir = create_test_directory().await;
        let discovery = FileDiscovery::new();

        let files = discovery.discover_files(temp_dir.path()).await.unwrap();

        assert_eq!(files.len(), 4); // 4 XML files

        let file_names: std::collections::HashSet<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(file_names.contains("file1.xml"));
        assert!(file_names.contains("file2.xml"));
        assert!(file_names.contains("nested.xml"));
        assert!(file_names.contains("deep.xml"));
    }

    #[tokio::test]
    async fn test_nonexistent_directory() {
        let discovery = FileDiscovery::new();
        let result = discovery
            .discover_files(Path::new("/nonexistent/path"))
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::Io(_) => {} // Expected
            _ => panic!("Expected IO error"),
        }
    }
}
