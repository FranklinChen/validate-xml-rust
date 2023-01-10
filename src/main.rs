use regex::Regex;
use std::path::{Path, PathBuf};
use std::io::prelude::*;
use std::io::BufReader;
use std::fs::File;
use lazy_static::lazy_static;
// TODO use clap
use docopt::Docopt;
use serde::Deserialize;
use std::ffi::CString;
use cached::proc_macro::cached;
use reqwest::blocking::Client;

/// For libxml2 FFI.
use libc::{c_char, c_int, c_uint, FILE};

/// Fake opaque structs from C libxml2.
pub enum XmlSchema {}
pub enum XmlSchemaParserCtxt {}
pub enum XmlSchemaValidCtxt {}

/// We know that libxml2 schema data structure is [thread-safe](http://xmlsoft.org/threads.hml).
#[derive(Clone, Copy)]
struct XmlSchemaPtr(pub *mut XmlSchema);

unsafe impl Send for XmlSchemaPtr {}
unsafe impl Sync for XmlSchemaPtr {}

#[link(name = "xml2")]
extern "C" {
    pub fn xmlInitParser();
    pub fn xmlInitGlobals();

    // xmlschemas
    pub fn xmlSchemaNewMemParserCtxt(buffer: *const c_char, size: c_int) -> *mut XmlSchemaParserCtxt;
    //pub fn xmlSchemaSetParserErrors();
    pub fn xmlSchemaParse(ctxt: *const XmlSchemaParserCtxt) -> *mut XmlSchema;
    pub fn xmlSchemaFreeParserCtxt(ctxt: *mut XmlSchemaParserCtxt);
    pub fn xmlSchemaDump(output: *mut FILE, schema: *const XmlSchema);
    pub fn xmlSchemaFree(schema: *mut XmlSchema);
    pub fn xmlSchemaNewValidCtxt(schema: *const XmlSchema) -> *mut XmlSchemaValidCtxt;
    pub fn xmlSchemaFreeValidCtxt(ctxt: *mut XmlSchemaValidCtxt);
    //pub fn xmlSchemaSetValidErrors();
    pub fn xmlSchemaValidateFile(
        ctxt: *const XmlSchemaValidCtxt,
        file_name: *const c_char,
        options: c_uint,
    ) -> c_int;
}

const USAGE: &str = "
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

#[derive(Deserialize)]
struct Args {
    flag_extension: String,
    arg_dir: String,
}

/// Return the first Schema URL found, if any.
/// Panic on any I/O error.
fn extract_schema_url(path: &Path) -> Option<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r#"xsi:schemaLocation="\S+\s+(.+?)""#)
            .expect("failed to compile schemaLocation regex");
    }

    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    for line in reader.lines() {
        if let Some(caps) = RE.captures(&line.unwrap()) {
            return Some(caps[1].to_owned());
        }
    }
    None
}

/// Cache schema into memory after downloading from Web once and stashing into memory.
///
/// Panics on I/O error.
#[cached(sync_writes = true)]
fn get_schema(url: String) -> XmlSchemaPtr {
    lazy_static! {
        static ref CLIENT: Client = Client::new();
    }

    // DEBUG to show that download happens only once.
    println!("Downloading now {url}...");

    let response = CLIENT.get(url.as_str()).send().unwrap().bytes().unwrap();

    unsafe {
        let schema_parser_ctxt = xmlSchemaNewMemParserCtxt(response.as_ptr() as *const c_char,
                                                           response.len() as i32);

        // Use default callbacks rather than overriding.
        //xmlSchemaSetParserErrors();

        let schema = xmlSchemaParse(schema_parser_ctxt);
        xmlSchemaFreeParserCtxt(schema_parser_ctxt);

        XmlSchemaPtr(schema)
    }
}

/// Copy the behavior of [`xmllint`](https://github.com/GNOME/libxml2/blob/master/xmllint.c)
fn validate(path_buf: PathBuf) {
    let url = extract_schema_url(path_buf.as_path()).unwrap();
    let schema = get_schema(url);

    let path_str = path_buf.to_str().unwrap();
    let c_path = CString::new(path_str).unwrap();

    unsafe {
        // Have to create new validation context for each parse.
        let schema_valid_ctxt = xmlSchemaNewValidCtxt(schema.0);

        // TODO better error message with integrated path using callback.
        //xmlSchemaSetValidErrors();

        // This reads the file and validates it.
        let result = xmlSchemaValidateFile(schema_valid_ctxt, c_path.as_ptr(), 0);
        if result == 0 {
            eprintln!("{path_str} validates");
        } else if result > 0 {
            // Note: the message is output after the validation messages.
            eprintln!("{path_str} fails to validate");
        } else {
            eprintln!(
                "{path_str} validation generated an internal error"
            );
        }

        xmlSchemaFreeValidCtxt(schema_valid_ctxt);
    }
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let extension_str = &(args.flag_extension);

    unsafe {
        xmlInitParser();
        xmlInitGlobals();
    }

    // No real point in using WalkParallel.
    for result in ignore::Walk::new(&args.arg_dir) {
        if let Ok(entry) = result {
            let path = entry.path().to_owned();
            if let Some(extension) = path.extension() {
                if extension.to_str().unwrap() == extension_str {
                    validate(path);
                }
            }
        }
    }
}
