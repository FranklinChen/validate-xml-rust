use divan::Bencher;
use std::sync::Arc;
use xmloxide::tree::Document;
use xmloxide::validation::xsd::{parse_xsd, validate_xsd};

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
fn bench_parse_schema(bencher: Bencher) {
    bencher.bench_local(|| parse_xsd(SIMPLE_XSD).expect("Failed to parse schema"));
}

#[divan::bench]
fn bench_validate_valid_file(bencher: Bencher) {
    let schema = Arc::new(parse_xsd(SIMPLE_XSD).unwrap());

    use std::io::Write;
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", VALID_XML).unwrap();
    let path = file.path().to_path_buf();

    bencher.bench_local(move || {
        let doc = Document::parse_file(&path).expect("Failed to parse XML");
        validate_xsd(&doc, &schema)
    });
}

#[divan::bench]
fn bench_validate_invalid_file(bencher: Bencher) {
    let schema = Arc::new(parse_xsd(SIMPLE_XSD).unwrap());

    use std::io::Write;
    use tempfile::NamedTempFile;
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", INVALID_XML).unwrap();
    let path = file.path().to_path_buf();

    bencher.bench_local(move || {
        let doc = Document::parse_file(&path).expect("Failed to parse XML");
        validate_xsd(&doc, &schema)
    });
}
