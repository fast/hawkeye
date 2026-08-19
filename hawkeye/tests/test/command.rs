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

use std::fs;

use super::support::Project;
use super::support::assert_exit;
use super::support::assert_report;
use super::support::stderr;
use super::support::stdout;

#[test]
fn reports_and_exit_codes_follow_command_policy() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["main.rs", "notes.txt"]

[git]
ignore = "disable"
"#,
    );
    project.write("main.rs", "fn main() {}\n");
    project.write("notes.txt", "notes\n");

    let checked = project.run(["check"]);
    assert_exit(&checked, 1);
    assert_eq!(
        stdout(&checked),
        "        add  main.rs\nunsupported  notes.txt\n2 files, 1 change, 0 conflicts, 1 unsupported\n"
    );
    assert!(stderr(&checked).is_empty());

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 1);
    assert_report(
        &checked,
        &[("main.rs", "add"), ("notes.txt", "unsupported")],
    );
    assert!(stderr(&checked).is_empty());

    let formatted = project.run(["format", "--fail-on-change"]);
    assert_exit(&formatted, 1);
    assert!(
        project
            .read("main.rs")
            .starts_with("// Copyright 2026 Acme\n\n")
    );

    let checked = project.run(["check"]);
    assert_exit(&checked, 0);
    assert_eq!(
        stdout(&checked),
        "unsupported  notes.txt\n2 files, 0 changes, 0 conflicts, 1 unsupported\n"
    );

    let strict = project.run(["check", "--fail-on-unknown"]);
    assert_exit(&strict, 1);

    let removed = project.run(["remove"]);
    assert_exit(&removed, 0);
    assert_eq!(project.read("main.rs"), "fn main() {}\n");
}

#[test]
fn dry_run_reports_changes_without_writing_files() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]

[git]
ignore = "disable"
"#,
    );
    project.write("main.rs", "fn main() {}\n");

    let formatted = project.run([
        "format",
        "--dry-run",
        "--fail-on-change",
        "--output-format=json",
    ]);
    assert_exit(&formatted, 1);
    assert_report(&formatted, &[("main.rs", "add")]);
    assert_eq!(project.read("main.rs"), "fn main() {}\n");
}

#[test]
fn commands_process_requested_files_and_directories() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
root = "source"
includes = ["**/*.rs"]

[git]
ignore = "enable"
"#,
    );
    project.write("source/selected.rs", "fn selected() {}\n");
    project.write("source/untouched.rs", "fn untouched() {}\n");
    project.write("source/nested/selected.rs", "fn nested() {}\n");
    project.git(["init", "--initial-branch=main"]);

    let formatted = project.run([
        "format",
        "source/selected.rs",
        "source/nested",
        "--output-format=json",
    ]);
    assert_exit(&formatted, 0);
    assert_report(
        &formatted,
        &[("nested/selected.rs", "add"), ("selected.rs", "add")],
    );
    assert!(
        project
            .read("source/selected.rs")
            .starts_with("// Copyright 2026 Acme\n")
    );
    assert!(
        project
            .read("source/nested/selected.rs")
            .starts_with("// Copyright 2026 Acme\n")
    );
    assert_eq!(project.read("source/untouched.rs"), "fn untouched() {}\n");
}

#[test]
fn requested_paths_outside_root_are_quietly_ignored() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
root = "source"
includes = ["**/*.rs"]

[git]
ignore = "disable"
"#,
    );
    project.write("source/selected.rs", "fn selected() {}\n");
    project.write("outside.rs", "fn outside() {}\n");

    let formatted = project.run(["format", "outside.rs", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[]);
    assert!(stderr(&formatted).is_empty());
    assert_eq!(project.read("outside.rs"), "fn outside() {}\n");
}

#[test]
fn requested_directories_respect_git_ignore() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]

[git]
ignore = "enable"
"#,
    );
    project.write(".gitignore", "ignored/\n");
    project.write("ignored/inside.rs", "fn ignored() {}\n");
    project.write("visible/inside.rs", "fn visible() {}\n");
    project.git(["init", "--initial-branch=main"]);

    let directories = project.run(["format", "ignored", "visible", "--output-format=json"]);
    assert_exit(&directories, 0);
    assert_report(&directories, &[("visible/inside.rs", "add")]);
    assert_eq!(project.read("ignored/inside.rs"), "fn ignored() {}\n");

    let direct = project.run(["format", "ignored/inside.rs", "--output-format=json"]);
    assert_exit(&direct, 0);
    assert_report(&direct, &[("ignored/inside.rs", "add")]);
}

#[test]
fn explicit_files_bypass_git_ignore_but_obey_config_filters() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]
excludes = ["excluded.rs"]

