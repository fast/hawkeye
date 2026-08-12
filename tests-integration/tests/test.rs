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
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::OnceLock;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn mixed_repository_lifecycle() {
    let project = case("mixed");
    assert_format_lifecycle("mixed", project.path(), false);

    let removed = hawkeye(
        project.path(),
        ["remove", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&removed, 0);
    assert_json_snapshot("mixed__remove", &removed);
    assert_tree_snapshot("mixed__tree_removed", project.path());

    let strict_unknown = hawkeye(project.path(), ["check", "--fail-if-unknown"]);
    assert_exit(&strict_unknown, 1);

    let default_failure = case("mixed");
    let formatted = hawkeye(default_failure.path(), ["format"]);
    assert_exit(&formatted, 1);
    assert!(
        read_normalized(default_failure.path().join("app.rs"))
            .starts_with("// Copyright 2026 Acme Labs\n// Sequence 1-2-3\n\n"),
        "format must write before applying the fail-if-updated policy"
    );
}

#[test]
fn preambles_and_line_endings_lifecycle() {
    let project = case("preambles");
    fs::write(
        project.path().join("bom.cs"),
        b"\xef\xbb\xbfpublic class Example {}\n",
    )
    .expect("write BOM source");
    fs::write(project.path().join("windows.rs"), b"fn main() {}\r\n").expect("write CRLF source");

    assert_format_lifecycle("preambles", project.path(), false);
}

#[test]
fn conflicting_header_lifecycle() {
    let project = case("conflict");
    assert_format_lifecycle("conflict", project.path(), true);
}

#[test]
fn git_index_and_ignore_lifecycle() {
    let project = case("git-ignore");
    git(project.path(), ["init", "-b", "main"]);
    git(project.path(), ["add", "-f", "tracked_ignored.rs"]);
    git(project.path(), ["add", "deleted.rs"]);
    fs::write(
        project.path().join(".git/info/exclude"),
        "info_ignored.rs\n",
    )
    .expect("write repository-local excludes");
    let global_excludes = project.path().join(".git/global-excludes");
    fs::write(&global_excludes, "global_ignored.rs\n").expect("write configured global excludes");
    git(
        project.path(),
        [
            "config".into(),
            "core.excludesFile".into(),
            global_excludes.into_os_string(),
        ],
    );
    fs::remove_file(project.path().join("deleted.rs")).expect("delete indexed source");

    assert_format_lifecycle("git_ignore", project.path(), false);
}

#[test]
fn git_repository_with_nested_files_root_lifecycle() {
    let project = case("nested-root");
    git(project.path(), ["init", "-b", "main"]);
    git(project.path(), ["add", "source/main.rs"]);

    assert_format_lifecycle("nested_root", project.path(), false);
}

#[test]
fn git_history_branches_dirty_files_and_untracked_directories() {
    let project = case("git-history");
    setup_history_repository(project.path());
    let year = Timestamp::now().to_zoned(TimeZone::UTC).year().to_string();

    assert_tree_snapshot("git_history__tree_before", project.path());

    let checked = hawkeye(project.path(), ["check", "--diff", "--output", "json"]);
    assert_exit(&checked, 1);
    assert_json_snapshot("git_history__check_before", &checked);
    insta::assert_snapshot!(
        "git_history__diff_before",
        normalize_year(&stderr(&checked), &year)
    );

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&formatted, 0);
    assert_json_snapshot("git_history__format", &formatted);
    insta::assert_snapshot!(
        "git_history__tree_after",
        normalize_year(&tree_snapshot(project.path()), &year)
    );

    let checked = hawkeye(project.path(), ["check", "--output", "json"]);
    assert_exit(&checked, 0);
    assert_json_snapshot("git_history__check_after", &checked);

    let idempotent = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&idempotent, 0);
    let report = assert_json_snapshot("git_history__format_idempotent", &idempotent);
    assert_eq!(changed_files(&report), 0);
}

