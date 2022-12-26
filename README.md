# Validate XML against XML Schema using Rust

[![Build Status](https://travis-ci.org/FranklinChen/validate-xml-rust.png)](https://travis-ci.org/FranklinChen/validate-xml-rust)

## Prerequisites

Cargo for Rust is required. I recommend installing [`rustup`](https://www.rustup.rs/) with the instructions

```
$ curl https://sh.rustup.rs -sSf | sh
```

and then putting into your shell startup

```
export PATH="$HOME/.cargo/bin:$PATH"
```

## Installation

```
$ cargo install --path .
```

will install `validate-xml` into `$HOME/.cargo/bin`.

## Usage

Basic usage:

```
$ validate-xml root_dir 2> log.txt
```

Detailed usage:

```
$ validate-xml --help
Validate XML files concurrently and downloading remote XML Schemas only once.

Usage:
  validate-xml [--extension=<extension>] <dir>
  validate-xml (-h | --help)
  validate-xml --version

Options:
  -h --help                Show this screen.
  --version                Show version.
  --extension=<extension>  File extension of XML files [default: cmdi].
```

## Performance

This was written to be super fast:

- Concurrency, using number of cores available.
- Use of C FFI with `libxml2` library.
- Downloading of remote XML Schema files only once, and caching to `~/.xmlschemas/` for future runs of the program.
- Caching of `libxml2` schema data structure for reuse in multiple concurrent parses.

On my machine, it takes a few seconds to validate a sample set of 20,000 XML files. This is hundreds of times faster than the first attempt, which was a shell script that sequentially runs `xmllint` (after first pre-downloading remote XML Schema files and passing the local files to `xmllint` with `--schema`). The concurrency is a big win, as is the reuse of the `libxml2` schema data structure across threads and avoiding having to spawn `xmllint` as a heavyweight process.

## Comparison

If I had known about Python's `lxml` binding to C `libxml2`, I might not have bothered this Rust program. I found out about it and wrote the Python version of this program, which looks very similar (but without all the types): https://github.com/FranklinChen/validate-xml-python

However, as the Rust ecosystem fills in gaps in available libraries, I think I would actually be pretty happy to write in Rust instead of Python.
