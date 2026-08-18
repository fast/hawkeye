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

use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::ErrorKind;
use test_integration::Project;
use test_integration::assert_exit;
use test_integration::stderr;

#[test]
fn public_validation_matches_engine_initialization() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"
"#,
    );

    let mut config =
        Config::load(project.path().join("licenserc.toml")).expect("load valid config");
    config.header.text = None;

    let validation = config
        .validate()
        .expect_err("mutated config must fail public validation");
    let initialization = match Engine::new(config) {
        Ok(_) => panic!("engine must validate a mutated config"),
        Err(err) => err,
    };
    assert_eq!(validation.kind(), ErrorKind::ConfigInvalid);
    assert_eq!(validation.to_string(), initialization.to_string());
    assert!(
        validation
            .to_string()
            .contains("exactly one of `builtin`, `path`, or `text` must be set")
    );

    for (name, source) in [
        ("missing.toml", "[header]\n"),
        (
            "multiple.toml",
            "[header]\nbuiltin = \"Apache-2.0\"\ntext = \"Copyright 2026 Acme\"\n",
        ),
    ] {
        project.write(name, source);
        let config = Config::load(project.path().join(name)).expect("deserialize header fields");
        let err = config
            .validate()
            .expect_err("header source must be exactly one value");
        assert_eq!(err.kind(), ErrorKind::ConfigInvalid);
    }
}

#[test]
fn discovery_patterns_are_checked_when_the_engine_is_built() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["["]
"#,
    );

    let config = Config::load(project.path().join("licenserc.toml"))
        .expect("deserialize config before resolving patterns");
    let err = match Engine::new(config) {
        Ok(_) => panic!("invalid pattern must fail engine initialization"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::ConfigInvalid);
    assert!(
        err.to_string()
            .contains("invalid files.includes or files.excludes pattern")
    );
}

#[test]
fn rules_use_first_match_and_user_rules_override_builtins() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.widget", "**/*.rs"]

[[rules]]
extensions = ["WIDGET"]
style_out = "script"

[[rules]]
extensions = ["widget"]
style_out = "doubleslash"

[[rules]]
extensions = ["rs"]
style_out = "script"
"#,
    );
    project.write("example.widget", "content\n");
    project.write("example.rs", "fn main() {}\n");

    let formatted = project.run(["format"]);
    assert_exit(&formatted, 0);
    assert!(
        project
            .read("example.widget")
            .starts_with("# Copyright 2026 Acme\n\n"),
        "the first matching user rule must win"
    );
    assert!(
        project
            .read("example.rs")
            .starts_with("# Copyright 2026 Acme\n\n"),
        "a user rule must win over a built-in rule"
    );
}

#[test]
fn input_styles_default_validate_and_deduplicate() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.widget"]

[[rules]]
extensions = ["widget"]
style_out = "script"
"#,
    );
    project.write("example.widget", "content\n");
    let formatted = project.run(["format"]);
    assert_exit(&formatted, 0);
    assert_exit(&project.run(["check"]), 0);

    project.write(
        "incomplete.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[[rules]]
extensions = ["widget"]
style_out = "doubleslash"
styles_in = ["slashstar"]
"#,
    );
    let config =
        Config::load(project.path().join("incomplete.toml")).expect("deserialize incomplete rule");
    let err = config
        .validate()
        .expect_err("explicit input styles must include the output style");
    assert!(
        err.to_string()
            .contains("rules[0].styles_in: must include `style_out`")
    );

    project.write(
        "licenserc.toml",
        r#"[header]
text = "Copyright 2026 Acme"

[files]
includes = ["**/*.widget"]

[[rules]]
extensions = ["widget"]
style_out = "doubleslash"
styles_in = ["doubleslash", "slashstar", "slashstar"]
"#,
    );
    project.write("example.widget", "content\n");
    let formatted = project.run(["format"]);
    assert_exit(&formatted, 0);
    assert!(
        stderr(&formatted)
            .contains("rules[0].styles_in contains duplicate style \"slashstar\"; ignoring it")
    );
}

#[test]
fn custom_styles_can_override_builtins_with_a_warning() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r##"[header]
text = "Copyright 2026 Acme"

[styles.doubleslash]
kind = "line"
prefix = "# "
"##,
    );
    project.write("main.rs", "fn main() {}\n");

    let formatted = project.run(["format"]);
    assert_exit(&formatted, 0);
    assert!(
        stderr(&formatted)
            .contains("custom style \"doubleslash\" overrides a built-in style of the same name")
    );
    assert_eq!(
        project.read("main.rs"),
        "# Copyright 2026 Acme\n\nfn main() {}\n"
    );
}

#[test]
fn rendered_headers_must_contain_every_recognition_keyword() {
    let project = Project::empty();
    project.write(
        "licenserc.toml",
        r#"[header]
text = "Confidential Siemens 2026"

[files]
includes = ["**/*.rs"]
"#,
    );
    project.write("main.rs", "fn main() {}\n");

    let formatted = project.run(["format"]);
    assert_exit(&formatted, 2);
    assert!(stderr(&formatted).contains(
        "header template output for \"main.rs\" does not contain recognition keyword \"copyright\""
    ));
    assert_eq!(project.read("main.rs"), "fn main() {}\n");
}

#[test]
fn parse_errors_name_the_selected_config_file() {
    let project = Project::empty();
    project.write(
        "bad.toml",
        r#"[header]
text = "Copyright"

[files]
useDefaultRules = true
"#,
    );

    let checked = project.run(["--config", "bad.toml", "check"]);
    assert_exit(&checked, 2);
    let diagnostic = stderr(&checked);
    assert!(diagnostic.contains("cannot parse config file bad.toml"));
    assert!(diagnostic.contains("useDefaultRules"));
    assert!(!diagnostic.contains("cannot parse licenserc.toml"));
}
