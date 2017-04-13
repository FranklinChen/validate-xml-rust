#[macro_use]
extern crate lazy_static;

extern crate hyper;
extern crate rustc_serialize;
extern crate docopt;

extern crate parking_lot;
extern crate libc;
extern crate ignore;
extern crate rayon;
extern crate regex;

use docopt::Docopt;

const USAGE: &'static str = "
Validate XML files concurrently and downloading remote XML Schemas only once.

Usage:
  validate-xml [--extension=<extension>] <dir>
  validate-xml (-h | --help)
  validate-xml --version

Options:
  -h --help                Show this screen.
  --version                Show version.
  --extension=<extension>  File extension of XML files [default: cmdi].
";

#[derive(Debug, RustcDecodable)]
struct Args {
    flag_extension: String,
    arg_dir: String,
}

use std::env;
use std::fs;
use std::ffi::CString;
use std::collections::HashMap;
use std::io::BufReader;
use std::io::prelude::*;
use std::fs::File;
use std::path::{Path, PathBuf};
use regex::Regex;
use parking_lot::RwLock;
use ignore::Walk;
use hyper::Client;

/// For libxml2 FFI.
use libc::{c_char, c_int, c_uint, FILE};

/// Fake opaque structs from C libxml2.
pub enum XmlSchema {}
pub enum XmlSchemaParserCtxt {}
pub enum XmlSchemaValidCtxt {}

/// We know that libxml2 schema data structure is [thread-safe](http://xmlsoft.org/threads.html).
#[derive(Clone, Copy)]
struct XmlSchemaPtr(pub *mut XmlSchema);

unsafe impl Send for XmlSchemaPtr {}
unsafe impl Sync for XmlSchemaPtr {}

#[link(name = "xml2")]
extern "C" {
    pub fn xmlInitParser();
    pub fn xmlInitGlobals();

    // xmlschemas
    pub fn xmlSchemaNewParserCtxt(url: *const c_char) -> *mut XmlSchemaParserCtxt;
    //pub fn xmlSchemaSetParserErrors();
    pub fn xmlSchemaParse(ctxt: *const XmlSchemaParserCtxt) -> *mut XmlSchema;
    pub fn xmlSchemaFreeParserCtxt(ctxt: *mut XmlSchemaParserCtxt);
    pub fn xmlSchemaDump(output: *mut FILE, schema: *const XmlSchema);
    pub fn xmlSchemaFree(schema: *mut XmlSchema);
    pub fn xmlSchemaNewValidCtxt(schema: *const XmlSchema) -> *mut XmlSchemaValidCtxt;
    pub fn xmlSchemaFreeValidCtxt(ctxt: *mut XmlSchemaValidCtxt);
    //pub fn xmlSchemaSetValidErrors();
    pub fn xmlSchemaValidateFile(ctxt: *const XmlSchemaValidCtxt,
                                 file_name: *const c_char,
                                 options: c_uint)
                                 -> c_int;
}

fn extract_schema_url(path: &Path) -> String {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#"xsi:schemaLocation="\S+\s+(.+?)""#)
	    .expect("failed to compile schemaLocation regex");
    }

    let f = File::open(path).unwrap();
    for line in BufReader::new(f).lines() {
        if let Some(caps) = RE.captures(&line.unwrap()) {
            return caps[1].to_string();
        }
    }
    unreachable!()
}


/// Cache into ~/.xmlschemas/ directory.
fn download_schema(url: &str) -> XmlSchemaPtr {
    lazy_static! {
        static ref CLIENT: Client = Client::new();

        static ref SCHEMA_DIR: PathBuf = {
            let home_dir = env::home_dir().expect("could not find home directory");
            let schema_dir = home_dir.join(".xmlschemas");

            fs::create_dir_all(schema_dir.as_path())
                .expect(&format!("could not create {}", schema_dir.display()));
            schema_dir
        };
    }

    let encoded_file_name = url.replace("/", "%2F");
    let file_path_buf = SCHEMA_DIR.join(encoded_file_name);
    let file_path = file_path_buf.as_path();

    if file_path.is_file() {
        // Already cached. (Hopefully not trash.)
    } else {
        // Synchronously download from Web to local file.
        // TODO Make async?

        // DEBUG to show that download happens only once.
        println!("Downloading now {}...", url);

        let mut response = CLIENT.get(url).send().unwrap();
        let mut new_file = File::create(file_path)
            .expect(&format!("could not create cache file {}", file_path.display()));
        let mut buf = Vec::new();
        response.read_to_end(&mut buf).expect("read_to_end failed");
        new_file.write_all(&buf).expect("write_all failed");
    }

    let c_url = CString::new(file_path.to_str().unwrap()).unwrap();

    unsafe {
        let schema_parser_ctxt = xmlSchemaNewParserCtxt(c_url.as_ptr());

        // Use default callbacks rather than overriding.
        //xmlSchemaSetParserErrors();

        let schema = xmlSchemaParse(schema_parser_ctxt);
        xmlSchemaFreeParserCtxt(schema_parser_ctxt);

        XmlSchemaPtr(schema)
    }
}

/// Download an XML Schema file only if necessary.
/// Many threads could be trying.
fn get_schema(url: String) -> XmlSchemaPtr {
    lazy_static! {
        static ref M: RwLock<HashMap<String, XmlSchemaPtr>> = RwLock::new(HashMap::new());
    }

    {
        let m = M.read();
        m.get(&url).map(|&s| s)
    }
    .unwrap_or_else(|| {
        let mut m = M.write();

        // Double-checked locking pattern.
        m.get(&url)
            .map(|&s| s)
            .unwrap_or_else(|| {
                // TODO make async?
                let schema = download_schema(&url);
                m.insert(url, schema);
                schema
            })
    })
}

/// Copy the behavior of [`xmllint`](https://github.com/GNOME/libxml2/blob/master/xmllint.c)
fn validate(path: &Path) {
    let url = extract_schema_url(path);
    let schema = get_schema(url);

    let path_str = path.to_str().unwrap();
    let c_path = CString::new(path_str).unwrap();

    unsafe {
        // Have to create new validation context for each parse.
        let schema_valid_ctxt = xmlSchemaNewValidCtxt(schema.0);

        // TODO better error message with integrated path using callback.
        //xmlSchemaSetValidErrors();
        let result = xmlSchemaValidateFile(schema_valid_ctxt, c_path.as_ptr(), 0);
        if result == 0 {
            writeln!(std::io::stderr(), "{} validates", path_str).unwrap();
        } else if result > 0 {
            // Note: the message is output after the validation messages.
            writeln!(std::io::stderr(), "{} fails to validate", path_str).unwrap();
        } else {
            writeln!(std::io::stderr(), "{} validation generated an internal error", path_str).unwrap();
        }

        xmlSchemaFreeValidCtxt(schema_valid_ctxt);
    }
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.decode())
        .unwrap_or_else(|e| e.exit());
    let extension_str = &(args.flag_extension);

    unsafe {
        xmlInitParser();
        xmlInitGlobals();
    }

    rayon::scope(|scope| {
        for result in Walk::new(&args.arg_dir) {
            scope.spawn(move |_| {
                if let Ok(entry) = result {
                    let path = entry.path();
                    if let Some(extension) = path.extension() {
                        if extension.to_str().unwrap() == extension_str {
                            validate(path);
                        }
                    }
                }
            });
        }
    });
}
