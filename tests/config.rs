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

use std::path::Path;

use hawkeye::config::Config;
use hawkeye::config::ConfigError;
use hawkeye::config::HeaderSource;
use hawkeye::config::Style;

#[test]
fn minimal_config_has_explicit_stable_defaults() {
    let config = Config::from_toml(
        r#"
schema_version = 1

[header]
builtin = "apache_2_0"
"#,
    )
    .expect("minimal config should be valid");

    assert_eq!(config.schema_version(), 1);
    assert_eq!(
        config.header().source(),
        &HeaderSource::Builtin("apache_2_0".to_owned())
    );
    assert_eq!(config.header().required_terms(), ["copyright"]);
    assert_eq!(config.files().root(), Path::new("."));
    assert!(config.files().includes().is_empty());
    assert!(config.files().excludes().is_empty());
    assert!(config.files().use_default_excludes());
    assert!(config.files().use_default_rules());
    assert!(config.git().respect_ignore());
    assert!(!config.git().file_dates());
}

#[test]
fn unknown_and_camel_case_fields_are_rejected() {
    let error = Config::from_toml(
        r#"
schema_version = 1

[header]
builtin = "apache_2_0"
requiredTerms = ["copyright"]
"#,
    )
    .expect_err("camel-case fields must not be accepted as aliases");

    let ConfigError::Parse(error) = error else {
        panic!("unknown fields should be rejected while parsing");
    };
    let message = error.to_string();
    assert!(message.contains("requiredTerms"), "{message}");
    assert!(message.contains("required_terms"), "{message}");
}

#[test]
fn header_source_is_an_exclusive_choice() {
    let error = Config::from_toml(
        r#"
schema_version = 1

[header]
path = "license.txt"
text = "Copyright {{ variables.year }}"
"#,
    )
    .expect_err("two sources must be rejected");

    let ConfigError::Validation(errors) = error else {
        panic!("source cardinality is a semantic invariant");
    };
    assert_eq!(errors.issues().len(), 1);
    assert_eq!(errors.issues()[0].path(), "header");
}

#[test]
fn advanced_config_preserves_rule_order_and_typed_variables() {
    let config = Config::from_toml(
        r#"
schema_version = 1

[header]
text = "Copyright {{ variables.project.start_year }} {{ variables.owner }}"
required_terms = ["Copyright", "FastLabs"]

[files]
root = "workspace"
includes = ["**/*.rs", "**/*.xml"]
excludes = ["fixtures/**"]
use_default_excludes = false
use_default_rules = true

[variables]
owner = "FastLabs Developers"
enabled = true

[variables.project]
start_year = 2026
tags = ["rust", "tooling"]

[git]
respect_ignore = false
file_dates = true

[styles.xml_line]
kind = "line"
prefix = "<!-- "
suffix = " -->"
align_suffix = true

[styles.swift_banner]
kind = "block"
start = "//===----------------------------------------------------------------------===//"
prefix = "// "
end = "//===----------------------------------------------------------------------===//"

[[rules]]
patterns = ["**/*.xml"]
write_style = "xml_line"

[[rules]]
patterns = ["**/*.swift"]
write_style = "swift_banner"
recognize_styles = ["slash_line", "slash_block"]
"#,
    )
    .expect("advanced config should be valid");

    assert_eq!(config.rules().len(), 2);
    assert_eq!(config.rules()[0].write_style().as_str(), "xml_line");
    assert_eq!(config.rules()[1].write_style().as_str(), "swift_banner");
    assert_eq!(
        config.rules()[1]
            .recognize_styles()
            .iter()
            .map(|style| style.as_str())
            .collect::<Vec<_>>(),
        ["slash_line", "slash_block"]
    );
    assert_eq!(config.variables()["enabled"].as_bool(), Some(true));
    assert_eq!(
        config.variables()["project"]["start_year"].as_integer(),
        Some(2026)
    );

    let Style::Line(xml) = &config.styles()["xml_line"] else {
        panic!("xml_line should retain its tagged variant");
    };
    assert_eq!(xml.prefix(), "<!-- ");
    assert!(xml.align_suffix());
}

#[test]
fn semantic_validation_reports_independent_issues_together() {
    let error = Config::from_toml(
        r#"
schema_version = 9

[header]
builtin = "Apache-2.0"
required_terms = []

[variables]
copyrightOwner = "FastLabs Developers"

[styles.bad_style]
kind = "line"

[[rules]]
patterns = []
write_style = "SLASH_LINE"
"#,
    )
    .expect_err("independent semantic issues must be reported together");

    let ConfigError::Validation(errors) = error else {
        panic!("the TOML shape itself is valid");
    };
    let paths = errors
        .issues()
        .iter()
        .map(|issue| issue.path())
        .collect::<Vec<_>>();

    assert!(paths.contains(&"schema_version"));
    assert!(paths.contains(&"header.builtin"));
    assert!(paths.contains(&"header.required_terms"));
    assert!(paths.contains(&"variables.copyrightOwner"));
    assert!(paths.contains(&"styles.bad_style"));
    assert!(paths.contains(&"rules[0].patterns"));
    assert!(paths.contains(&"rules[0].write_style"));
}

#[test]
fn style_layout_and_recognition_ambiguities_are_rejected() {
    let error = Config::from_toml(
        r#"
schema_version = 1

[header]
builtin = "apache_2_0"

[styles.xml_line]
kind = "line"
prefix = "<!--\n"
align_suffix = true

[[rules]]
patterns = ["**/*.xml"]
write_style = "xml_line"
recognize_styles = ["xml_line", "slash_block", "slash_block"]
"#,
    )
    .expect_err("ambiguous style declarations must be rejected");

    let ConfigError::Validation(errors) = error else {
        panic!("the TOML shape itself is valid");
    };
    let paths = errors
        .issues()
        .iter()
        .map(|issue| issue.path())
        .collect::<Vec<_>>();

    assert!(paths.contains(&"styles.xml_line.prefix"));
    assert!(paths.contains(&"styles.xml_line.align_suffix"));
    assert!(paths.contains(&"rules[0].recognize_styles[0]"));
    assert!(paths.contains(&"rules[0].recognize_styles[2]"));
}
