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

use hawkeye::Config;
use hawkeye::Engine;
use hawkeye::ErrorKind;
use hawkeye::FileOutcome;
use hawkeye::Scope;
use tempfile::TempDir;

fn project() -> (TempDir, Config) {
    let directory = tempfile::tempdir().expect("create project");
    let path = directory.path().join("licenserc.toml");
    fs::write(
        &path,
        "[header]\ntext = 'Copyright Acme'\n[files]\nincludes = ['*.rs']\n[git]\nignore = 'disable'\n",
    )
    .unwrap();
    let config = Config::load(path).expect("load config");
    (directory, config)
}

#[test]
fn plans_can_be_inspected_discarded_and_applied() {
    let (directory, config) = project();
    let path = directory.path().join("source.rs");
    let source = "fn main() {}\n";
    fs::write(&path, source).unwrap();
    let engine = Engine::new(config).unwrap();
    let checked = engine.check(Scope::All).unwrap();
    assert_eq!(checked.files[0].outcome, FileOutcome::Add);

    let discarded = engine.format(Scope::All).unwrap().into_report();
    assert_eq!(discarded, checked);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);

    let edits = engine.format(Scope::All).unwrap();
    assert_eq!(edits.report(), &checked);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(edits.apply().unwrap(), checked);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "// Copyright Acme\n\nfn main() {}\n"
    );
    assert_eq!(
        engine.check(Scope::All).unwrap().files[0].outcome,
        FileOutcome::Clean
    );

    let edits = engine.remove(Scope::All).unwrap();
    let report = edits.report().clone();
    assert_eq!(report.files[0].outcome, FileOutcome::Remove);
    assert_eq!(edits.apply().unwrap(), report);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(
        engine.remove(Scope::All).unwrap().into_report().files[0].outcome,
        FileOutcome::Clean
    );
}

#[test]
fn scopes_are_root_relative_and_deduplicate_overlapping_paths() {
    let (directory, config) = project();
    fs::create_dir(directory.path().join("nested")).unwrap();
    fs::write(directory.path().join("nested/a.rs"), "fn a() {}\n").unwrap();
    fs::write(directory.path().join("b.rs"), "fn b() {}\n").unwrap();
    let engine = Engine::new(config).unwrap();
    assert_eq!(engine.check(Scope::All).unwrap().files.len(), 2);
    let selected = engine
        .check(Scope::Paths(&[
            "nested".into(),
            "nested/a.rs".into(),
            directory.path().join("nested/a.rs"),
        ]))
        .unwrap();
    assert_eq!(selected.files.len(), 1);
    assert_eq!(selected.files[0].path, std::path::Path::new("nested/a.rs"));
    assert!(engine.check(Scope::Paths(&[])).unwrap().files.is_empty());
    assert!(
        engine
            .format(Scope::Paths(&[]))
            .unwrap()
            .apply()
            .unwrap()
            .files
            .is_empty()
    );
    assert!(
        engine
            .remove(Scope::Paths(&[]))
            .unwrap()
            .apply()
            .unwrap()
            .files
            .is_empty()
    );
}

#[test]
fn engines_validate_programmatically_modified_configuration() {
    let (_directory, mut config) = project();
    config.header.text = None;
    config.header.keywords.clear();
    let validation = config.validate().expect_err("validate changed config");
    let initialization = Engine::new(config)
        .err()
        .expect("engine must validate config");
    assert_eq!(validation.kind(), ErrorKind::ConfigInvalid);
    assert_eq!(validation.to_string(), initialization.to_string());
    assert!(
        validation
            .to_string()
            .contains("exactly one of `builtin`, `path`, or `text`")
    );
    assert!(
        validation
            .to_string()
            .contains("header.keywords: at least one keyword")
    );

    for source in [
        "[header]",
        "[header]\nbuiltin = 'SPDX'\ntext = 'Copyright Acme'",
    ] {
        let config: Config = toml::from_str(source).expect("deserialize header fields");
        assert_eq!(
            config.validate().unwrap_err().kind(),
            ErrorKind::ConfigInvalid
        );
    }
}

#[test]
fn invalid_file_patterns_fail_when_building_the_engine() {
    let (_directory, mut config) = project();
    config.files.includes = vec!["[".into()];
    config
        .validate()
        .expect("field validation does not compile patterns");
    let error = Engine::new(config)
        .err()
        .expect("reject an invalid pattern");
    assert_eq!(error.kind(), ErrorKind::ConfigInvalid);
    assert!(
        error
            .to_string()
            .contains("invalid files.includes or files.excludes pattern")
    );
}

