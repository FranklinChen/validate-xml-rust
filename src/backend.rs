//! Hardened libxml2 validation and the compiled-schema state boundary.

use std::collections::HashSet;
use std::path::Path;

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;

use crate::error::Result;

/// A schema that has crossed the parse/compile boundary successfully.
#[derive(Debug)]
pub struct CompiledSchema(libxml2::Schema);

#[derive(Debug)]
pub enum SchemaValidation {
    Valid,
    Invalid(Vec<String>),
}

pub fn compile(data: &[u8], source: &str, local_path: Option<&Path>) -> Result<CompiledSchema> {
    enforce_resource_policy(data, source, local_path)?;
    libxml2::compile(data, source, local_path).map(CompiledSchema)
}

pub fn validate(schema: &CompiledSchema, file: &Path) -> Result<SchemaValidation> {
    libxml2::validate(&schema.0, file)
}

/// Keep schema composition inside the application's resource policy. Local
/// schemas may use relative filesystem references. Remote schemas are cached
/// as bytes and may not ask libxml2 to perform secondary I/O.
pub(crate) fn enforce_resource_policy(
    data: &[u8],
    source: &str,
    local_path: Option<&Path>,
) -> Result<()> {
    let locations = composition_locations(data, source)?;
    let Some(local_path) = local_path else {
        if let Some(location) = locations.first() {
            return Err(crate::error::ValidationError::SchemaParsing {
                url: source.to_owned(),
                details: format!(
                    "remote schema composition is disabled; nested resource {location:?} must be supplied explicitly"
                ),
            });
        }
        return Ok(());
    };

    inspect_local_dependencies(locations, local_path).map(|_| ())
}

pub(crate) fn local_dependency_paths(
    data: &[u8],
    local_path: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    let locations = composition_locations(data, &local_path.display().to_string())?;
    inspect_local_dependencies(locations, local_path)
}

fn composition_locations(data: &[u8], source: &str) -> Result<Vec<String>> {
    const XSD_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
    let mut reader = NsReader::from_reader(data);
    let mut buffer = Vec::new();
    let mut locations = Vec::new();

    loop {
        buffer.clear();
        let (namespace, event) = match reader.read_resolved_event_into(&mut buffer) {
            Ok(value) => value,
            Err(error) => {
                return Err(crate::error::ValidationError::SchemaParsing {
                    url: source.to_owned(),
                    details: format!(
                        "XSD resource-policy parse error at byte {}: {error}",
                        reader.buffer_position()
                    ),
                });
            }
        };
        let start = match event {
            Event::Start(start) | Event::Empty(start) => start,
            Event::Eof => return Ok(locations),
            _ => continue,
        };
        for attribute in start.attributes().with_checks(false) {
            let attribute =
                attribute.map_err(|error| crate::error::ValidationError::SchemaParsing {
                    url: source.to_owned(),
                    details: format!("malformed XSD attribute: {error}"),
                })?;
            if attribute.key.as_ref() == "xml:base" {
                return Err(crate::error::ValidationError::SchemaParsing {
                    url: source.to_owned(),
                    details: "xml:base is disabled in schemas because it can redirect nested resource loading outside the local-only policy".to_owned(),
                });
            }
        }
        let is_xsd = matches!(
            namespace,
            ResolveResult::Bound(Namespace(uri)) if uri == XSD_NAMESPACE
        );
        if !is_xsd
            || !matches!(
                start.local_name().as_ref(),
                "include" | "import" | "redefine" | "override"
            )
        {
            continue;
        }

        for attribute in start.attributes().with_checks(false) {
            let attribute =
                attribute.map_err(|error| crate::error::ValidationError::SchemaParsing {
                    url: source.to_owned(),
                    details: format!("malformed XSD composition attribute: {error}"),
                })?;
            if attribute.key.local_name().as_ref() != "schemaLocation" {
                continue;
            }
            let location = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| crate::error::ValidationError::SchemaParsing {
                    url: source.to_owned(),
                    details: format!("invalid schemaLocation: {error}"),
                })?;
            locations.push(location.into_owned());
        }
    }
}

