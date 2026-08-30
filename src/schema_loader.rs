use std::path::{Path, PathBuf};
use std::sync::Arc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesPI, BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;

/// Schema-URL extraction stops at the root start tag, so the parser never
/// observes an XML declaration and cannot disambiguate XML 1.0 from 1.1.
/// `Implicit1_0` is the honest "no declaration seen, default to 1.0
/// normalization rules" choice — and matches the behavior of the
/// deprecated `Attribute::unescape_value()` we used to call.
const ATTR_NORMALIZATION_VERSION: XmlVersion = XmlVersion::Implicit1_0;

use crate::cache::{CachedSchema, SchemaCache};
use crate::error::{Result, ValidationError};
use crate::http_client::AsyncHttpClient;

/// The XML Schema Instance namespace URI.
///
/// xsi attributes (`schemaLocation`, `noNamespaceSchemaLocation`, …) are
/// identified by this namespace URI regardless of the prefix the document
/// author chose to bind to it. Matching by URI rather than by literal
/// `xsi:` prefix makes the extractor correct for documents that bind xsi
/// to a non-standard prefix.
const XSI_NAMESPACE_URI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Namespace URI of XML Schema Definition elements themselves, used when
/// probing fetched schema bytes to confirm they are an XSD.
const XSD_NAMESPACE_URI: &str = "http://www.w3.org/2001/XMLSchema";

/// Target of the W3C xml-model processing instruction. See
/// <https://www.w3.org/TR/xml-model/>.
const XML_MODEL_PI_TARGET: &str = "xml-model";

/// A schema source extracted from an XML input — either a resolved local
/// filesystem path or a remote URL.
///
/// This collapses an earlier `SchemaReference { url, source_type }` design
/// that carried two near-duplicate representations of the same location.
/// For local refs the previous design additionally kept the *unresolved*
/// relative reference as `url`, which silently collided as a cache key
/// across XML files in different directories (two files each referencing
/// "schema.xsd" would share an L1 parsed-schema cache entry despite
/// resolving to different files on disk). Cache keys now come from
/// [`Self::cache_key`], which uses the fully resolved path for Local.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaReference {
    /// Filesystem path, already resolved against the XML file's parent
    /// directory if the original reference was relative.
    Local(PathBuf),
    /// Normalized `http://` or `https://` URL. It remains a string at this
    /// boundary because the HTTP client accepts strings and the enum already
    /// records that parsing and scheme validation succeeded.
    Remote(String),
}

/// Why a document points at a schema. Keeping this alongside the location
/// prevents an unrelated namespace pair from being selected merely because it
/// appeared first in `xsi:schemaLocation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaHintKind {
    Namespace(String),
    NoNamespace,
    XmlModel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaHint {
    pub kind: SchemaHintKind,
    pub reference: SchemaReference,
}

/// Schema hints plus the namespace of the document element they describe.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedSchemaHints {
    pub root_namespace: Option<String>,
    pub hints: Vec<SchemaHint>,
}

impl ExtractedSchemaHints {
    /// Select the hint applicable to the document element. An xml-model PI is
    /// a fallback; namespace-aware xsi hints take precedence.
    pub fn applicable(&self) -> Option<&SchemaReference> {
        let matching_kind = match &self.root_namespace {
            Some(namespace) => self.hints.iter().find(|hint| {
                matches!(&hint.kind, SchemaHintKind::Namespace(candidate) if candidate == namespace)
            }),
            None => self
                .hints
                .iter()
                .find(|hint| matches!(hint.kind, SchemaHintKind::NoNamespace)),
        };
        matching_kind
            .or_else(|| {
                self.hints
                    .iter()
                    .find(|hint| matches!(hint.kind, SchemaHintKind::XmlModel))
            })
            .map(|hint| &hint.reference)
    }
}

impl SchemaReference {
    /// Stable location identity. The `local:` prefix partitions local and
    /// remote sources. Local cache freshness additionally requires
    /// [`SchemaLoader::cache_key`], which includes the schema graph digest.
    pub fn cache_key(&self) -> String {
        match self {
            Self::Local(path) => format!("local:{}", path.display()),
            Self::Remote(url) => url.clone(),
        }
    }
}

impl std::fmt::Display for SchemaReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Remote(url) => f.write_str(url),
        }
    }
}

/// Schema extraction engine.
///
/// Uses a streaming XML pull parser (quick-xml) that walks only the prolog
/// and the root element's start tag, stopping immediately afterward. This
/// replaces an earlier regex-and-line-reader implementation that mishandled
/// comment-embedded attribute look-alikes, multi-pair `schemaLocation`
/// values, multi-line attribute wraps, and the `<?xml-model ?>` PI; see
/// GitHub issue #235 for the failure cases.
///
/// Sources considered, in document order:
///
/// * `xsi:schemaLocation="ns1 loc1 ns2 loc2 ..."` on the root element —
///   each pair is retained as a namespace-aware `SchemaHint` so the location
///   matching the document element can be selected.
/// * `xsi:noNamespaceSchemaLocation="loc"` on the root element.
/// * `<?xml-model href="loc" … ?>` processing instructions in the prolog.
///
/// Attributes are resolved by namespace URI, not by prefix, so documents
/// that bind xsi to a non-standard prefix are handled correctly.
pub struct SchemaExtractor;