#[test]
fn a_later_template_error_does_not_modify_earlier_sources() {
    let (directory, mut config) = project();
    config.header.text = Some(
        "Copyright {% if attrs.filename == 'b.rs' %}{{ props.missing }}{% else %}Acme{% endif %}"
            .into(),
    );
    for name in ["a.rs", "b.rs"] {
        fs::write(directory.path().join(name), "fn main() {}\n").unwrap();
    }
    let engine = Engine::new(config).unwrap();
    let error = engine
        .format(Scope::All)
        .err()
        .expect("reject incomplete template props");
    assert_eq!(error.kind(), ErrorKind::ConfigInvalid);
    assert!(error.to_string().contains("props.missing"));
    for name in ["a.rs", "b.rs"] {
        assert_eq!(
            fs::read_to_string(directory.path().join(name)).unwrap(),
            "fn main() {}\n"
        );
    }
}

#[test]
fn changed_sources_are_preserved_and_stop_further_edits() {
    let (directory, config) = project();
    let first = directory.path().join("a.rs");
    let second = directory.path().join("b.rs");
    let third = directory.path().join("c.rs");
    let source = "// Copyright Old\n\nfn main() {}\n";
    let engine = Engine::new(config).unwrap();
    for changed in [b"".as_slice(), "\u{1f600}".as_bytes(), b"\xff"] {
        fs::write(&first, source).unwrap();
        fs::write(&second, source).unwrap();
        fs::write(&third, source).unwrap();
        let edits = engine.format(Scope::All).unwrap();
        fs::write(&second, changed).unwrap();
        let error = edits.apply().expect_err("reject a changed source");
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(error.to_string().contains("b.rs"));
        assert_eq!(
            fs::read_to_string(&first).unwrap(),
            "// Copyright Acme\n\nfn main() {}\n"
        );
        assert_eq!(fs::read(&second).unwrap(), changed);
        assert_eq!(fs::read_to_string(&third).unwrap(), source);
    }
}

#[test]
fn missing_sources_stop_further_edits_without_rolling_back() {
    let (directory, config) = project();
    let first = directory.path().join("a.rs");
    let second = directory.path().join("b.rs");
    let third = directory.path().join("c.rs");
    let source = "fn main() {}\n";
    fs::write(&first, source).unwrap();
    fs::write(&second, source).unwrap();
    fs::write(&third, source).unwrap();
    let edits = Engine::new(config).unwrap().format(Scope::All).unwrap();
    fs::remove_file(&second).unwrap();
    let error = edits.apply().expect_err("reject a missing source");
    assert!(error.to_string().contains("b.rs"));
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "// Copyright Acme\n\nfn main() {}\n"
    );
    assert!(!second.exists());
    assert_eq!(fs::read_to_string(&third).unwrap(), source);
}

#[test]
fn selected_hard_links_share_one_header_lifecycle() {
    let (directory, config) = project();
    let first = directory.path().join("a.rs");
    let second = directory.path().join("b.rs");
    let source = "fn main() {}\n";
    fs::write(&first, source).unwrap();
    fs::hard_link(&first, &second).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.rs", directory.path().join("c.rs")).unwrap();
    let engine = Engine::new(config).unwrap();
    engine.format(Scope::All).unwrap().apply().unwrap();
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "// Copyright Acme\n\nfn main() {}\n"
    );
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    assert!(
        engine
            .format(Scope::All)
            .unwrap()
            .into_report()
            .files
            .iter()
            .all(|file| file.outcome == FileOutcome::Clean)
    );
    engine.remove(Scope::All).unwrap().apply().unwrap();
    assert_eq!(fs::read_to_string(&first).unwrap(), source);
    assert_eq!(fs::read_to_string(&second).unwrap(), source);
}

#[test]
fn incompatible_alias_edits_return_an_error_without_stacking_headers() {
    let (directory, mut config) = project();
    config.header.text = Some("Copyright {{ attrs.filename }}".into());
    let first = directory.path().join("a.rs");
    let second = directory.path().join("b.rs");
    fs::write(&first, "fn main() {}\n").unwrap();
    fs::hard_link(&first, &second).unwrap();
    let engine = Engine::new(config).unwrap();
    let error = engine
        .format(Scope::All)
        .unwrap()
        .apply()
        .expect_err("aliases require different headers");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.to_string().contains("b.rs"));
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "// Copyright a.rs\n\nfn main() {}\n"
    );
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
}

#[test]
fn deserialized_keywords_are_resolved_and_nonempty() {
    for (source, expected) in [
        ("builtin = \"SPDX\"", vec!["SPDX-"]),
        ("builtin = \"Apache-2.0\"", vec!["copyright"]),
        ("text = \"Copyright Acme\"", vec!["copyright"]),
        (
            "builtin = \"SPDX\"\nkeywords = [\"SPDX-License-Identifier\", \"MIT\"]",
            vec!["SPDX-License-Identifier", "MIT"],
        ),
    ] {
        let config: Config = toml::from_str(&format!("[header]\n{source}\n"))
            .expect("deserialize and resolve keywords");
        assert_eq!(config.header.keywords, expected);
    }

    let error = toml::from_str::<Config>("[header]\nbuiltin = \"SPDX\"\nkeywords = []\n")
        .expect_err("an explicit empty list must fail during deserialization");
    assert!(
        error
            .to_string()
            .contains("header.keywords: at least one keyword")
    );
}