fn inspect_local_dependencies(
    locations: Vec<String>,
    parent_schema: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    let mut visited = HashSet::new();
    if let Ok(canonical) = std::fs::canonicalize(parent_schema) {
        visited.insert(canonical);
    }
    let mut dependencies = Vec::new();
    inspect_local_dependencies_inner(locations, parent_schema, &mut visited, &mut dependencies)?;
    Ok(dependencies)
}

fn inspect_local_dependencies_inner(
    locations: Vec<String>,
    parent_schema: &Path,
    visited: &mut HashSet<std::path::PathBuf>,
    dependencies: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for location in locations {
        if is_uri_reference(&location) || location.contains(['?', '#']) {
            return Err(crate::error::ValidationError::SchemaParsing {
                url: parent_schema.display().to_string(),
                details: format!(
                    "schema composition may only use local paths; refusing {location:?}"
                ),
            });
        }
        let dependency = parent_schema
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&location);
        let canonical = std::fs::canonicalize(&dependency).map_err(|error| {
            crate::error::ValidationError::SchemaParsing {
                url: parent_schema.display().to_string(),
                details: format!(
                    "could not resolve nested schema {}: {error}",
                    dependency.display()
                ),
            }
        })?;
        if !visited.insert(canonical.clone()) {
            continue;
        }
        dependencies.push(canonical.clone());
        let data = std::fs::read(&canonical).map_err(|error| {
            crate::error::ValidationError::SchemaParsing {
                url: canonical.display().to_string(),
                details: format!("could not read nested schema: {error}"),
            }
        })?;
        let nested = composition_locations(&data, &canonical.display().to_string())?;
        inspect_local_dependencies_inner(nested, &canonical, visited, dependencies)?;
    }
    Ok(())
}

fn is_uri_reference(location: &str) -> bool {
    if location.starts_with("//") || location.starts_with("\\\\") {
        return true;
    }
    let Some((scheme, remainder)) = location.split_once(':') else {
        return false;
    };
    let windows_drive = scheme.len() == 1
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && (remainder.starts_with('/') || remainder.starts_with('\\'));
    !windows_drive && url::Url::parse(location).is_ok()
}

mod libxml2 {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::ptr::NonNull;
    use std::sync::{Mutex, Once};

    use super::SchemaValidation;
    use crate::error::{Result, ValidationError};

    static INIT: Once = Once::new();
    static SCHEMA_PARSE_LOCK: Mutex<()> = Mutex::new(());

    #[repr(C)]
    struct XmlSchema {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct XmlSchemaParserCtxt {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct XmlSchemaValidCtxt {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct XmlDoc {
        _private: [u8; 0],
    }
    #[repr(C)]
    struct XmlError {
        domain: c_int,
        code: c_int,
        message: *const c_char,
        level: c_int,
        file: *const c_char,
        line: c_int,
        str1: *const c_char,
        str2: *const c_char,
        str3: *const c_char,
        int1: c_int,
        int2: c_int,
        ctxt: *mut c_void,
        node: *mut c_void,
    }

    type StructuredError = Option<unsafe extern "C" fn(*mut c_void, *mut XmlError)>;

    #[cfg_attr(target_os = "windows", link(name = "libxml2"))]
    #[cfg_attr(not(target_os = "windows"), link(name = "xml2"))]
    unsafe extern "C" {
        fn xmlInitParser();
        fn xmlSchemaNewMemParserCtxt(
            buffer: *const c_char,
            size: c_int,
        ) -> *mut XmlSchemaParserCtxt;
        fn xmlSchemaNewDocParserCtxt(document: *mut XmlDoc) -> *mut XmlSchemaParserCtxt;
        fn xmlSchemaParse(context: *mut XmlSchemaParserCtxt) -> *mut XmlSchema;
        fn xmlSchemaFreeParserCtxt(context: *mut XmlSchemaParserCtxt);
        fn xmlSchemaSetParserStructuredErrors(
            context: *mut XmlSchemaParserCtxt,
            callback: StructuredError,
            user_data: *mut c_void,
        );
        fn xmlSchemaFree(schema: *mut XmlSchema);
        fn xmlSchemaNewValidCtxt(schema: *const XmlSchema) -> *mut XmlSchemaValidCtxt;
        fn xmlSchemaFreeValidCtxt(context: *mut XmlSchemaValidCtxt);
        fn xmlSchemaSetValidStructuredErrors(
            context: *mut XmlSchemaValidCtxt,
            callback: StructuredError,
            user_data: *mut c_void,
        );
        fn xmlReadFile(
            file_name: *const c_char,
            encoding: *const c_char,
            options: c_int,
        ) -> *mut XmlDoc;
        fn xmlReadMemory(
            buffer: *const c_char,
            size: c_int,
            url: *const c_char,
            encoding: *const c_char,
            options: c_int,
        ) -> *mut XmlDoc;
        fn xmlFreeDoc(document: *mut XmlDoc);
        fn xmlResetLastError();
        fn xmlGetLastError() -> *mut XmlError;
        fn xmlSchemaValidateDoc(context: *mut XmlSchemaValidCtxt, document: *mut XmlDoc) -> c_int;
    }