#[test]
fn configuration_candidates_are_local_strict_and_ordered() {
    let project = tempfile::tempdir().expect("create config project");
    let child = project.path().join("child");
    fs::create_dir(&child).expect("create child directory");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = "Copyright 2026 Parent"

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write parent config");
    fs::write(child.join("source.rs"), "fn child() {}\n").expect("write child source");

    let implicit = hawkeye(&child, ["check"]);
    assert_exit(&implicit, 2);
    assert!(stderr(&implicit).contains("licenserc.toml"));

    let configured = hawkeye(&child, ["--config", "../licenserc.toml", "check", "--diff"]);
    assert_exit(&configured, 1);
    assert!(stdout(&configured).contains("Copyright 2026 Parent"));

    let fallback = tempfile::tempdir().expect("create fallback config project");
    fs::write(
        fallback.path().join(".licenserc.toml"),
        r#"[header]
text = "Copyright 2026 Fallback"

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write fallback config");
    fs::write(fallback.path().join("fallback.rs"), "fn fallback() {}\n")
        .expect("write fallback source");
    let fallback_result = hawkeye(fallback.path(), ["check", "--diff"]);
    assert_exit(&fallback_result, 1);
    assert!(stdout(&fallback_result).contains("Copyright 2026 Fallback"));

    fs::write(
        fallback.path().join("licenserc.toml"),
        r#"[header]
text = "Copyright 2026 Primary"

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write primary config");
    let primary_result = hawkeye(fallback.path(), ["check", "--diff"]);
    assert_exit(&primary_result, 1);
    assert!(stdout(&primary_result).contains("Copyright 2026 Primary"));
    assert!(!stdout(&primary_result).contains("Copyright 2026 Fallback"));

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
    assert_exit(&invalid, 2);
    assert!(stderr(&invalid).contains("useDefaultRules"));
}

#[test]
fn rendered_header_must_include_recognition_keywords() {
    let project = tempfile::tempdir().expect("create template validation project");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = "Confidential Siemens 2026"

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write configuration");
    fs::write(project.path().join("main.rs"), "fn main() {}\n").expect("write source");

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&formatted, 2);
    assert!(
        stderr(&formatted).contains("does not contain recognition keyword \"copyright\""),
        "{}",
        stderr(&formatted)
    );
    assert_eq!(read_normalized(project.path().join("main.rs")), "fn main() {}\n");
}

#[test]
fn malformed_header_is_not_partially_replaced() {
    let project = tempfile::tempdir().expect("create malformed header project");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
text = """
Copyright 2026 Acme
Licensed under Example
"""

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write configuration");
    let source = "// Copyright 2025 Acme\n\n// Licensed under Example\n\nfn main() {}\n";
    fs::write(project.path().join("main.rs"), source).expect("write malformed header");

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&formatted, 1);
    let report = json(&formatted);
    assert_eq!(report["files"][0]["status"], "conflict");
    assert_eq!(report["files"][0]["changed"], false);
    assert_eq!(read_normalized(project.path().join("main.rs")), source);
}

#[test]
fn header_template_path_is_not_a_source_target() {
    let project = tempfile::tempdir().expect("create header path project");
    fs::write(
        project.path().join("licenserc.toml"),
        r#"[header]
path = "license.rs"

[files]
includes = ["**/*.rs"]
"#,
    )
    .expect("write configuration");
    fs::write(project.path().join("license.rs"), "Copyright 2026 Acme\n")
        .expect("write header template");
    fs::write(project.path().join("main.rs"), "fn main() {}\n").expect("write source");

    let formatted = hawkeye(
        project.path(),
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&formatted, 0);
    let report = json(&formatted);
    assert_eq!(report["files"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["files"][0]["path"], "main.rs");
    assert_eq!(
        read_normalized(project.path().join("license.rs")),
        "Copyright 2026 Acme\n"
    );
}

fn assert_format_lifecycle(name: &str, project: &Path, conflict: bool) {
    assert_tree_snapshot(&format!("{name}__tree_before"), project);

    let checked = hawkeye(project, ["check", "--output", "json"]);
    assert_exit(&checked, 1);
    assert_json_snapshot(&format!("{name}__check_before"), &checked);

    let formatted = hawkeye(
        project,
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&formatted, i32::from(conflict));
    let formatted_report = assert_json_snapshot(&format!("{name}__format"), &formatted);
    assert!(changed_files(&formatted_report) > 0);
    assert_tree_snapshot(&format!("{name}__tree_after"), project);

    let checked = hawkeye(project, ["check", "--output", "json"]);
    assert_exit(&checked, i32::from(conflict));
    assert_json_snapshot(&format!("{name}__check_after"), &checked);

    let idempotent = hawkeye(
        project,
        ["format", "--fail-if-updated=false", "--output", "json"],
    );
    assert_exit(&idempotent, i32::from(conflict));
    let idempotent_report =
        assert_json_snapshot(&format!("{name}__format_idempotent"), &idempotent);
    assert_eq!(changed_files(&idempotent_report), 0);
}

fn case(name: &str) -> TempDir {
    let temporary = tempfile::tempdir().expect("create case directory");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases")
        .join(name);
    copy_tree(&source, temporary.path());
    temporary
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create case destination");
    for entry in fs::read_dir(source).expect("read case directory") {
        let entry = entry.expect("read case entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read case file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy case file");
        }
    }
}

fn hawkeye<I, S>(directory: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(hawkeye_binary())
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run hawkeye")
}