impl SchemaExtractor {
    /// Create a new schema extractor. Zero-cost; no state.
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Extract every schema reference from the given XML file.
    ///
    /// Uses a sync `BufReader` over the file driven by quick-xml's pull
    /// parser. Because the parser stops at the root element's start tag,
    /// only the prolog + root start tag is actually read off disk — large
    /// XML bodies are never loaded into memory. The blocking read and
    /// parse happen on a tokio blocking thread so the async runtime stays
    /// free for I/O elsewhere.
    pub async fn extract_schema_urls(&self, file_path: &Path) -> Result<Vec<SchemaReference>> {
        Ok(self
            .extract_schema_hints(file_path)
            .await?
            .hints
            .into_iter()
            .map(|hint| hint.reference)
            .collect())
    }

    /// Extract namespace-preserving schema hints from an XML file.
    pub async fn extract_schema_hints(&self, file_path: &Path) -> Result<ExtractedSchemaHints> {
        let path = file_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&path).map_err(ValidationError::from)?;
            parse_schema_hints(std::io::BufReader::new(file), &path)
        })
        .await
        .map_err(|e| ValidationError::Concurrency {
            details: e.to_string(),
        })?
    }

    /// Resolve a raw schema reference string from an XML document into a
    /// fully-typed [`SchemaReference`]. Relative local paths are resolved
    /// against the XML file's parent directory so that the resulting
    /// [`SchemaReference::Local`] path is absolute-or-relative-to-cwd, not
    /// relative-to-some-other-XML-file's-directory.
    ///
    /// URL parsing is delegated to `url`, which is already part of reqwest's
    /// dependency graph. Only HTTP(S) references are remote; everything else
    /// remains a filesystem path.
    fn resolve_reference(raw: &str, xml_file_path: &Path) -> SchemaReference {
        if let Ok(url) = url::Url::parse(raw)
            && matches!(url.scheme(), "http" | "https")
        {
            SchemaReference::Remote(url.into())
        } else if raw.starts_with('/') || Path::new(raw).is_absolute() {
            SchemaReference::Local(raw.into())
        } else {
            SchemaReference::Local(xml_file_path.parent().unwrap_or(Path::new(".")).join(raw))
        }
    }
}

/// Pull-parse the given XML input, collecting every schema reference from
/// the prolog and the root element. Stops after the root element's Start
/// (or Empty) event — we never read past the first element's attributes.
fn parse_schema_hints<R: std::io::BufRead>(
    input: R,
    xml_file_path: &Path,
) -> Result<ExtractedSchemaHints> {
    let mut reader = NsReader::from_reader(input);
    // Strip leading/trailing whitespace from text events so indentation
    // doesn't produce noise. Does not affect attribute values.
    reader.config_mut().trim_text(true);

    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut hints = Vec::new();
    let mut root_namespace = None;

    loop {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf) {
            Err(e) => {
                return Err(ValidationError::XmlParsing {
                    file: xml_file_path.to_path_buf(),
                    details: format!("parse error at byte {}: {e}", reader.buffer_position()),
                });
            }
            Ok((_, Event::Eof)) => break,
            Ok((_, Event::PI(pi))) => {
                if pi.target() == XML_MODEL_PI_TARGET {
                    extract_xml_model_href(&pi, xml_file_path, &mut hints)?;
                }
            }
            Ok((namespace, Event::Start(ref start))) | Ok((namespace, Event::Empty(ref start))) => {
                root_namespace = match namespace {
                    ResolveResult::Bound(Namespace(uri)) => Some(
                        std::str::from_utf8(uri.as_ref())
                            .map_err(|error| ValidationError::XmlParsing {
                                file: xml_file_path.to_path_buf(),
                                details: format!("root namespace is not UTF-8: {error}"),
                            })?
                            .to_owned(),
                    ),
                    ResolveResult::Unbound | ResolveResult::Unknown(_) => None,
                };
                extract_xsi_schema_attrs(&reader, start, xml_file_path, &mut hints)?;
                // Root element reached; nothing past it can contribute
                // schema references.
                break;
            }
            // Decl, DocType, Comment, Text (ignorable whitespace only,
            // thanks to trim_text), CData, End (impossible before root),
            // GeneralRef — none contribute schema references.
            Ok(_) => {}
        }
    }

    if hints.is_empty() {
        return Err(ValidationError::SchemaUrlNotFound {
            file: xml_file_path.to_path_buf(),
        });
    }
    Ok(ExtractedSchemaHints {
        root_namespace,
        hints,
    })
}