    pub struct Schema {
        raw: NonNull<XmlSchema>,
        // xmlSchemaNewDocParserCtxt leaves caller-provided documents owned by
        // the application. Retain that tree for the compiled schema's full
        // lifetime because schema components may retain node pointers.
        _source_document: Option<Document>,
    }

    impl std::fmt::Debug for Schema {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_tuple("Libxml2Schema")
                .field(&self.raw)
                .finish()
        }
    }

    // SAFETY: after parsing, libxml2 schemas are immutable and may be shared.
    // Every validation call below creates and owns a distinct validation context.
    unsafe impl Send for Schema {}
    unsafe impl Sync for Schema {}

    impl Drop for Schema {
        fn drop(&mut self) {
            // Free the compiled schema before Rust drops _source_document.
            unsafe { xmlSchemaFree(self.raw.as_ptr()) };
        }
    }

    struct ParserContext(NonNull<XmlSchemaParserCtxt>);
    impl Drop for ParserContext {
        fn drop(&mut self) {
            unsafe { xmlSchemaFreeParserCtxt(self.0.as_ptr()) };
        }
    }
    struct ValidationContext(NonNull<XmlSchemaValidCtxt>);
    impl Drop for ValidationContext {
        fn drop(&mut self) {
            unsafe { xmlSchemaFreeValidCtxt(self.0.as_ptr()) };
        }
    }
    struct Document(NonNull<XmlDoc>);
    impl Drop for Document {
        fn drop(&mut self) {
            unsafe { xmlFreeDoc(self.0.as_ptr()) };
        }
    }

    pub fn compile(
        data: &[u8],
        source: &str,
        local_path: Option<&std::path::Path>,
    ) -> Result<Schema> {
        INIT.call_once(|| unsafe { xmlInitParser() });
        let size =
            c_int::try_from(data.len()).map_err(|_| ValidationError::ResourceExhaustion {
                resource: "schema buffer".into(),
                details: "libxml2 accepts at most c_int::MAX bytes".into(),
            })?;
        let _guard = SCHEMA_PARSE_LOCK
            .lock()
            .map_err(|_| ValidationError::Concurrency {
                details: "libxml2 schema-parse lock was poisoned".into(),
            })?;

        // A document parsed with its local path as URL gives libxml2 the base
        // URI it needs for relative xs:include/xs:import resolution. Remote
        // schemas intentionally stay memory-only: giving libxml2 an HTTP base
        // URI would let nested resources bypass AsyncHttpClient's network policy.
        const XML_PARSE_NOERROR: c_int = 1 << 5;
        const XML_PARSE_NOWARNING: c_int = 1 << 6;
        const XML_PARSE_NONET: c_int = 1 << 11;
        let document = if let Some(path) = local_path {
            let absolute_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
            let base_url = url::Url::from_file_path(&absolute_path).map_err(|()| {
                ValidationError::SchemaParsing {
                    url: source.into(),
                    details: format!(
                        "schema path cannot be represented as a file URL: {}",
                        absolute_path.display()
                    ),
                }
            })?;
            let url =
                CString::new(base_url.as_str()).map_err(|_| ValidationError::SchemaParsing {
                    url: source.into(),
                    details: "schema file URL contains a NUL byte".into(),
                })?;
            unsafe { xmlResetLastError() };
            Some(Document(
                NonNull::new(unsafe {
                    xmlReadMemory(
                        data.as_ptr().cast(),
                        size,
                        url.as_ptr(),
                        std::ptr::null(),
                        XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET,
                    )
                })
                .ok_or_else(|| ValidationError::SchemaParsing {
                    url: source.into(),
                    details: last_error_details("libxml2 could not parse the schema document"),
                })?,
            ))
        } else {
            None
        };

        // Declared before the parser context so its callback target remains
        // alive through context teardown.
        let mut errors = Vec::new();
        let raw_context = match &document {
            Some(document) => unsafe { xmlSchemaNewDocParserCtxt(document.0.as_ptr()) },
            None => unsafe { xmlSchemaNewMemParserCtxt(data.as_ptr().cast(), size) },
        };
        let context = NonNull::new(raw_context)
            .map(ParserContext)
            .ok_or_else(|| ValidationError::SchemaParsing {
                url: source.into(),
                details: "libxml2 could not allocate a parser context".into(),
            })?;
        unsafe {
            xmlSchemaSetParserStructuredErrors(
                context.0.as_ptr(),
                Some(collect_error),
                (&raw mut errors).cast(),
            );
        }
        let raw = NonNull::new(unsafe { xmlSchemaParse(context.0.as_ptr()) }).ok_or_else(|| {
            ValidationError::SchemaParsing {
                url: source.into(),
                details: diagnostic_details(&errors, "libxml2 rejected the schema"),
            }
        })?;
        Ok(Schema {
            raw,
            _source_document: document,
        })
    }

