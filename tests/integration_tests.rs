use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_validate-xml"))
}

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

    let output = command()
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
    let output = command()
        .arg("/nonexistent/path/that/really/should/not/exist")
        .output()
        .expect("Failed to run validation");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Path does not exist"));
}

#[test]
fn test_cli_help() {
    let output = command()
        .arg("--help")
        .output()
        .expect("Failed to run validation");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
}

#[test]
fn test_fail_fast_preserves_invalid_exit_code() {
    let temp_dir = TempDir::new().unwrap();
    let schema_path = temp_dir.path().join("schema.xsd");
    fs::write(
        &schema_path,
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:element name="expected" type="xs:string"/>
</xs:schema>"#,
    )
    .unwrap();
    let xml_path = temp_dir.path().join("invalid.xml");
    fs::write(&xml_path, "<unexpected/>").unwrap();

    let output = command()
        .args(["--fail-fast", "--schema"])
        .arg(schema_path)
        .arg(xml_path)
        .output()
        .expect("Failed to run validation");

    assert_eq!(output.status.code(), Some(3));
}