[git]
ignore = "enable"
"#,
    );
    project.write(".gitignore", "ignored.rs\n");
    project.write("ignored.rs", "fn ignored() {}\n");
    project.write("selected.rs", "fn selected() {}\n");
    project.write("excluded.rs", "fn excluded() {}\n");
    project.git(["init", "--initial-branch=main"]);

    let formatted = project.run([
        "format",
        "ignored.rs",
        "selected.rs",
        "excluded.rs",
        "missing.rs",
        "--output-format=json",
    ]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[("ignored.rs", "add"), ("selected.rs", "add")]);
    assert!(stderr(&formatted).contains("skipping path that does not exist"));
    assert!(stderr(&formatted).contains("missing.rs"));
    assert_eq!(project.read("excluded.rs"), "fn excluded() {}\n");
}

#[test]
fn files_from_accepts_files_stdin_and_empty_input() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
root = "source"
includes = ["**/*.rs"]

[git]
ignore = "disable"
"#,
    );
    project.write("source/first.rs", "fn first() {}\n");
    project.write("source/second.rs", "fn second() {}\n");
    project.write("paths.txt", "source/first.rs\r\n");

    let from_file = project.run(["format", "--files-from=paths.txt", "--output-format=json"]);
    assert_exit(&from_file, 0);
    assert_report(&from_file, &[("first.rs", "add")]);

    let from_stdin = project.run_with_stdin(
        ["format", "--files-from=-", "--output-format=json"],
        b"source/second.rs\n",
    );
    assert_exit(&from_stdin, 0);
    assert_report(&from_stdin, &[("second.rs", "add")]);

    let empty = project.run_with_stdin(["check", "--files-from=-", "--output-format=json"], []);
    assert_exit(&empty, 0);
    assert_report(&empty, &[]);
}

#[test]
fn errors_and_debug_logs_use_stderr_only() {
    let project = Project::empty();
    let missing_config = project.run(["check"]);
    assert_exit(&missing_config, 2);
    assert!(stdout(&missing_config).is_empty());
    assert_eq!(
        stderr(&missing_config),
        "error: cannot find a config file in any of the default locations: [\"licenserc.toml\", \".licenserc.toml\"]\n"
    );

    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["binary.rs", "notes.txt"]

[git]
ignore = "disable"
"#,
    );
    project.write("binary.rs", [0xff]);
    project.write("notes.txt", "notes\n");

    let logged = project
        .command(["check", "--output-format=json"])
        .env("RUST_LOG", "hawkeye=debug")
        .output()
        .expect("run hawkeye with debug logs");
    assert_exit(&logged, 0);
    assert_report(
        &logged,
        &[("binary.rs", "unsupported"), ("notes.txt", "unsupported")],
    );
    let logs = stderr(&logged);
    assert!(logs.contains("binary.rs is not UTF-8 text; reporting it as unsupported"));
    assert!(logs.contains("notes.txt has no matching rule; reporting it as unsupported"));
}

#[test]
fn config_lookup_is_local_and_prefers_licenserc() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Parent"

[files]
includes = ["**/*.rs"]
"#,
    );
    project.write("child/source.rs", "fn child() {}\n");

    let implicit = project
        .command(["check"])
        .current_dir(project.path().join("child"))
        .output()
        .expect("run hawkeye from child directory");
    assert_exit(&implicit, 2);

    let configured = project
        .command(["--config", "../licenserc.toml", "format"])
        .current_dir(project.path().join("child"))
        .output()
        .expect("run hawkeye with explicit config");
    assert_exit(&configured, 0);
    assert_eq!(
        project.read("child/source.rs"),
        "// Copyright 2026 Parent\n\nfn child() {}\n"
    );

    let fallback = Project::empty();
    fs::create_dir(fallback.path().join("licenserc.toml"))
        .expect("create non-file primary candidate");
    fallback.write(
        ".licenserc.toml",
        r#"[header]
text = "Copyright 2026 Fallback"

[files]
includes = ["**/*.rs"]
"#,
    );
    fallback.write("fallback.rs", "fn fallback() {}\n");
    let formatted = fallback.run(["format"]);
    assert_exit(&formatted, 0);
    assert!(
        fallback
            .read("fallback.rs")
            .contains("Copyright 2026 Fallback")
    );
}

#[cfg(unix)]
#[test]
fn report_paths_preserve_unix_filename_characters() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]

[git]
ignore = "disable"
"#,
    );
    project.write(r"back\slash.rs", "fn main() {}\n");

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 1);
    assert_report(&checked, &[(r"back\slash.rs", "add")]);
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_report_paths_return_a_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
excludes = ["licenserc.toml"]
"#,
    );
    project.write(
        OsString::from_vec(b"source-\xff.rs".to_vec()),
        "fn main() {}\n",
    );

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 2);
    assert!(stdout(&checked).is_empty());
    assert!(stderr(&checked).contains("cannot serialize JSON report"));
}