    unsafe extern "C" fn collect_error(user_data: *mut c_void, error: *mut XmlError) {
        if user_data.is_null() || error.is_null() {
            return;
        }
        if let Some(formatted) = format_error(unsafe { &*error }) {
            unsafe { &mut *user_data.cast::<Vec<String>>() }.push(formatted);
        }
    }

    fn format_error(error: &XmlError) -> Option<String> {
        if error.message.is_null() {
            return None;
        }
        let message = unsafe { CStr::from_ptr(error.message) }.to_string_lossy();
        let message = message.trim();
        if message.is_empty() {
            return None;
        }
        let file = (!error.file.is_null())
            .then(|| unsafe { CStr::from_ptr(error.file).to_string_lossy().into_owned() });
        let location = match (file, error.line, error.int2) {
            (Some(file), line, column) if line > 0 && column > 0 => {
                format!("{file}:{line}:{column}: ")
            }
            (Some(file), line, _) if line > 0 => format!("{file}:{line}: "),
            (Some(file), _, _) => format!("{file}: "),
            (None, line, column) if line > 0 && column > 0 => {
                format!("line {line}, column {column}: ")
            }
            (None, line, _) if line > 0 => format!("line {line}: "),
            _ => String::new(),
        };
        Some(format!("{location}{message}"))
    }

    fn last_error_details(fallback: &str) -> String {
        let error = unsafe { xmlGetLastError() };
        if error.is_null() {
            fallback.to_owned()
        } else {
            format_error(unsafe { &*error }).unwrap_or_else(|| fallback.to_owned())
        }
    }

    fn diagnostic_details(errors: &[String], fallback: &str) -> String {
        if errors.is_empty() {
            fallback.to_owned()
        } else {
            errors.join("; ")
        }
    }

