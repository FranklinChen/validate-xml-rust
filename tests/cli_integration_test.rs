use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_cli_help_output() {
    let output = Command::new("cargo")
        .args(&["run", "--", "--help"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // Check that help contains key elements
    assert!(
        stdout.contains("A high-performance XML validation tool")
            || stdout.contains("High-performance XML validation tool")
    );
    assert!(stdout.contains("EXAMPLES:"));
    assert!(stdout.contains("--extensions"));
    assert!(stdout.contains("--threads"));
    assert!(stdout.contains("--verbose"));
    assert!(stdout.contains("--quiet"));
    assert!(stdout.contains("--config"));
    assert!(stdout.contains("--cache-dir"));
    assert!(stdout.contains("--format"));
}

#[test]
fn test_cli_version_output() {
    let output = Command::new("cargo")
        .args(&["run", "--", "--version"])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("validate-xml 0.2.0"));
}

#[test]
fn test_cli_invalid_directory_error() {
    let output = Command::new("cargo")
        .args(&["run", "--", "/nonexistent/directory/path"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Directory does not exist"));
}

#[test]
fn test_cli_conflicting_options() {
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--verbose",
            "--quiet",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn test_cli_valid_directory_success() {
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new("cargo")
        .args(&["run", "--", "--quiet", temp_dir.path().to_str().unwrap()])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // In quiet mode, we should see no output (except warnings from compilation)
    // The stdout should be empty or only contain compilation warnings
    let lines: Vec<&str> = stdout.lines().collect();
    let non_warning_lines: Vec<&str> = lines
        .iter()
        .filter(|line| !line.contains("warning:") && !line.trim().is_empty())
        .copied()
        .collect();
    assert!(
        non_warning_lines.is_empty(),
        "Expected no output in quiet mode, but got: {:?}",
        non_warning_lines
    );
}

#[test]
fn test_cli_verbose_output() {
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--verbose",
            "--extensions",
            "xml,cmdi",
            "--threads",
            "2",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    // Check verbose output contains configuration details
    assert!(stdout.contains("Configuration:"));
    assert!(stdout.contains("Directory:"));
    assert!(stdout.contains("Extensions: [\"xml\", \"cmdi\"]"));
    assert!(stdout.contains("Threads: 2"));
    assert!(stdout.contains("Cache directory:"));
    assert!(stdout.contains("Cache TTL:"));
}

#[test]
fn test_cli_multiple_extensions_parsing() {
    let temp_dir = TempDir::new().unwrap();

    let output = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--verbose",
            "--extensions",
            "xml,cmdi,xsd,txt",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Extensions: [\"xml\", \"cmdi\", \"xsd\", \"txt\"]"));
}

#[test]
fn test_cli_output_format_options() {
    let temp_dir = TempDir::new().unwrap();

    // Test JSON format
    let output = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--verbose",
            "--format",
            "json",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Output format: Json"));

    // Test summary format
    let output = Command::new("cargo")
        .args(&[
            "run",
            "--",
            "--verbose",
            "--format",
            "summary",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Output format: Summary"));
}
