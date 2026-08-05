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

//! End-to-end command-line behavior tests.

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

    let json_check = hawkeye(directory.path(), &["check", "--output-format", "json"]);
    assert_eq!(json_check.status.code(), Some(1));
    let output: serde_json::Value = serde_json::from_slice(&json_check.stdout).unwrap();
    assert_eq!(output["changed"], 1, "{output:#}");
    assert!(output.get("dry_run").is_none());

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
    assert_eq!(output["dry_run"], false);
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

    let remove_dry_run = hawkeye(directory.path(), &["remove", "--dry-run", "--diff"]);
    assert_eq!(remove_dry_run.status.code(), Some(1));
    assert!(
        String::from_utf8(remove_dry_run.stdout)
            .unwrap()
            .contains("-// Copyright 2026 FastLabs Developers")
    );
    assert!(
        fs::read_to_string(&source)
            .unwrap()
            .starts_with("// Copyright 2026 FastLabs Developers")
    );

    let remove = hawkeye(directory.path(), &["remove", "--output-format", "json"]);
    assert_eq!(remove.status.code(), Some(0));
    let output: serde_json::Value = serde_json::from_slice(&remove.stdout).unwrap();
    assert_eq!(output["changed"], 1);
    assert_eq!(fs::read_to_string(&source).unwrap(), "fn main() {}\n");
}

#[test]
fn conflict_returns_one_and_preserves_the_original_bytes() {
    let directory = write_project();
    let source = directory.path().join("src/main.rs");
    let safe_source = directory.path().join("src/safe.rs");
    let original = b"# Copyright 2025 FastLabs Developers\n\nfn main() {}\n";
    fs::write(&source, original).unwrap();
    fs::write(&safe_source, "pub fn safe() {}\n").unwrap();

    let output = hawkeye(directory.path(), &["format"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("conflict")
    );
    assert_eq!(fs::read(source).unwrap(), original);
    assert!(
        fs::read_to_string(safe_source)
            .unwrap()
            .starts_with("// Copyright 2026 FastLabs Developers")
    );
}

#[test]
fn invalid_output_combination_fails_before_configuration_loading() {
    let directory = tempfile::tempdir().unwrap();

    let output = hawkeye(
        directory.path(),
        &["check", "--diff", "--output-format", "json"],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--diff cannot be combined with --output-format json")
    );
}

#[cfg(unix)]
#[test]
fn report_json_survives_a_non_unicode_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    let filename = OsString::from_vec(b"invalid-\x80.rs".to_vec());
    let report = hawkeye::Report::new(vec![hawkeye::FileOutcome::new(
        PathBuf::from(filename),
        hawkeye::Status::Unsupported,
    )]);
    let value = serde_json::to_value(report).unwrap();

    assert!(value["files"].as_array().is_some());
}