    pub fn validate(schema: &Schema, file: &std::path::Path) -> Result<SchemaValidation> {
        let path = CString::new(file.as_os_str().as_encoded_bytes()).map_err(|_| {
            ValidationError::XmlParsing {
                file: file.to_owned(),
                details: "path contains a NUL byte".into(),
            }
        })?;
        // XML_PARSE_NONET: never fetch network resources while parsing an
        // input document. Entity substitution is intentionally not enabled.
        const XML_PARSE_NOERROR: c_int = 1 << 5;
        const XML_PARSE_NOWARNING: c_int = 1 << 6;
        const XML_PARSE_NONET: c_int = 1 << 11;
        unsafe { xmlResetLastError() };
        let document = NonNull::new(unsafe {
            xmlReadFile(
                path.as_ptr(),
                std::ptr::null(),
                XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET,
            )
        })
        .map(Document)
        .ok_or_else(|| ValidationError::XmlParsing {
            file: file.to_owned(),
            details: last_error_details("libxml2 could not parse the document"),
        })?;
        // Declared before the context so its callback target remains alive
        // through context teardown.
        let mut errors = Vec::new();
        let context = NonNull::new(unsafe { xmlSchemaNewValidCtxt(schema.raw.as_ptr()) })
            .map(ValidationContext)
            .ok_or_else(|| ValidationError::ResourceExhaustion {
                resource: "libxml2 validation context".into(),
                details: "allocation failed".into(),
            })?;
        unsafe {
            xmlSchemaSetValidStructuredErrors(
                context.0.as_ptr(),
                Some(collect_error),
                (&mut errors as *mut Vec<String>).cast(),
            );
        }
        let code = unsafe { xmlSchemaValidateDoc(context.0.as_ptr(), document.0.as_ptr()) };
        match code {
            0 => Ok(SchemaValidation::Valid),
            value if value > 0 => Ok(SchemaValidation::Invalid(errors)),
            value => Err(ValidationError::ValidationFailed {
                file: file.to_owned(),
                details: format!(
                    "libxml2 internal validation error {value}: {}",
                    diagnostic_details(&errors, "no diagnostic details")
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{SchemaValidation, compile, validate};

    #[test]
    fn local_schema_resolves_relative_include() -> crate::error::Result<()> {
        let directory = TempDir::new()?;
        let included = directory.path().join("types.xsd");
        fs::write(
            &included,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:complexType name="RootType"><xs:sequence>
<xs:element name="value" type="xs:string"/>
</xs:sequence></xs:complexType></xs:schema>"#,
        )?;
        let main = directory.path().join("main.xsd");
        let schema_data = br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:include schemaLocation="types.xsd"/>
<xs:element name="root" type="RootType"/>
</xs:schema>"#;
        fs::write(&main, schema_data)?;
        let document = directory.path().join("document.xml");
        fs::write(&document, "<root><value>ok</value></root>")?;

        let schema = compile(schema_data, &main.display().to_string(), Some(&main))?;
        assert!(matches!(
            validate(&schema, &document)?,
            SchemaValidation::Valid
        ));
        Ok(())
    }

    #[test]
    fn schema_compile_error_contains_libxml_diagnostic() {
        let error = compile(
            br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:element name="root" type="MissingType"/>
</xs:schema>"#,
            "broken.xsd",
            None,
        )
        .unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("MissingType"), "{rendered}");
    }

    #[test]
    fn malformed_document_error_contains_libxml_diagnostic() -> crate::error::Result<()> {
        let directory = TempDir::new()?;
        let schema_data = br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:element name="root" type="xs:string"/>
</xs:schema>"#;
        let schema = compile(schema_data, "schema.xsd", None)?;
        let document = directory.path().join("malformed.xml");
        fs::write(&document, "<root>")?;

        let rendered = validate(&schema, &document).unwrap_err().to_string();
        assert!(rendered.contains("line 1"), "{rendered}");
        assert!(rendered.contains("Premature end"), "{rendered}");
        Ok(())
    }

    #[test]
    fn remote_schema_composition_is_rejected_before_libxml_io() {
        let error = compile(
            br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:include schemaLocation="nested.xsd"/>
</xs:schema>"#,
            "https://example.invalid/main.xsd",
            None,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("remote schema composition is disabled")
        );
    }

    #[test]
    fn nested_local_schema_cannot_escape_to_a_url() -> crate::error::Result<()> {
        let directory = TempDir::new()?;
        let nested = directory.path().join("nested.xsd");
        fs::write(
            &nested,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:include schemaLocation="https://example.invalid/escape.xsd"/>
</xs:schema>"#,
        )?;
        let main = directory.path().join("main.xsd");
        let schema_data = br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xs:include schemaLocation="nested.xsd"/>
</xs:schema>"#;
        fs::write(&main, schema_data)?;

        let error = compile(schema_data, &main.display().to_string(), Some(&main)).unwrap_err();
        assert!(error.to_string().contains("may only use local paths"));
        Ok(())
    }

    #[test]
    fn xml_base_cannot_redirect_local_composition() -> crate::error::Result<()> {
        let directory = TempDir::new()?;
        let main = directory.path().join("main.xsd");
        let schema_data = br#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
xml:base="https://example.invalid/"><xs:include schemaLocation="nested.xsd"/></xs:schema>"#;
        fs::write(&main, schema_data)?;

        let error = compile(schema_data, &main.display().to_string(), Some(&main)).unwrap_err();
        assert!(error.to_string().contains("xml:base is disabled"));
        Ok(())
    }
}
