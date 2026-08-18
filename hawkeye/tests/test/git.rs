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

use std::ffi::OsString;
use std::fs;

use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::ErrorKind;
use hawkeye::FileOutcome;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::support::Project;
use super::support::assert_exit;
use super::support::assert_report;
use super::support::stderr;

#[test]
fn git_discovery_combines_tracked_files_and_ignore_sources() {
    let project = Project::from_case("git-ignore");
    project.git(["init", "-b", "main"]);
    project.git(["add", "-f", "tracked_ignored.rs"]);
    project.git(["add", "deleted.rs"]);
    project.write(".git/info/exclude", "info_ignored.rs\n");
    project.write(".git/global-excludes", "global_ignored.rs\n");
    let global_excludes = project.path().join(".git/global-excludes");
    project.git([
        OsString::from("config"),
        OsString::from("core.excludesFile"),
        global_excludes.into_os_string(),
    ]);
    fs::remove_file(project.path().join("deleted.rs")).expect("delete indexed source");

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[("tracked_ignored.rs", "add")]);
    assert!(
        project
            .read("tracked_ignored.rs")
            .contains("Copyright 2026 Acme Labs")
    );
    assert_eq!(project.read("untracked_ignored.rs"), "fn untracked() {}\n");
    assert_eq!(
        project.read("info_ignored.rs"),
        "fn ignored_by_info_exclude() {}\n"
    );
    assert_eq!(
        project.read("global_ignored.rs"),
        "fn ignored_by_configured_global_excludes() {}\n"
    );
    assert_eq!(project.read("excluded.rs"), "fn excluded() {}\n");
}

#[test]
fn git_discovery_respects_nested_file_roots() {
    let project = Project::from_case("nested-root");
    project.git(["init", "-b", "main"]);
    project.git(["add", "source/main.rs"]);

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[("main.rs", "add")]);
    assert_eq!(
        project.read("source/main.rs"),
        "// Copyright 2026 Nested Root\n\nfn main() {}\n"
    );
    assert_eq!(project.read("source/generated.rs"), "fn generated() {}\n");
    assert_eq!(project.read("source/ignored.rs"), "fn ignored() {}\n");
}

#[test]
fn git_history_tracks_branches_dirty_files_and_untracked_files() {
    let project = Project::from_case("git-history");
    setup_history(&project);
    let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();

    let formatted = project.run(["format", "--output-format=json"]);
    assert_exit(&formatted, 0);
    assert_report(&formatted, &[("app.rs", "add"), ("new/fresh.rs", "add")]);
    assert_eq!(
        project.read("app.rs"),
        format!(
            "// Copyright 2019-{current_year} Alice, Bob, Current User\n\nfn feature() {{}}\nfn dirty() {{}}\n"
        )
    );
    assert_eq!(
        project.read("new/fresh.rs"),
        format!("// Copyright {current_year}-{current_year} Current User\n\nfn untracked() {{}}\n")
    );
    assert_exit(&project.run(["check"]), 0);
}

#[test]
fn nested_roots_scope_worktree_status_for_git_attributes() {
    let project = Project::new();
    project.write("source/main.rs", "fn main() {}\n");
    project.write("outside.rs", "fn outside() {}\n");
    project.git(["init", "-b", "main"]);
    project.git(["config", "user.name", "Current User"]);
    project.git(["config", "user.email", "current@example.com"]);
    project.git(["add", "source/main.rs", "outside.rs"]);
    project.commit(
        "add sources",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );

    project.write("source/main.rs", "fn main() {}\nfn dirty() {}\n");
    project.write("outside.rs", "fn outside() {}\nfn dirty() {}\n");
    project.write("untracked.rs", "fn untracked() {}\n");
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright {{ attrs.git_file_created_year }}-{{ attrs.git_file_modified_year }} Acme"

[files]
root = "source"
includes = ["**/*.rs"]

[git]
file_attrs = "enable"
ignore = "enable"
"#,
    );

    let formatted = project.run(["format"]);
    assert_exit(&formatted, 0);
    let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
    assert_eq!(
        project.read("source/main.rs"),
        format!("// Copyright 2019-{current_year} Acme\n\nfn main() {{}}\nfn dirty() {{}}\n")
    );
}

