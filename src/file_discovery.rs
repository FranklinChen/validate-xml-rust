use crate::error::{Result, ValidationError};
use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Async file discovery engine that replaces ignore::Walk with async alternatives
#[derive(Debug, Clone)]
pub struct FileDiscovery {
    /// File extensions to include (e.g., ["xml", "xsd"])
    extensions: Vec<String>,
    /// Include patterns (glob-style patterns)
    include_patterns: Vec<Regex>,
    /// Exclude patterns (glob-style patterns)
    exclude_patterns: Vec<Regex>,
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
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            max_depth: None,
            follow_symlinks: false,
        }
    }

    /// Set file extensions to discover
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Add include patterns (converted from glob to regex)
    pub fn with_include_patterns(mut self, patterns: Vec<String>) -> Result<Self> {
        self.include_patterns = patterns
            .into_iter()
            .map(|pattern| glob_to_regex(&pattern))
            .collect::<Result<Vec<_>>>()?;
        Ok(self)
    }

    /// Add exclude patterns (converted from glob to regex)
    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Result<Self> {
        self.exclude_patterns = patterns
            .into_iter()
            .map(|pattern| glob_to_regex(&pattern))
            .collect::<Result<Vec<_>>>()?;
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

    /// Discover files asynchronously in the given directory
    pub async fn discover_files(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        // Start with depth -1 so that files in root directory are at depth 0
        let mut read_dir = fs::read_dir(root).await.map_err(ValidationError::Io)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(ValidationError::Io)? {
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

            let metadata = fs::metadata(path).await.map_err(ValidationError::Io)?;

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

                let mut read_dir = fs::read_dir(path).await.map_err(ValidationError::Io)?;

                while let Some(entry) = read_dir.next_entry().await.map_err(ValidationError::Io)? {
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

        let path_str = path.to_string_lossy();

        // Check exclude patterns first
        for exclude_pattern in &self.exclude_patterns {
            if exclude_pattern.is_match(&path_str) {
                return false;
            }
        }

        // Check include patterns (if any are specified, at least one must match)
        if !self.include_patterns.is_empty() {
            for include_pattern in &self.include_patterns {
                if include_pattern.is_match(&path_str) {
                    return true;
                }
            }
            return false;
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

/// Convert glob pattern to regex
fn glob_to_regex(pattern: &str) -> Result<Regex> {
    let mut regex_pattern = String::new();
    let mut chars = pattern.chars().peekable();

    regex_pattern.push('^');

    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume /
                        regex_pattern.push_str("(?:.*/)?");
                    } else {
                        regex_pattern.push_str(".*");
                    }
                } else {
                    regex_pattern.push_str("[^/]*");
                }
            }
            '?' => regex_pattern.push_str("[^/]"),
            '[' => {
                regex_pattern.push('[');
                while let Some(ch) = chars.next() {
                    if ch == ']' {
                        regex_pattern.push(']');
                        break;
                    }
                    if ch == '\\' {
                        regex_pattern.push('\\');
                        if let Some(escaped) = chars.next() {
                            regex_pattern.push(escaped);
                        }
                    } else {
                        regex_pattern.push(ch);
                    }
                }
            }
            '\\' => {
                regex_pattern.push('\\');
                if let Some(escaped) = chars.next() {
                    regex_pattern.push(escaped);
                }
            }
            '.' | '^' | '$' | '(' | ')' | '{' | '}' | '+' | '|' => {
                regex_pattern.push('\\');
                regex_pattern.push(ch);
            }
            _ => regex_pattern.push(ch),
        }
    }

    regex_pattern.push('$');

    Regex::new(&regex_pattern)
        .map_err(|e| ValidationError::Config(format!("Invalid glob pattern '{}': {}", pattern, e)))
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

    #[test]
    fn test_glob_to_regex() {
        // Test basic patterns
        let regex = glob_to_regex("*.xml").unwrap();
        assert!(regex.is_match("test.xml"));
        assert!(!regex.is_match("test.txt"));
        assert!(!regex.is_match("dir/test.xml")); // * doesn't match /

        // Test recursive patterns
        let regex = glob_to_regex("**/*.xml").unwrap();
        assert!(regex.is_match("test.xml"));
        assert!(regex.is_match("dir/test.xml"));
        assert!(regex.is_match("dir/subdir/test.xml"));

        // Test question mark
        let regex = glob_to_regex("test?.xml").unwrap();
        assert!(regex.is_match("test1.xml"));
        assert!(regex.is_match("testa.xml"));
        assert!(!regex.is_match("test12.xml"));

        // Test character classes
        let regex = glob_to_regex("test[0-9].xml").unwrap();
        assert!(regex.is_match("test1.xml"));
        assert!(regex.is_match("test9.xml"));
        assert!(!regex.is_match("testa.xml"));
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
