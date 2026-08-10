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

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;
use tempfile::TempDir;

fn fixture(name: &str) -> TempDir {
    let temporary = tempfile::tempdir().expect("create fixture directory");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    copy_tree(&source, temporary.path());
    temporary
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture destination");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn hawkeye<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_hawkeye"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run hawkeye")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("invalid JSON output: {error}\n{}", stdout(output)))
}

fn status<'report>(report: &'report Value, path: &str) -> &'report str {
    report["files"]
        .as_array()
        .expect("files array")
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("missing report entry for {path}: {report}"))["status"]
        .as_str()
        .expect("status string")
}

#[test]
fn format_check_remove_directory_corpus() {
    let project = fixture("core");

    let checked = hawkeye(project.path(), ["check", "--output", "json"]);
    assert_eq!(checked.status.code(), Some(1), "{}", stderr(&checked));
    let report = json(&checked);
    assert_eq!(report.as_object().expect("report object").len(), 1);
    assert_eq!(status(&report, "app.rs"), "missing");
    assert_eq!(status(&report, "legacy.rs"), "replaceable");
    assert_eq!(status(&report, "types.d.ts"), "missing");
    assert_eq!(status(&report, "Makefile"), "missing");
    assert_eq!(status(&report, "notes.txt"), "unsupported");
    assert!(
        report["files"]
            .as_array()
            .expect("files array")
            .iter()
            .all(|file| file["path"] != "excluded/skip.rs")
    );

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    let report = json(&formatted);
    assert_eq!(
        report["files"]
            .as_array()
            .expect("files array")
            .iter()
            .filter(|file| file["changed"] == true)
            .count(),
        4
    );
    let canonical = "// Copyright 2026 Acme Labs\n// Sequence 1-2-3\n\n";
    assert!(
        fs::read_to_string(project.path().join("app.rs"))
            .expect("read app.rs")
            .starts_with(canonical)
    );
    assert!(
        fs::read_to_string(project.path().join("legacy.rs"))
            .expect("read legacy.rs")
            .starts_with(canonical)
    );
    assert!(
        fs::read_to_string(project.path().join("types.d.ts"))
            .expect("read types.d.ts")
            .starts_with(canonical)
    );
    assert!(
        fs::read_to_string(project.path().join("Makefile"))
            .expect("read Makefile")
            .starts_with(canonical)
    );
    assert_eq!(
        fs::read_to_string(project.path().join("excluded/skip.rs")).expect("read excluded file"),
        "fn excluded() {}\n"
    );

    let clean = hawkeye(project.path(), ["check"]);
    assert!(clean.status.success(), "{}", stderr(&clean));
    let strict_unknown = hawkeye(project.path(), ["check", "--fail-if-unknown"]);
    assert_eq!(
        strict_unknown.status.code(),
        Some(1),
        "{}",
        stderr(&strict_unknown)
    );

    let removed = hawkeye(project.path(), ["remove", "--fail-if-updated=false"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    assert_eq!(
        fs::read_to_string(project.path().join("app.rs")).expect("read removed app.rs"),
        "fn main() {}\n"
    );

    let default_failure = fixture("core");
    let formatted = hawkeye(default_failure.path(), ["format"]);
    assert_eq!(formatted.status.code(), Some(1), "{}", stderr(&formatted));
    assert!(
        fs::read_to_string(default_failure.path().join("app.rs"))
            .expect("read formatted source")
            .starts_with(canonical),
        "format writes before applying the fail-if-updated policy"
    );
}

#[test]
fn preserves_preambles_bom_and_line_endings() {
    let project = fixture("preambles");
    fs::write(
        project.path().join("bom.cs"),
        b"\xef\xbb\xbfpublic class Example {}\n",
    )
    .expect("write BOM source");
    fs::write(project.path().join("windows.rs"), b"fn main() {}\r\n").expect("write CRLF source");

    let formatted = hawkeye(project.path(), ["format", "--fail-if-updated=false"]);
    assert!(formatted.status.success(), "{}", stderr(&formatted));

    let script = fs::read_to_string(project.path().join("script.py")).expect("read script.py");
    assert!(script.starts_with(
        "#!/usr/bin/env python3\n# -*- coding: utf-8 -*-\n# Copyright 2026 Acme Labs\n\n"
    ));
    let xml = fs::read_to_string(project.path().join("document.xml")).expect("read XML source");
    assert!(xml.starts_with(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!--\n    Copyright 2026 Acme Labs\n-->\n\n"
    ));
    let bom = fs::read(project.path().join("bom.cs")).expect("read BOM source");
    assert!(bom.starts_with(b"\xef\xbb\xbf/*\n * Copyright 2026 Acme Labs\n */\n\n"));
    let windows = fs::read(project.path().join("windows.rs")).expect("read CRLF source");
    assert!(windows.windows(2).any(|pair| pair == b"\r\n"));
    assert!(
        windows
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte != b'\n' || index > 0 && windows[index - 1] == b'\r')
    );

    let checked = hawkeye(project.path(), ["check"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
}

#[test]
fn refuses_to_guess_an_unaccepted_header_style() {
    let project = fixture("conflict");
    let original =
        fs::read_to_string(project.path().join("foreign.rs")).expect("read foreign-style header");

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_eq!(formatted.status.code(), Some(1), "{}", stderr(&formatted));
    let report = json(&formatted);
    assert_eq!(status(&report, "foreign.rs"), "conflict");
    assert_eq!(
        fs::read_to_string(project.path().join("foreign.rs")).expect("reread foreign-style header"),
        original
    );
    assert!(
        fs::read_to_string(project.path().join("ordinary.rs"))
            .expect("read safely formatted source")
            .starts_with("// Confidential © Siemens 2026\n\n// An ordinary leading comment.")
    );
}

#[test]
fn explicit_files_bypass_git_ignore_but_not_config_excludes() {
    let project = tempfile::tempdir().expect("create discovery project");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = "Copyright 2026 Acme Labs"

[files]
includes = ["**/*.rs"]
excludes = ["excluded.rs"]
"#,
    )
    .expect("write config");
    fs::write(project.path().join(".gitignore"), "ignored.rs\n").expect("write Git ignore file");
    fs::write(project.path().join("ignored.rs"), "fn ignored() {}\n")
        .expect("write ignored source");
    fs::write(project.path().join("excluded.rs"), "fn excluded() {}\n")
        .expect("write excluded source");
    git(project.path(), ["init", "-b", "main"]);

    let walked = hawkeye(project.path(), ["check", "--output", "json"]);
    assert!(walked.status.success(), "{}", stderr(&walked));
    assert_eq!(
        json(&walked)["files"]
            .as_array()
            .expect("files array")
            .len(),
        0
    );

    let explicit = hawkeye(project.path(), ["check", "ignored.rs"]);
    assert_eq!(explicit.status.code(), Some(1), "{}", stderr(&explicit));
    assert!(stdout(&explicit).contains("ignored.rs"));

    let excluded = hawkeye(project.path(), ["check", "excluded.rs"]);
    assert!(excluded.status.success(), "{}", stderr(&excluded));
    assert!(stdout(&excluded).starts_with("0 files"));
}

#[test]
fn config_is_exact_snake_case_and_never_searched_upward() {
    let project = tempfile::tempdir().expect("create config project");
    let child = project.path().join("child");
    fs::create_dir(&child).expect("create child directory");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = "Copyright 2026 Acme Labs"

[files]
includes = ["**/*.rs"]
excludes = ["licenserc.toml"]
"#,
    )
    .expect("write parent config");
    fs::write(child.join("source.rs"), "fn child() {}\n").expect("write child source");

    let implicit = hawkeye(&child, ["check"]);
    assert_eq!(implicit.status.code(), Some(2));
    assert!(stderr(&implicit).contains("licenserc.toml"));

    let explicit = hawkeye(
        &child,
        ["--config", "../licenserc.toml", "check", "source.rs"],
    );
    assert_eq!(explicit.status.code(), Some(1), "{}", stderr(&explicit));
    assert!(stdout(&explicit).contains("child/source.rs"));

    fs::write(
        child.join("bad.toml"),
        r#"[header]
text = "Copyright"

[files]
useDefaultRules = true
"#,
    )
    .expect("write invalid config");
    let invalid = hawkeye(&child, ["--config", "bad.toml", "check"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(stderr(&invalid).contains("useDefaultRules"));
}

#[test]
fn git_file_attrs_cover_merges_dirty_files_and_untracked_directories() {
    let project = tempfile::tempdir().expect("create Git project");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = "Copyright {{ attrs.git_file_created_year }}-{{ attrs.git_file_modified_year }} {{ attrs.git_authors | join(', ') }}"

[files]
includes = ["**/*.rs"]
excludes = ["licenserc.toml"]

[git]
ignore = "disable"
file_attrs = "enable"
"#,
    )
    .expect("write Git attrs config");
    git(project.path(), ["init", "-b", "main"]);
    git(project.path(), ["config", "user.name", "Current User"]);
    git(
        project.path(),
        ["config", "user.email", "current@example.com"],
    );

    fs::write(project.path().join("app.rs"), "fn initial() {}\n").expect("write initial app");
    git(project.path(), ["add", "app.rs"]);
    git_commit(
        project.path(),
        "initial",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    git(project.path(), ["switch", "-c", "feature"]);
    fs::write(project.path().join("app.rs"), "fn feature() {}\n").expect("write feature app");
    git(project.path(), ["add", "app.rs"]);
    git_commit(
        project.path(),
        "feature",
        "Bob",
        "bob@example.com",
        "2020-06-01T12:00:00+0000",
    );
    git(project.path(), ["switch", "main"]);
    fs::write(project.path().join("other.rs"), "fn other() {}\n").expect("write other source");
    git(project.path(), ["add", "other.rs"]);
    git_commit(
        project.path(),
        "other",
        "Carol",
        "carol@example.com",
        "2021-06-01T12:00:00+0000",
    );
    git_with_env(
        project.path(),
        ["merge", "--no-ff", "feature", "-m", "merge feature"],
        "Current User",
        "current@example.com",
        "2022-06-01T12:00:00+0000",
    );

    let historical = hawkeye(project.path(), ["check", "--diff", "app.rs"]);
    assert_eq!(historical.status.code(), Some(1), "{}", stderr(&historical));
    assert!(
        stdout(&historical).contains("Copyright 2019-2020 Alice, Bob"),
        "{}",
        stdout(&historical)
    );
    assert!(!stdout(&historical).contains("2019-2022"));

    fs::write(
        project.path().join("app.rs"),
        "fn feature() {}\nfn dirty() {}\n",
    )
    .expect("dirty tracked source");
    fs::create_dir(project.path().join("new")).expect("create untracked directory");
    fs::write(project.path().join("new/fresh.rs"), "fn untracked() {}\n")
        .expect("write untracked source");

    let formatted = hawkeye(
        project.path(),
        [
            "format",
            "--fail-if-updated=false",
            "app.rs",
            "new/fresh.rs",
        ],
    );
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    let year = Timestamp::now().to_zoned(TimeZone::UTC).year();
    let app = fs::read_to_string(project.path().join("app.rs")).expect("read dirty app");
    assert!(
        app.starts_with(&format!(
            "// Copyright 2019-{year} Alice, Bob, Current User\n\n"
        )),
        "{app}"
    );
    let fresh = fs::read_to_string(project.path().join("new/fresh.rs")).expect("read fresh source");
    assert!(
        fresh.starts_with(&format!("// Copyright {year}-{year} Current User\n\n")),
        "{fresh}"
    );

    let checked = hawkeye(project.path(), ["check", "app.rs", "new/fresh.rs"]);
    assert!(checked.status.success(), "{}", stderr(&checked));
}

fn git<I, S>(directory: &Path, arguments: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_commit(directory: &Path, message: &str, name: &str, email: &str, date: &str) {
    git_with_env(directory, ["commit", "-m", message], name, email, date);
}

fn git_with_env<I, S>(directory: &Path, arguments: I, name: &str, email: &str, date: &str)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_AUTHOR_NAME", name)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", name)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .expect("run Git with identity");
    assert!(
        output.status.success(),
        "Git failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
