#[macro_use]
extern crate lazy_static;

extern crate rustc_serialize;
extern crate docopt;

extern crate parking_lot;
extern crate libc;
extern crate walkdir;
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
  --extension=<extension>  File extension of XML files [default: .cmdi].
";

#[derive(Debug, RustcDecodable)]
struct Args {
    flag_extension: String,
    arg_dir: String,
}

use std::ffi::CString;
use std::collections::HashMap;
use std::io::BufReader;
use std::io::prelude::*;
use std::fs::File;
use std::path::Path;
use regex::Regex;
use parking_lot::RwLock;
use walkdir::{DirEntry, WalkDir};
use rayon::prelude::*;

/// For libxml2 FFI.
use libc::{c_char, c_int, c_uint, FILE};

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
        static ref RE: Regex = Regex::new(r#"xsi:schemaLocation="\S+\s+(.+?)""#).unwrap();
    }

    let f = File::open(path).unwrap();
    for line in BufReader::new(f).lines() {
        if let Some(caps) = RE.captures(&line.unwrap()) {
            return caps[1].to_string();
        }
    }
    unreachable!()
}

fn download_schema(url: &str) -> XmlSchemaPtr {
    let c_url = CString::new(url).unwrap();

    unsafe {
        let schema_parser_ctxt = xmlSchemaNewParserCtxt(c_url.as_ptr());
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
                // DEBUG to show that download happens only once.
                println!("Downloading now {}...", url);

                let schema = download_schema(&url);
                m.insert(url, schema);
                schema
            })
    })
}

/// Copy the behavior of [`xmllint`](https://github.com/GNOME/libxml2/blob/master/xmllint.c)
fn validate(e: &DirEntry) {
    let path = e.path();
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
            writeln!(std::io::stderr(), "{} validates", path_str);
        } else if result > 0 {
            // Note: the message is output after the validation messages.
            writeln!(std::io::stderr(), "{} fails to validate", path_str);
        } else {
            writeln!(std::io::stderr(), "{} validation generated an internal error", path_str);
        }

        xmlSchemaFreeValidCtxt(schema_valid_ctxt);
    }
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.decode())
        .unwrap_or_else(|e| e.exit());
    unsafe {
        xmlInitParser();
        xmlInitGlobals();
    }

    let entries = WalkDir::new(&args.arg_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| s.ends_with(&args.flag_extension))
                        .unwrap_or(false)
                })
        .collect::<Vec<_>>();

    &entries.par_iter().for_each(validate);
}