fn hawkeye_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("integration package is inside the workspace");
            let cargo = env!("CARGO");
            let metadata = Command::new(cargo)
                .args(["metadata", "--format-version=1", "--no-deps"])
                .current_dir(workspace)
                .output()
                .expect("read Cargo metadata");
            assert!(metadata.status.success(), "{}", stderr(&metadata));
            let metadata: Value =
                serde_json::from_slice(&metadata.stdout).expect("parse Cargo metadata");
            let target = PathBuf::from(
                metadata["target_directory"]
                    .as_str()
                    .expect("Cargo target directory"),
            );
            let output = Command::new(cargo)
                .args([
                    "build",
                    "--locked",
                    "--package",
                    "hawkeye",
                    "--bin",
                    "hawkeye",
                ])
                .current_dir(workspace)
                .output()
                .expect("build hawkeye binary");
            assert!(output.status.success(), "{}", stderr(&output));
            let path = target
                .join("debug")
                .join(format!("hawkeye{}", std::env::consts::EXE_SUFFIX));
            assert!(
                path.is_file(),
                "missing HawkEye binary at {}",
                path.display()
            );
            path
        })
        .as_path()
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(output.status.code(), Some(expected), "{}", stderr(output));
}

fn assert_json_snapshot(name: &str, output: &Output) -> Value {
    let report = json(output);
    insta::assert_json_snapshot!(name, &report);
    report
}

fn assert_tree_snapshot(name: &str, root: &Path) {
    insta::assert_snapshot!(name, tree_snapshot(root));
}

fn tree_snapshot(root: &Path) -> String {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();

    let mut snapshot = String::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .expect("collected file is inside repository");
        snapshot.push_str("=== ");
        snapshot.push_str(&relative.to_string_lossy().replace('\\', "/"));
        snapshot.push_str(" ===\n");
        let bytes = fs::read(&path).expect("read repository file");
        match std::str::from_utf8(&bytes) {
            Ok(text) => snapshot.push_str(&visible_text(text)),
            Err(_) => {
                snapshot.push_str("<hex>");
                for byte in bytes {
                    snapshot.push_str(&format!(" {byte:02x}"));
                }
                snapshot.push('\n');
            }
        }
    }
    snapshot
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .expect("read repository tree")
        .collect::<Result<Vec<_>, _>>()
        .expect("read repository entry");
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("repository entry is inside root");
        if relative
            .components()
            .next()
            .is_some_and(|part| part.as_os_str() == ".git")
        {
            continue;
        }
        if entry
            .file_type()
            .expect("read repository file type")
            .is_dir()
        {
            collect_files(root, &path, files);
        } else {
            files.push(path);
        }
    }
}

fn visible_text(text: &str) -> String {
    let mut visible = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' if characters.peek() == Some(&'\n') => {
                characters.next();
                visible.push_str("␍␊\n");
            }
            '\r' => visible.push('␍'),
            '\n' => visible.push_str("␊\n"),
            '\t' => visible.push_str("\\t"),
            '\u{feff}' => visible.push_str("\\u{feff}"),
            value if value.is_control() => visible.extend(value.escape_default()),
            value => visible.push(value),
        }
    }
    if !visible.ends_with('\n') {
        visible.push('\n');
    }
    visible
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("invalid JSON output: {error}\n{}", stdout(output)))
}

fn changed_files(report: &Value) -> usize {
    report["files"]
        .as_array()
        .expect("files array")
        .iter()
        .filter(|file| file["changed"] == true)
        .count()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn read_normalized(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .expect("read UTF-8 fixture")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn normalize_year(value: &str, year: &str) -> String {
    value.replace(year, "<CURRENT_YEAR>")
}

fn setup_history_repository(project: &Path) {
    git(project, ["init", "-b", "main"]);
    git(project, ["config", "user.name", "Current User"]);
    git(project, ["config", "user.email", "current@example.com"]);

    fs::write(project.join("app.rs"), "fn initial() {}\n").expect("write initial app");
    git(project, ["add", "app.rs"]);
    git_commit(
        project,
        "initial",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    git(project, ["switch", "-c", "feature"]);
    fs::write(project.join("app.rs"), "fn feature() {}\n").expect("write feature app");
    git(project, ["add", "app.rs"]);
    git_commit(
        project,
        "feature",
        "Bob",
        "bob@example.com",
        "2020-06-01T12:00:00+0000",
    );
    git(project, ["switch", "main"]);
    fs::write(project.join("other.rs"), "fn other() {}\n").expect("write other source");
    git(project, ["add", "other.rs"]);
    git_commit(
        project,
        "other",
        "Carol",
        "carol@example.com",
        "2021-06-01T12:00:00+0000",
    );
    git_with_env(
        project,
        ["merge", "--no-ff", "feature", "-m", "merge feature"],
        "Current User",
        "current@example.com",
        "2022-06-01T12:00:00+0000",
    );

    fs::write(project.join("app.rs"), "fn feature() {}\nfn dirty() {}\n")
        .expect("dirty tracked source");
    fs::create_dir(project.join("new")).expect("create untracked directory");
    fs::write(project.join("new/fresh.rs"), "fn untracked() {}\n").expect("write untracked source");
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
    assert!(output.status.success(), "Git failed:\n{}", stderr(&output));
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
    assert!(output.status.success(), "Git failed:\n{}", stderr(&output));
}