#[test]
fn recreated_paths_keep_their_earliest_commit_year() {
    let project = Project::new();
    project.git(["init", "-b", "main"]);
    project.git(["config", "user.name", "Current User"]);
    project.git(["config", "user.email", "current@example.com"]);
    project.write("staged.rs", "fn staged() {}\n");
    project.write("untracked.rs", "fn untracked() {}\n");
    project.git(["add", "staged.rs", "untracked.rs"]);
    project.commit(
        "add sources",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    project.git(["rm", "staged.rs", "untracked.rs"]);
    project.commit(
        "remove sources",
        "Bob",
        "bob@example.com",
        "2020-06-01T12:00:00+0000",
    );
    project.write("staged.rs", "fn staged() {}\n");
    project.write("untracked.rs", "fn untracked() {}\n");
    project.git(["add", "staged.rs"]);
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright {{ attrs.git_file_created_year }}-{{ attrs.git_file_modified_year }} Acme"

[files]
includes = ["**/*.rs"]

[git]
file_attrs = "enable"
ignore = "disable"
"#,
    );

    assert_exit(&project.run(["format"]), 0);
    let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
    assert_eq!(
        project.read("staged.rs"),
        format!("// Copyright 2019-{current_year} Acme\n\nfn staged() {{}}\n")
    );
    assert_eq!(
        project.read("untracked.rs"),
        format!("// Copyright 2019-{current_year} Acme\n\nfn untracked() {{}}\n")
    );
}

