// Copyright 2024 tison <wander4096@gmail.com>
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::HashMap;
use std::path::Path;

use hawkeye_fmt::config::ExistingStrategy;
use hawkeye_fmt::document::Attributes;
use hawkeye_fmt::document::Document;
use hawkeye_fmt::header::model::default_headers;
use hawkeye_fmt::header::parser::parse_header;
use hawkeye_fmt::header::parser::FileContent;

#[test]
fn test_remove_file_only_header() {
    let file = Path::new("tests/content/empty.py");
    let defs = default_headers();
    let def = defs.get("script_style").unwrap().clone();
    let keywords = vec!["copyright".to_string()];

    let file_content = FileContent::new(file).unwrap();
    let document = parse_header(file_content, &def, &keywords);
    let end_pos = document.end_pos.unwrap();
    let content = document.file_content.content();
    assert!(content[end_pos..].trim().is_empty());
}

#[test]
fn test_two_headers_should_only_remove_the_first() {
    let file = Path::new("tests/content/two_headers.rs");
    let defs = default_headers();
    let def = defs.get("doubleslash_style").unwrap().clone();
    let keywords = vec!["copyright".to_string()];

    let file_content = FileContent::new(file).unwrap();
    let document = parse_header(file_content, &def, &keywords);
    let end_pos = document.end_pos.unwrap();
    let content = document.file_content.content();
    assert!(content[end_pos..].contains("Copyright 2015 The Prometheus Authors"));
}

fn attributes(name: &str) -> Attributes {
    Attributes {
        filename: Some(name.to_string()),
        disk_file_created_year: None,
        git_file_created_year: None,
        git_file_modified_year: None,
        git_authors: Default::default(),
    }
}

fn rust_document(fixture: &str) -> Document {
    let defs = default_headers();
    let preferred = defs.get("doubleslash_style").unwrap().clone();
    let slashstar = defs.get("slashstar_style").unwrap().clone();
    Document::new(
        Path::new(fixture).to_path_buf(),
        preferred,
        vec![slashstar], // SLASHSTAR accepted for removal, so /* */ headers can migrate
        vec!["copyright".to_string()],
        ExistingStrategy::Replace,
        HashMap::new(),
        attributes(fixture),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn foreign_style_header_detected_and_removed() {
    let mut doc = rust_document("tests/content/foreign_style.rs");

    // The /* */ header is not in the preferred // style, so structural detection misses it,
    // but the keyword scan still recognizes the file as already licensed.
    assert!(!doc.header_detected());
    assert!(doc.looks_licensed());

    // Multi-style removal locates the header via the listed SLASHSTAR style and strips it.
    assert!(doc.remove_foreign_header());
    assert!(!doc.looks_licensed());
}

#[test]
fn unlicensed_file_is_not_detected() {
    let mut doc = rust_document("tests/content/no_header.rs");

    assert!(!doc.header_detected());
    assert!(!doc.looks_licensed());
    assert!(!doc.remove_foreign_header());
}
