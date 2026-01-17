use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_cli_basic_validation() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("schema.xsd");
    let xml_path = temp_dir.path().join("valid.xml");

    fs::write(
        &schema_path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root" type="xs:string"/>
</xs:schema>"#,
    )
    .unwrap();

    fs::write(
        &xml_path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="{}">Hello</root>"#,
            schema_path.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();

    let output = Command::new("./target/release/validate-xml")
        .arg(temp_dir.path())
        .output()
        .expect("Failed to run validation");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        println!("STDOUT: {}", stdout);
        println!("STDERR: {}", stderr);
    }

    assert!(output.status.success());
    assert!(stdout.contains("Valid: 1"));
}

#[test]
fn test_cli_invalid_path() {
    let output = Command::new("./target/release/validate-xml")
        .arg("/nonexistent/path/that/really/should/not/exist")
        .output()
        .expect("Failed to run validation");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Path does not exist"));
}

#[test]
fn test_cli_help() {
    let output = Command::new("./target/release/validate-xml")
        .arg("--help")
        .output()
        .expect("Failed to run validation");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
}
