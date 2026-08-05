// Copyright 2026 FastLabs Developers
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

#![cfg(feature = "cli")]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

fn hawkeye(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hawkeye"))
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap()
}

fn write_project() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(
        directory.path().join("hawkeye.toml"),
        r#"
[header]
text = "Copyright 2026 FastLabs Developers"
identifiers = ["Copyright"]

[[rules]]
patterns = ["**/*.rs"]
write_style = "slash"
read_styles = ["slash_star"]
"#,
    )
    .unwrap();
    fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    directory
}

#[test]
fn check_dry_run_format_and_clean_check_form_one_consistent_flow() {
    let directory = write_project();
    let source = directory.path().join("src/main.rs");

    let check = hawkeye(directory.path(), &["check"]);
    assert_eq!(check.status.code(), Some(1));
    assert!(String::from_utf8(check.stdout).unwrap().contains("missing"));

    let dry_run = hawkeye(directory.path(), &["format", "--dry-run", "--diff"]);
    assert_eq!(dry_run.status.code(), Some(1));
    let dry_run_output = String::from_utf8(dry_run.stdout).unwrap();
    assert!(dry_run_output.contains("--- a/src/main.rs"));
    assert!(dry_run_output.contains("+++ b/src/main.rs"));
    assert_eq!(fs::read_to_string(&source).unwrap(), "fn main() {}\n");

    let format = hawkeye(directory.path(), &["format", "--output-format", "json"]);
    assert_eq!(format.status.code(), Some(0));
    let output: serde_json::Value = serde_json::from_slice(&format.stdout).unwrap();
    assert_eq!(output["command"], "format");
    assert_eq!(output["changed"], 1);
    assert_eq!(output["report"]["files"][0]["status"], "missing");
    assert!(
        fs::read_to_string(&source)
            .unwrap()
            .starts_with("// Copyright 2026 FastLabs Developers\n\n")
    );

    let clean = hawkeye(directory.path(), &["check"]);
    assert_eq!(clean.status.code(), Some(0));
    assert!(!String::from_utf8(clean.stdout).unwrap().contains("dry run"));
}

#[test]
fn conflict_returns_one_and_preserves_the_original_bytes() {
    let directory = write_project();
    let source = directory.path().join("src/main.rs");
    let original = b"# Copyright 2025 FastLabs Developers\n\nfn main() {}\n";
    fs::write(&source, original).unwrap();

    let output = hawkeye(directory.path(), &["format"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("conflict")
    );
    assert_eq!(fs::read(source).unwrap(), original);
}