#[cfg(unix)]
#[test]
fn git_history_accepts_control_characters_in_paths() {
    use std::os::unix::ffi::OsStringExt;

    let project = Project::new();
    let paths = [
        OsString::from_vec(b"\x1e2020.rs".to_vec()),
        OsString::from_vec(b"line\nbreak.rs".to_vec()),
    ];
    for path in &paths {
        project.write(path, "fn main() {}\n");
    }
    project.git(["init", "-b", "main"]);
    project.git(["add", "--all"]);
    project.commit(
        "add sources",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright {{ attrs.git_file_created_year }} Acme"

[files]
includes = ["**/*.rs"]

[git]
file_attrs = "enable"
ignore = "disable"
"#,
    );

    assert_exit(&project.run(["format"]), 0);
    for path in paths {
        assert_eq!(
            project.read(path),
            "// Copyright 2019 Acme\n\nfn main() {}\n"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn git_discovery_accepts_non_utf8_repository_roots() {
    use std::os::unix::ffi::OsStringExt;

    let project = Project::named(OsString::from_vec(b"repository-\xff".to_vec()));
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
    project.write("main.rs", "fn main() {}\n");
    project.git(["init", "-b", "main"]);

    let checked = project.run(["check", "--output-format=json"]);
    assert_exit(&checked, 1);
    assert_report(&checked, &[("main.rs", "add")]);
}

#[test]
fn shallow_history_is_required_only_for_supported_files() {
    let project = Project::new();
    project.write("main.rs", "fn main() {}\n");
    project.git(["init", "-b", "main"]);
    project.git(["config", "user.name", "Current User"]);
    project.git(["config", "user.email", "current@example.com"]);
    project.git(["add", "main.rs"]);
    project.commit(
        "add source",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    let head = project.git_stdout(["rev-parse", "HEAD"]);
    project.write(".git/shallow", format!("{head}\n"));

    project.write("licenserc.toml", attrs_config("enable"));
    let required = project.run(["check"]);
    assert_exit(&required, 2);
    assert!(stderr(&required).contains("repository is shallow"));
    assert_eq!(project.read("main.rs"), "fn main() {}\n");

    project.write("licenserc.toml", attrs_config("auto"));
    let automatic = project.run(["format"]);
    assert_exit(&automatic, 0);
    assert!(stderr(&automatic).contains("repository is shallow"));
    assert!(
        project
            .read("main.rs")
            .starts_with("// Copyright 2026 Acme\n\n")
    );

    project.write("notes.txt", "unsupported\n");
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.txt"]

[git]
file_attrs = "enable"
ignore = "disable"
"#,
    );
    let unsupported = project.run(["check", "--output-format=json"]);
    assert_exit(&unsupported, 0);
    assert_report(&unsupported, &[("notes.txt", "unsupported")]);
}

#[test]
fn gix_supports_sha256_repositories_without_a_git_executable() {
    let project = Project::new();
    project.git(["init", "--object-format=sha256", "-b", "main"]);
    project.write("main.rs", "fn main() {}\n");
    project.git(["add", "main.rs"]);
    project.commit(
        "add source",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright {{ attrs.git_file_created_year }} Acme"

[files]
includes = ["**/*.rs"]

[git]
file_attrs = "enable"
ignore = "enable"
"#,
    );

    let formatted = project
        .command(["format"])
        .env("PATH", "")
        .output()
        .expect("run hawkeye without Git in PATH");
    assert_exit(&formatted, 0);
    assert_eq!(
        project.read("main.rs"),
        "// Copyright 2019 Acme\n\nfn main() {}\n"
    );
}

#[test]
fn automatic_git_mode_falls_back_only_when_no_repository_exists() {
    let project = Project::new();
    project.write("main.rs", "fn main() {}\n");
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
    let config =
        Config::load(project.path().join("licenserc.toml")).expect("load required Git config");
    let engine = Engine::new(config).expect("build engine before discovering files");
    let err = engine
        .check()
        .expect_err("required Git discovery must reject a non-repository");
    assert_eq!(err.kind(), ErrorKind::Unsupported);

    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]

[git]
ignore = "auto"
"#,
    );
    let config =
        Config::load(project.path().join("licenserc.toml")).expect("load automatic Git config");
    let report = Engine::new(config)
        .expect("build automatic Git engine")
        .check()
        .expect("fall back to filesystem discovery");
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].outcome, FileOutcome::Add);

    project.git(["init", "-b", "main"]);
    project.write(".git/config", "[broken\n");
    let checked = project.run(["check"]);
    assert_exit(&checked, 2);
    assert!(stderr(&checked).contains("cannot open Git repository"));
}

fn setup_history(project: &Project) {
    project.git(["init", "-b", "main"]);
    project.git(["config", "user.name", "Current User"]);
    project.git(["config", "user.email", "current@example.com"]);

    project.write("app.rs", "fn initial() {}\n");
    project.git(["add", "app.rs"]);
    project.commit(
        "initial",
        "Alice",
        "alice@example.com",
        "2019-06-01T12:00:00+0000",
    );

    project.git(["switch", "-c", "feature"]);
    project.write("app.rs", "fn feature() {}\n");
    project.git(["add", "app.rs"]);
    project.commit(
        "feature",
        "Bob",
        "bob@example.com",
        "2020-06-01T12:00:00+0000",
    );

    project.git(["switch", "main"]);
    project.write("other.rs", "fn other() {}\n");
    project.git(["add", "other.rs"]);
    project.commit(
        "other",
        "Carol",
        "carol@example.com",
        "2021-06-01T12:00:00+0000",
    );
    project.git(["merge", "--no-ff", "feature", "-m", "merge feature"]);

    project.write("app.rs", "fn feature() {}\nfn dirty() {}\n");
    project.write("new/fresh.rs", "fn untracked() {}\n");
}

fn attrs_config(mode: &str) -> String {
    format!(
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.rs"]

[git]
file_attrs = "{mode}"
ignore = "disable"
"#
    )
}
