use divan::Bencher;
use std::sync::Arc;
use validate_xml::LibXml2Wrapper;

fn main() {
    divan::main();
}

const SIMPLE_XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="element" type="xs:string"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

const VALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <element>Valid content</element>
</root>"#;

const INVALID_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
    <invalid>Content</invalid>
</root>"#;

#[divan::bench]
fn parse_schema(bencher: Bencher) {
    let wrapper = LibXml2Wrapper::new();
    let schema_data = SIMPLE_XSD.as_bytes();

    bencher.bench_local(move || {
        wrapper
            .parse_schema_from_memory(schema_data)
            .expect("Failed to parse schema")
    });
}

#[divan::bench]
fn validate_valid_file(bencher: Bencher) {
    let wrapper = LibXml2Wrapper::new();
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = Arc::new(wrapper.parse_schema_from_memory(schema_data).unwrap());

    // Write to a temp file because validation API requires a file path
    use std::io::Write;
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", VALID_XML).unwrap();
    let path = file.path().to_path_buf();

    bencher.bench_local(move || {
        wrapper
            .validate_file(&schema, &path)
            .expect("Validation failed")
    });
}

#[divan::bench]
fn validate_invalid_file(bencher: Bencher) {
    let wrapper = LibXml2Wrapper::new();
    let schema_data = SIMPLE_XSD.as_bytes();
    let schema = Arc::new(wrapper.parse_schema_from_memory(schema_data).unwrap());

    use std::io::Write;
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", INVALID_XML).unwrap();
    let path = file.path().to_path_buf();

    bencher.bench_local(move || {
        wrapper
            .validate_file(&schema, &path)
            .expect("Validation failed")
    });
}