/// Walk the attributes of the root start tag, emitting a `SchemaReference`
/// for each one that lives in the XMLSchema-instance namespace and names a
/// schema location. Attributes in other namespaces are ignored.
fn extract_xsi_schema_attrs<R: std::io::BufRead>(
    reader: &NsReader<R>,
    start: &BytesStart<'_>,
    xml_file_path: &Path,
    hints: &mut Vec<SchemaHint>,
) -> Result<()> {
    for attr_result in start.attributes().with_checks(false) {
        let attr = attr_result.map_err(|err| ValidationError::XmlParsing {
            file: xml_file_path.to_path_buf(),
            details: format!("malformed attribute: {err}"),
        })?;
        let (ns, local) = reader.resolver().resolve_attribute(attr.key);
        let is_xsi = matches!(
            ns,
            ResolveResult::Bound(Namespace(uri)) if uri == XSI_NAMESPACE_URI
        );
        if !is_xsi {
            continue;
        }
        let value = attr
            .normalized_value(ATTR_NORMALIZATION_VERSION)
            .map_err(|err| ValidationError::XmlParsing {
                file: xml_file_path.to_path_buf(),
                details: format!("attribute unescape: {err}"),
            })?;

        match local.as_ref() {
            "schemaLocation" => {
                // Per the XML Schema spec, schemaLocation is a
                // whitespace-separated list of `(namespace, location)`
                // pairs. Stream pairwise and surface an odd trailing token
                // as a malformed-attribute error, rather than silently
                // dropping it — a document with an unpaired token is
                // malformed and the caller deserves to know.
                let mut tokens = value.split_whitespace();
                while let Some(ns_token) = tokens.next() {
                    let Some(location) = tokens.next() else {
                        return Err(ValidationError::XmlParsing {
                            file: xml_file_path.to_path_buf(),
                            details: format!(
                                "xsi:schemaLocation has unpaired trailing token {ns_token:?}"
                            ),
                        });
                    };
                    hints.push(SchemaHint {
                        kind: SchemaHintKind::Namespace(ns_token.to_owned()),
                        reference: SchemaExtractor::resolve_reference(location, xml_file_path),
                    });
                }
            }
            "noNamespaceSchemaLocation" => {
                hints.push(SchemaHint {
                    kind: SchemaHintKind::NoNamespace,
                    reference: SchemaExtractor::resolve_reference(&value, xml_file_path),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Walk the pseudo-attributes of an xml-model PI, emitting a
/// `SchemaReference` for the first `href` value found. Other pseudo-attrs
/// (`type`, `schematypens`, `title`, …) are ignored — validation of XML
/// against Schematron etc. is out of scope for this tool.
///
/// quick-xml's `BytesPI::attributes()` handles pseudo-attribute quoting
/// and whitespace for us, so we don't need a hand-rolled scanner.
fn extract_xml_model_href(
    pi: &BytesPI<'_>,
    xml_file_path: &Path,
    hints: &mut Vec<SchemaHint>,
) -> Result<()> {
    for attr_result in pi.attributes() {
        let attr = attr_result.map_err(|err| ValidationError::XmlParsing {
            file: xml_file_path.to_path_buf(),
            details: format!("malformed xml-model pseudo-attribute: {err}"),
        })?;
        if attr.key.as_ref() != "href" {
            continue;
        }
        let value = attr
            .normalized_value(ATTR_NORMALIZATION_VERSION)
            .map_err(|err| ValidationError::XmlParsing {
                file: xml_file_path.to_path_buf(),
                details: format!("xml-model href unescape: {err}"),
            })?;
        hints.push(SchemaHint {
            kind: SchemaHintKind::XmlModel,
            reference: SchemaExtractor::resolve_reference(&value, xml_file_path),
        });
        return Ok(());
    }
    Ok(())
}

/// Unified async schema loader that handles both local and remote schemas
pub struct SchemaLoader {
    extractor: SchemaExtractor,
    cache: Arc<SchemaCache>,
    http_client: AsyncHttpClient,
}

impl SchemaLoader {
    pub fn new(cache: Arc<SchemaCache>, http_client: AsyncHttpClient) -> Result<Self> {
        let extractor = SchemaExtractor::new()?;

        Ok(Self {
            extractor,
            cache,
            http_client,
        })
    }

    /// Load the schema for an XML file.
    ///
    /// Namespace-aware xsi hints are matched to the document element;
    /// `xml-model` is used only as a fallback.
    pub async fn load_schema_for_file(&self, xml_file_path: &Path) -> Result<Arc<CachedSchema>> {
        let extracted = self.extractor.extract_schema_hints(xml_file_path).await?;
        let schema_ref =
            extracted
                .applicable()
                .ok_or_else(|| ValidationError::SchemaUrlNotFound {
                    file: xml_file_path.to_path_buf(),
                })?;

        self.load_schema(schema_ref).await
    }

    /// Load a schema by reference (local or remote)
    pub async fn load_schema(&self, schema_ref: &SchemaReference) -> Result<Arc<CachedSchema>> {
        match schema_ref {
            SchemaReference::Local(path) => self.load_local_schema(path).await,
            SchemaReference::Remote(url) => self.load_remote_schema(url).await,
        }
    }

    /// Return a cache key that changes when a local schema file changes.
    ///
    /// Remote schemas use their URL identity. Local schemas include the
    /// canonical path plus a SHA-256 digest of the root schema and its complete
    /// local composition graph, so edits to either the root or a dependency
    /// invalidate both raw and parsed entries.
    pub async fn cache_key(&self, schema_ref: &SchemaReference) -> Result<String> {
        match schema_ref {
            SchemaReference::Remote(_) => Ok(schema_ref.cache_key()),
            SchemaReference::Local(path) => local_schema_cache_key(path).await,
        }
    }

    /// Load a local schema file
    pub async fn load_local_schema(&self, schema_path: &Path) -> Result<Arc<CachedSchema>> {
        let cache_key = local_schema_cache_key(schema_path).await?;

        // Check cache first
        if let Some(cached_schema) = self.cache.get(&cache_key).await? {
            return Ok(cached_schema);
        }

        // Read the local schema file
        let schema_data = tokio::fs::read(schema_path)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ValidationError::SchemaNotFound {
                    url: schema_path.display().to_string(),
                },
                _ => ValidationError::from(e),
            })?;

        // Validate the schema content
        self.validate_schema_content(&schema_data, &schema_path.display().to_string())?;

        // Cache the schema (local schemas don't have ETags or Last-Modified headers)
        self.cache.set(&cache_key, schema_data, None, None).await
    }

    /// Load a remote schema with caching
    pub async fn load_remote_schema(&self, url: &str) -> Result<Arc<CachedSchema>> {
        // Check cache first
        if let Some(cached_schema) = self.cache.get(url).await? {
            return Ok(cached_schema);
        }

        // Download the schema
        let schema_data = self.http_client.download_schema(url).await?;

        // Validate the schema content
        self.validate_schema_content(&schema_data, url)?;
        crate::backend::enforce_resource_policy(&schema_data, url, None)?;

        // Cache the schema (TODO: extract ETags and Last-Modified from HTTP response)
        self.cache.set(url, schema_data, None, None).await
    }

    /// Probe the fetched bytes to confirm they are an XML Schema Definition
    /// (root element `{http://www.w3.org/2001/XMLSchema}schema`).
    ///
    /// This is a lightweight pre-flight at the loader boundary — enough to
    /// catch "this URL returned HTML/404/plain text" before we poison the
    /// disk cache with non-XSD bytes. Full XSD compilation is deferred to
    /// libxml2 at use time.
    ///
    /// The match is by namespace URI rather than by prefix, so
    /// `<xs:schema>`, `<xsd:schema>`, and any other prefix binding all
    /// work — as does a default-namespace-bound `<schema>`.
    fn validate_schema_content(&self, data: &[u8], source: &str) -> Result<()> {
        let mut reader = NsReader::from_reader(data);
        reader.config_mut().trim_text(true);
        let mut buf: Vec<u8> = Vec::with_capacity(256);

        loop {
            buf.clear();
            match reader.read_resolved_event_into(&mut buf) {
                Err(e) => {
                    return Err(ValidationError::SchemaParsing {
                        url: source.to_string(),
                        details: format!(
                            "XSD parse error at byte {}: {e}",
                            reader.buffer_position()
                        ),
                    });
                }
                Ok((_, Event::Eof)) => {
                    return Err(ValidationError::SchemaParsing {
                        url: source.to_string(),
                        details: "content has no root element".to_string(),
                    });
                }
                Ok((ns, Event::Start(ref start))) | Ok((ns, Event::Empty(ref start))) => {
                    let in_xsd_ns = matches!(
                        ns,
                        ResolveResult::Bound(Namespace(uri)) if uri == XSD_NAMESPACE_URI
                    );
                    let local = start.local_name();
                    if in_xsd_ns && local.as_ref() == "schema" {
                        return Ok(());
                    }
                    return Err(ValidationError::SchemaParsing {
                        url: source.to_string(),
                        details: "root element is not {http://www.w3.org/2001/XMLSchema}schema"
                            .to_string(),
                    });
                }
                // Decl, DocType, Comment, PI, CData before the root: keep
                // walking. A well-formed XSD reaches Start very quickly.
                Ok(_) => {}
            }
        }
    }

    /// Get the schema extractor for direct use
    pub fn extractor(&self) -> &SchemaExtractor {
        &self.extractor
    }

    /// Get the cache for direct access
    pub fn cache(&self) -> &Arc<SchemaCache> {
        &self.cache
    }

    /// Get the HTTP client for direct access
    pub fn http_client(&self) -> &AsyncHttpClient {
        &self.http_client
    }
}

async fn local_schema_cache_key(schema_path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let canonical = tokio::fs::canonicalize(schema_path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ValidationError::SchemaNotFound {
                url: schema_path.display().to_string(),
            },
            _ => ValidationError::from(error),
        })?;
    let data = tokio::fs::read(&canonical).await?;
    let mut hasher = Sha256::new();
    hash_schema_resource(&mut hasher, &canonical, &data);
    let scan_path = canonical.clone();
    let scan_data = data.clone();
    let mut dependencies = tokio::task::spawn_blocking(move || {
        crate::backend::local_dependency_paths(&scan_data, &scan_path)
    })
    .await
    .map_err(|error| ValidationError::Concurrency {
        details: format!("local schema dependency scan failed to join: {error}"),
    })??;
    dependencies.sort_unstable();
    for dependency in dependencies {
        let dependency_data = tokio::fs::read(&dependency).await?;
        hash_schema_resource(&mut hasher, &dependency, &dependency_data);
    }
    let digest = hasher.finalize();
    Ok(format!("local:{}:{digest:x}", canonical.display()))
}

fn hash_schema_resource(hasher: &mut sha2::Sha256, path: &Path, data: &[u8]) {
    use sha2::Digest;

    let path = path.as_os_str().as_encoded_bytes();
    hasher.update((path.len() as u64).to_le_bytes());
    hasher.update(path);
    hasher.update((data.len() as u64).to_le_bytes());
    hasher.update(data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use crate::http_client::HttpClientConfig;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    // All test helpers and test bodies return `Result<(), ValidationError>`
    // and propagate errors via `?` rather than using `.unwrap()` / `.expect()`,
    // per the project's no-panic rule. `assert!` / `assert_eq!` are retained —
    // those are the test framework's explicit failure mechanism, not ad-hoc
    // panics.

    fn create_test_cache() -> Result<(Arc<SchemaCache>, TempDir)> {
        let temp_dir = TempDir::new().map_err(ValidationError::from)?;
        let config = CacheConfig {
            directory: temp_dir.path().to_path_buf(),
            ttl_hours: 1,
            max_size_mb: 100,
            max_memory_entries: 100,
            memory_ttl_seconds: 300,
        };
        Ok((Arc::new(SchemaCache::new(config)), temp_dir))
    }

    fn create_test_http_client() -> Result<AsyncHttpClient> {
        AsyncHttpClient::new(HttpClientConfig::default())
    }

    /// Write `contents` to a fresh temp file and return it. The caller must
    /// hold the `NamedTempFile`; dropping it removes the file on disk.
    fn write_xml_file(contents: &str) -> Result<NamedTempFile> {
        let mut f = NamedTempFile::new().map_err(ValidationError::from)?;
        f.write_all(contents.as_bytes())
            .map_err(ValidationError::from)?;
        f.flush().map_err(ValidationError::from)?;
        Ok(f)
    }

    /// Short helper to produce an assertion-style `Err` when a test's
    /// match arm falls into an unexpected branch. Keeps test code free of
    /// `panic!` while still reporting the mismatch clearly.
    fn unexpected<T: std::fmt::Debug>(what: &str, got: T) -> ValidationError {
        ValidationError::Config(format!("{what}: got {got:?}"))
    }

    #[tokio::test]
    async fn test_schema_extractor_creation() -> Result<()> {
        let _extractor = SchemaExtractor::new()?;
        Ok(())
    }

    #[tokio::test]
    async fn applicable_hint_matches_root_namespace_not_first_pair() -> Result<()> {
        let file = write_xml_file(
            r#"<wanted:root xmlns:wanted="urn:wanted"
                xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
                xsi:schemaLocation="urn:other https://example.test/other.xsd urn:wanted https://example.test/wanted.xsd"/>"#,
        )?;
        let hints = SchemaExtractor::new()?
            .extract_schema_hints(file.path())
            .await?;
        assert_eq!(hints.root_namespace.as_deref(), Some("urn:wanted"));
        assert_eq!(
            hints.applicable().map(ToString::to_string).as_deref(),
            Some("https://example.test/wanted.xsd")
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_cache_key_changes_after_edit() -> Result<()> {
        let (cache, _cache_dir) = create_test_cache()?;
        let loader = SchemaLoader::new(cache, create_test_http_client()?)?;
        let file = write_xml_file("12345678")?;
        let reference = SchemaReference::Local(file.path().to_path_buf());
        let before = loader.cache_key(&reference).await?;
        // Same length, with no sleep: timestamp/size-only keys can miss this.
        tokio::fs::write(file.path(), "abcdefgh").await?;
        let after = loader.cache_key(&reference).await?;
        assert_ne!(before, after);
        Ok(())
    }

    #[tokio::test]
    async fn local_cache_key_changes_after_dependency_edit() -> Result<()> {
        let (cache, _cache_dir) = create_test_cache()?;
        let loader = SchemaLoader::new(cache, create_test_http_client()?)?;
        let directory = TempDir::new()?;
        let dependency = directory.path().join("types.xsd");
        tokio::fs::write(
            &dependency,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:annotation><xs:documentation>aaaa</xs:documentation></xs:annotation></xs:schema>"#,
        )
        .await?;
        let main = directory.path().join("main.xsd");
        tokio::fs::write(
            &main,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:include schemaLocation="types.xsd"/></xs:schema>"#,
        )
        .await?;
        let reference = SchemaReference::Local(main);
        let before = loader.cache_key(&reference).await?;
        tokio::fs::write(
            &dependency,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"><xs:annotation><xs:documentation>bbbb</xs:documentation></xs:annotation></xs:schema>"#,
        )
        .await?;
        let after = loader.cache_key(&reference).await?;
        assert_ne!(before, after);
        Ok(())
    }

    #[tokio::test]
    async fn test_extract_schema_location() -> Result<()> {
        let extractor = SchemaExtractor::new()?;
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/ns http://example.com/schema.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(
            refs,
            vec![SchemaReference::Remote(
                "http://example.com/schema.xsd".to_string()
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_extract_no_namespace_schema_location() -> Result<()> {
        let extractor = SchemaExtractor::new()?;
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="schema.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(refs.len(), 1);
        match &refs[0] {
            SchemaReference::Local(path) => {
                assert!(
                    path.to_string_lossy().ends_with("schema.xsd"),
                    "expected path to end in schema.xsd; got {}",
                    path.display()
                );
                Ok(())
            }
            other => Err(unexpected("expected SchemaReference::Local", other)),
        }
    }

    #[tokio::test]
    async fn test_extract_local_absolute_path() -> Result<()> {
        let extractor = SchemaExtractor::new()?;
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/ns /absolute/path/schema.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(
            refs,
            vec![SchemaReference::Local(PathBuf::from(
                "/absolute/path/schema.xsd"
            ))]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_extract_no_schema_found() -> Result<()> {
        let extractor = SchemaExtractor::new()?;
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root>
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;

        let result = extractor.extract_schema_urls(temp_file.path()).await;

        match result {
            Err(ValidationError::SchemaUrlNotFound { .. }) => Ok(()),
            other => Err(unexpected("expected SchemaUrlNotFound", other)),
        }
    }

    #[tokio::test]
    async fn test_schema_loader_creation() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let _loader = SchemaLoader::new(cache, http_client)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_load_local_schema() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="xs:string"/>
</xs:schema>
"#;
        let schema_file = write_xml_file(schema_xml)?;

        let cached_schema = loader.load_local_schema(schema_file.path()).await?;
        let content = std::str::from_utf8(&cached_schema.data).map_err(|e| {
            ValidationError::SchemaParsing {
                url: schema_file.path().display().to_string(),
                details: format!("cached schema not UTF-8: {e}"),
            }
        })?;
        assert!(content.contains("<xs:schema"));
        Ok(())
    }

    #[tokio::test]
    async fn test_load_local_schema_not_found() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let result = loader
            .load_local_schema(Path::new("/nonexistent/schema.xsd"))
            .await;

        match result {
            Err(ValidationError::SchemaNotFound { .. }) => Ok(()),
            other => Err(unexpected("expected SchemaNotFound", other)),
        }
    }

    #[tokio::test]
    async fn test_validate_schema_content_valid() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let valid_schema = br#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

        loader.validate_schema_content(valid_schema, "test.xsd")?;
        Ok(())
    }

    /// Random non-UTF-8 bytes should be rejected. quick-xml surfaces the
    /// encoding problem as a parse error; we only assert on the variant.
    #[tokio::test]
    async fn test_validate_schema_content_invalid_utf8() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let invalid_utf8 = &[0xFF, 0xFE, 0xFD];
        let result = loader.validate_schema_content(invalid_utf8, "test.xsd");

        match result {
            Err(ValidationError::SchemaParsing { .. }) => Ok(()),
            other => Err(unexpected("expected SchemaParsing", other)),
        }
    }

    /// Plain text with no XML markup must be rejected as non-schema.
    #[tokio::test]
    async fn test_validate_schema_content_not_xml() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let not_xml = b"This is not XML content";
        let result = loader.validate_schema_content(not_xml, "test.xsd");

        match result {
            Err(ValidationError::SchemaParsing { .. }) => Ok(()),
            other => Err(unexpected("expected SchemaParsing", other)),
        }
    }

    /// Well-formed XML whose root element is not `{XMLSchema}schema` must
    /// be rejected — the bytes are not an XSD.
    #[tokio::test]
    async fn test_validate_schema_content_not_schema() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let not_schema = br#"<?xml version="1.0" encoding="UTF-8"?>
<root>
  <element>This is XML but not a schema</element>
</root>"#;

        let result = loader.validate_schema_content(not_schema, "test.xsd");
        match result {
            Err(ValidationError::SchemaParsing { details, .. }) => {
                assert!(
                    details.contains("root element"),
                    "error should mention the root-element mismatch; got: {details}"
                );
                Ok(())
            }
            other => Err(unexpected("expected SchemaParsing", other)),
        }
    }

    #[tokio::test]
    async fn test_schema_loader_caching() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache.clone(), http_client)?;

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="root" type="xs:string"/>
</xs:schema>
"#;
        let schema_file = write_xml_file(schema_xml)?;

        let schema1 = loader.load_local_schema(schema_file.path()).await?;
        let schema2 = loader.load_local_schema(schema_file.path()).await?;
        assert_eq!(
            schema1.data, schema2.data,
            "second load should return the same bytes via cache"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_reference() -> Result<()> {
        let xml_path = Path::new("/path/to/xml/file.xml");

        assert_eq!(
            SchemaExtractor::resolve_reference("https://example.com/schema.xsd", xml_path),
            SchemaReference::Remote("https://example.com/schema.xsd".to_string())
        );
        assert_eq!(
            SchemaExtractor::resolve_reference("HTTPS://EXAMPLE.COM/schema.xsd", xml_path),
            SchemaReference::Remote("https://example.com/schema.xsd".to_string())
        );
        assert_eq!(
            SchemaExtractor::resolve_reference("/absolute/schema.xsd", xml_path),
            SchemaReference::Local(PathBuf::from("/absolute/schema.xsd"))
        );
        assert_eq!(
            SchemaExtractor::resolve_reference("schema.xsd", xml_path),
            SchemaReference::Local(PathBuf::from("/path/to/xml/schema.xsd"))
        );
        Ok(())
    }

    /// Regression test for the latent L1-cache collision that existed when
    /// `SchemaReference` kept the raw `url` field alongside a resolved
    /// `source_type`: two XML files in different directories that each
    /// referenced "schema.xsd" resolved to different paths but shared the
    /// same `url` key, colliding in the parsed-schema cache.
    #[test]
    fn test_cache_key_distinguishes_local_refs_by_full_path() {
        let a = SchemaReference::Local(PathBuf::from("/dir1/schema.xsd"));
        let b = SchemaReference::Local(PathBuf::from("/dir2/schema.xsd"));
        assert_ne!(a.cache_key(), b.cache_key());

        // And local/remote cache-key spaces must not bleed: a URL that
        // happens to look path-like shouldn't shadow a local file.
        let local = SchemaReference::Local(PathBuf::from("http://example.com/schema.xsd"));
        let remote = SchemaReference::Remote("http://example.com/schema.xsd".to_string());
        assert_ne!(local.cache_key(), remote.cache_key());
    }

    // ----------------------------------------------------------------------
    // Tests for issue #235: schema-loader heuristics are weak.
    // ----------------------------------------------------------------------

    /// A schemaLocation that appears inside a leading XML comment must be
    /// ignored; only the real attribute on the root element counts.
    #[tokio::test]
    async fn test_ignores_schema_location_inside_comment() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
    Example reference: xsi:schemaLocation="http://fake.example.com/ns http://fake.example.com/bogus.xsd"
    This is documentation inside a comment and must NOT drive schema loading.
-->
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:noNamespaceSchemaLocation="real.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(refs.len(), 1, "expected exactly one ref; got {refs:?}");
        match &refs[0] {
            SchemaReference::Local(path) => {
                assert!(
                    path.to_string_lossy().ends_with("real.xsd"),
                    "expected path to end in real.xsd; got {}",
                    path.display()
                );
                Ok(())
            }
            other => Err(unexpected("expected Local(real.xsd)", other)),
        }
    }

    /// xsi:schemaLocation is defined as whitespace-separated
    /// `(namespace location)+` pairs. Both locations must be extracted,
    /// and namespaces must NOT be returned as schema URLs.
    #[tokio::test]
    async fn test_multi_pair_schema_location_yields_multiple_refs() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/ns1 http://example.com/schema1.xsd http://example.com/ns2 http://example.com/schema2.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(
            refs,
            vec![
                SchemaReference::Remote("http://example.com/schema1.xsd".to_string()),
                SchemaReference::Remote("http://example.com/schema2.xsd".to_string()),
            ]
        );
        Ok(())
    }

    /// The schemaLocation attribute value may legitimately wrap across
    /// multiple lines; the extractor must handle that.
    #[tokio::test]
    async fn test_multi_line_schema_location_attribute() -> Result<()> {
        // NB: the attribute value intentionally spans two lines.
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<root xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n\
      xsi:schemaLocation=\"http://example.com/ns\n\
                          http://example.com/schema.xsd\">\n\
  <element>content</element>\n\
</root>\n";
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(
            refs,
            vec![SchemaReference::Remote(
                "http://example.com/schema.xsd".to_string()
            )]
        );
        Ok(())
    }

    /// The W3C `<?xml-model ?>` processing instruction is an alternative
    /// schema association mechanism. It must be honored when no
    /// xsi:schemaLocation / xsi:noNamespaceSchemaLocation is present.
    #[tokio::test]
    async fn test_xml_model_processing_instruction() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<?xml-model href="schema.xsd" type="application/xml" schematypens="http://www.w3.org/2001/XMLSchema"?>
<root>
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(refs.len(), 1, "expected one ref; got {refs:?}");
        match &refs[0] {
            SchemaReference::Local(path) => {
                assert!(
                    path.to_string_lossy().ends_with("schema.xsd"),
                    "expected path to end in schema.xsd; got {}",
                    path.display()
                );
                Ok(())
            }
            other => Err(unexpected("expected Local(schema.xsd)", other)),
        }
    }

    /// A `schemaLocation` with an odd number of whitespace-separated tokens
    /// is malformed (the last namespace has no paired location). Extraction
    /// must surface this as `XmlParsing`, not silently drop the orphan.
    #[tokio::test]
    async fn test_schema_location_with_unpaired_trailing_token_errors() -> Result<()> {
        // Three tokens: one full (ns, loc) pair plus a dangling namespace.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/ns1 http://example.com/a.xsd http://example.com/dangling">
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let result = extractor.extract_schema_urls(temp_file.path()).await;

        match result {
            Err(ValidationError::XmlParsing { details, .. }) => {
                assert!(
                    details.contains("unpaired"),
                    "error should describe the unpaired token; got: {details}"
                );
                Ok(())
            }
            other => Err(unexpected(
                "expected XmlParsing for odd-token schemaLocation",
                other,
            )),
        }
    }

    // ----------------------------------------------------------------------
    // Coverage fill-ins for claims made in module docs and AGENTS.md.
    // ----------------------------------------------------------------------

    /// Namespace-URI-based attribute resolution, not literal `xsi:` prefix
    /// matching. A document that binds the XMLSchema-instance namespace to
    /// a non-standard prefix (here `foo`) must still produce schema refs.
    #[tokio::test]
    async fn test_non_standard_xsi_prefix_binding() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:foo="http://www.w3.org/2001/XMLSchema-instance"
      foo:noNamespaceSchemaLocation="schema.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(refs.len(), 1, "expected one ref; got {refs:?}");
        match &refs[0] {
            SchemaReference::Local(path) => {
                assert!(
                    path.to_string_lossy().ends_with("schema.xsd"),
                    "expected path ending in schema.xsd; got {}",
                    path.display()
                );
                Ok(())
            }
            other => Err(unexpected(
                "expected Local(schema.xsd) from non-standard-prefix xsi binding",
                other,
            )),
        }
    }

    /// `validate_schema_content` must accept XSDs regardless of which prefix
    /// (or no prefix) is bound to the XMLSchema namespace. Happy path for
    /// `<xs:schema>` is already covered; these pin down the other two
    /// conventional forms.
    #[tokio::test]
    async fn test_validate_schema_content_accepts_xsd_prefix() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        let xsd_bytes = br#"<?xml version="1.0" encoding="UTF-8"?>
<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <xsd:element name="root" type="xsd:string"/>
</xsd:schema>"#;

        loader.validate_schema_content(xsd_bytes, "test.xsd")
    }

    #[tokio::test]
    async fn test_validate_schema_content_accepts_default_namespace_binding() -> Result<()> {
        let (cache, _temp_dir) = create_test_cache()?;
        let http_client = create_test_http_client()?;
        let loader = SchemaLoader::new(cache, http_client)?;

        // XMLSchema bound as the default namespace — no prefix at all.
        let xsd_bytes = br#"<?xml version="1.0" encoding="UTF-8"?>
<schema xmlns="http://www.w3.org/2001/XMLSchema">
  <element name="root" type="string"/>
</schema>"#;

        loader.validate_schema_content(xsd_bytes, "test.xsd")
    }

    /// `Display` for `SchemaReference` must emit a bare path/URL — the
    /// `local:` namespacing that `cache_key()` adds is an internal
    /// concern and must not leak into user-facing output (it reaches
    /// users via the validation outcome's schema field).
    #[test]
    fn test_schema_reference_display() {
        let local = SchemaReference::Local(PathBuf::from("/etc/schema.xsd"));
        assert_eq!(local.to_string(), "/etc/schema.xsd");

        let remote = SchemaReference::Remote("http://example.com/a.xsd".to_string());
        assert_eq!(remote.to_string(), "http://example.com/a.xsd");
    }

    /// Malformed XML input must surface through `XmlParsing`, not through a
    /// silent `SchemaUrlNotFound` or a panic. Here the root element's
    /// attribute has an unterminated quoted value, which quick-xml rejects
    /// well before emitting a `Start` event.
    #[tokio::test]
    async fn test_malformed_xml_input_returns_xml_parsing_error() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance
      xsi:noNamespaceSchemaLocation="schema.xsd">
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let result = extractor.extract_schema_urls(temp_file.path()).await;

        match result {
            Err(ValidationError::XmlParsing { details, .. }) => {
                assert!(
                    details.contains("parse error"),
                    "error should describe the parse failure; got: {details}"
                );
                Ok(())
            }
            other => Err(unexpected("expected XmlParsing for malformed XML", other)),
        }
    }

    /// When a document has both an `<?xml-model?>` PI in the prolog and an
    /// `xsi:schemaLocation` on the root, both sources are honored and the
    /// refs are emitted in document order (PI first, since the prolog
    /// precedes the root element).
    #[tokio::test]
    async fn test_combined_sources_emitted_in_document_order() -> Result<()> {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<?xml-model href="pi.xsd" type="application/xml" schematypens="http://www.w3.org/2001/XMLSchema"?>
<root xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
      xsi:schemaLocation="http://example.com/ns http://example.com/attr.xsd">
  <element>content</element>
</root>
"#;
        let temp_file = write_xml_file(xml)?;
        let extractor = SchemaExtractor::new()?;

        let refs = extractor.extract_schema_urls(temp_file.path()).await?;

        assert_eq!(refs.len(), 2, "expected PI + xsi ref; got {refs:?}");
        match &refs[0] {
            SchemaReference::Local(path) => assert!(
                path.to_string_lossy().ends_with("pi.xsd"),
                "first ref should be the xml-model PI (prolog precedes root); got {}",
                path.display()
            ),
            other => return Err(unexpected("expected Local(pi.xsd) first", other)),
        }
        assert_eq!(
            refs[1],
            SchemaReference::Remote("http://example.com/attr.xsd".to_string()),
            "second ref should be the xsi:schemaLocation location"
        );
        Ok(())
    }
}
