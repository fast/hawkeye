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

//! The strict `licenserc.toml` model.
//!
//! [`Config`] contains every value represented directly by TOML. A later
//! resolution step turns it into a filesystem-ready `ResolvedConfig`; there is
//! intentionally no separately named raw configuration layer.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use thiserror::Error;

/// The configuration filename used by the command-line tool.
pub const DEFAULT_CONFIG_FILE: &str = "licenserc.toml";

/// A parsed and locally validated `licenserc.toml` document.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    header: HeaderConfig,
    #[serde(default)]
    files: FilesConfig,
    #[serde(default)]
    props: BTreeMap<String, toml::Value>,
    #[serde(default)]
    git: GitConfig,
    #[serde(default)]
    styles: BTreeMap<String, StyleConfig>,
    #[serde(default)]
    rules: Vec<RuleConfig>,
}

impl Config {
    /// Parses strict snake-case TOML and validates local invariants.
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let config = toml::from_str::<Self>(source)?;
        let issues = validate(&config);
        if issues.is_empty() {
            Ok(config)
        } else {
            Err(ConfigError::Validation(ValidationErrors { issues }))
        }
    }

    /// Returns the configured header source and recognition keywords.
    pub fn header(&self) -> &HeaderConfig {
        &self.header
    }

    /// Returns file discovery settings.
    pub fn files(&self) -> &FilesConfig {
        &self.files
    }

    /// Returns user values passed to MiniJinja as the `props` object.
    pub fn props(&self) -> &BTreeMap<String, toml::Value> {
        &self.props
    }

    /// Returns Git integration settings.
    pub fn git(&self) -> GitConfig {
        self.git
    }

    /// Returns custom style definitions.
    pub fn styles(&self) -> &BTreeMap<String, StyleConfig> {
        &self.styles
    }

    /// Returns user rules in declaration order.
    pub fn rules(&self) -> &[RuleConfig] {
        &self.rules
    }
}

impl FromStr for Config {
    type Err = ConfigError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::from_toml(source)
    }
}

/// The header template source and the words used to recognize an old header.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderConfig {
    builtin: Option<String>,
    path: Option<PathBuf>,
    text: Option<String>,
    #[serde(default = "default_keywords")]
    keywords: Vec<String>,
}

impl HeaderConfig {
    /// Returns the built-in resource key, if selected.
    pub fn builtin(&self) -> Option<&str> {
        self.builtin.as_deref()
    }

    /// Returns the configured template path, if selected.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns the inline template, if selected.
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Returns words that must all occur in a structurally recognized header.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// File discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesConfig {
    root: PathBuf,
    includes: Vec<String>,
    excludes: Vec<String>,
}

impl Default for FilesConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            includes: Vec::new(),
            excludes: Vec::new(),
        }
    }
}

impl FilesConfig {
    /// Returns the root scanned by HawkEye, relative to `licenserc.toml`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns Git-ignore-style inclusion patterns; an empty list selects all files.
    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    /// Returns Git-ignore-style exclusion patterns.
    pub fn excludes(&self) -> &[String] {
        &self.excludes
    }
}

/// Whether a Git-backed capability is disabled, opportunistic, or required.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureMode {
    /// Never use the capability.
    Disable,
    /// Use the capability when a Git repository is available.
    #[default]
    Auto,
    /// Require the capability and fail when it cannot be initialized.
    Enable,
}

/// Git integration settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    ignore: FeatureMode,
    file_attrs: FeatureMode,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            ignore: FeatureMode::Auto,
            file_attrs: FeatureMode::Disable,
        }
    }
}

impl GitConfig {
    /// Returns how Git ignore files participate in discovery.
    pub fn ignore(self) -> FeatureMode {
        self.ignore
    }

    /// Returns how per-file Git attributes are populated for templates.
    pub fn file_attrs(self) -> FeatureMode {
        self.file_attrs
    }
}

/// A filename/extension rule and its accepted/output comment styles.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    filenames: Vec<String>,
    style_out: String,
    #[serde(default)]
    styles_in: Vec<String>,
}

impl RuleConfig {
    /// Returns suffixes matched after the final filename separator, without a leading dot.
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// Returns complete filenames matched case-insensitively.
    pub fn filenames(&self) -> &[String] {
        &self.filenames
    }

    /// Returns the canonical style used for output.
    pub fn style_out(&self) -> &str {
        &self.style_out
    }

    /// Returns additional styles accepted as structurally safe input.
    pub fn styles_in(&self) -> &[String] {
        &self.styles_in
    }
}

/// A syntax-only custom comment style.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
pub enum StyleConfig {
    /// Every logical header line is wrapped independently.
    Line {
        /// Text written before every logical line.
        #[serde(default)]
        prefix: String,
        /// Text written after every logical line.
        #[serde(default)]
        suffix: String,
        /// Right-pad logical lines so all suffixes align.
        #[serde(default)]
        pad_lines: bool,
    },
    /// One opening and closing delimiter encloses all logical lines.
    Block {
        /// Opening delimiter written on its own line.
        start: String,
        /// Text written before every enclosed logical line.
        #[serde(default)]
        prefix: String,
        /// Text written after every enclosed logical line.
        #[serde(default)]
        suffix: String,
        /// Closing delimiter written on its own line.
        end: String,
    },
}

/// An error produced while parsing `licenserc.toml`.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The TOML is malformed or does not match the closed schema.
    #[error("cannot parse licenserc.toml: {0}")]
    Parse(#[from] toml::de::Error),

    /// The TOML shape is valid but one or more values are invalid.
    #[error(transparent)]
    Validation(ValidationErrors),
}

/// All local semantic errors found in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors {
    issues: Vec<ValidationIssue>,
}

impl ValidationErrors {
    /// Returns validation errors in deterministic traversal order.
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "licenserc.toml has {} validation error{}",
            self.issues.len(),
            if self.issues.len() == 1 { "" } else { "s" }
        )?;
        for issue in &self.issues {
            write!(formatter, "\n- {}: {}", issue.path, issue.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// One local semantic configuration error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    path: String,
    message: String,
}

impl ValidationIssue {
    /// Returns the dotted/indexed location of the invalid value.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the reason the value is invalid.
    pub fn message(&self) -> &str {
        &self.message
    }
}

fn default_keywords() -> Vec<String> {
    vec!["copyright".to_owned()]
}

fn validate(config: &Config) -> Vec<ValidationIssue> {
    let mut validator = Validator::default();
    validator.header(&config.header);
    validator.files(&config.files);
    validator.styles(&config.styles);
    validator.rules(&config.rules);
    validator.issues
}

#[derive(Default)]
struct Validator {
    issues: Vec<ValidationIssue>,
}

impl Validator {
    fn issue(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues.push(ValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn header(&mut self, header: &HeaderConfig) {
        let source_count = usize::from(header.builtin.is_some())
            + usize::from(header.path.is_some())
            + usize::from(header.text.is_some());
        if source_count != 1 {
            self.issue(
                "header",
                "exactly one of `builtin`, `path`, or `text` must be set",
            );
        }

        if let Some(value) = &header.builtin {
            self.non_blank("header.builtin", value, "built-in header key");
            self.no_nul("header.builtin", value);
        }
        if let Some(value) = &header.path
            && value.as_os_str().is_empty()
        {
            self.issue("header.path", "header path must not be empty");
        }
        if let Some(value) = &header.text {
            self.non_blank("header.text", value, "inline header template");
            self.no_nul("header.text", value);
        }

        if header.keywords.is_empty() {
            self.issue(
                "header.keywords",
                "at least one keyword is required to distinguish a header from an ordinary comment",
            );
        }
        let mut seen = HashMap::<String, usize>::new();
        for (index, keyword) in header.keywords.iter().enumerate() {
            let path = format!("header.keywords[{index}]");
            self.non_blank(&path, keyword, "keyword");
            let folded = keyword.to_lowercase();
            if let Some(first) = seen.get(&folded) {
                self.issue(
                    path,
                    format!("duplicates `header.keywords[{first}]` case-insensitively"),
                );
            } else {
                seen.insert(folded, index);
            }
        }
    }

    fn files(&mut self, files: &FilesConfig) {
        if files.root.as_os_str().is_empty() {
            self.issue("files.root", "file root must not be empty");
        }
        self.patterns("files.includes", &files.includes);
        self.patterns("files.excludes", &files.excludes);
    }

    fn patterns(&mut self, path: &str, patterns: &[String]) {
        for (index, pattern) in patterns.iter().enumerate() {
            let path = format!("{path}[{index}]");
            self.non_blank(&path, pattern, "pattern");
            self.no_nul(&path, pattern);
            if pattern.starts_with('!') {
                self.issue(
                    path,
                    "negation is not accepted because includes and excludes are separate lists",
                );
            }
        }
    }

    fn styles(&mut self, styles: &BTreeMap<String, StyleConfig>) {
        for (name, style) in styles {
            let path = format!("styles.{name}");
            self.non_blank(&path, name, "style key");
            self.no_nul(&path, name);
            match style {
                StyleConfig::Line {
                    prefix,
                    suffix,
                    pad_lines,
                } => {
                    self.token(&format!("{path}.prefix"), prefix);
                    self.token(&format!("{path}.suffix"), suffix);
                    if prefix.trim().is_empty() && suffix.trim().is_empty() {
                        self.issue(&path, "line style needs a non-whitespace prefix or suffix");
                    }
                    if *pad_lines && suffix.is_empty() {
                        self.issue(
                            format!("{path}.pad_lines"),
                            "line padding requires a non-empty suffix",
                        );
                    }
                }
                StyleConfig::Block {
                    start,
                    prefix,
                    suffix,
                    end,
                } => {
                    self.token(&format!("{path}.start"), start);
                    self.token(&format!("{path}.prefix"), prefix);
                    self.token(&format!("{path}.suffix"), suffix);
                    self.token(&format!("{path}.end"), end);
                    if start.trim().is_empty() {
                        self.issue(format!("{path}.start"), "block start must not be blank");
                    }
                    if end.trim().is_empty() {
                        self.issue(format!("{path}.end"), "block end must not be blank");
                    }
                }
            }
        }
    }

    fn rules(&mut self, rules: &[RuleConfig]) {
        for (index, rule) in rules.iter().enumerate() {
            let path = format!("rules[{index}]");
            if rule.extensions.is_empty() && rule.filenames.is_empty() {
                self.issue(
                    &path,
                    "at least one extension or filename must be configured",
                );
            }

            for (item, extension) in rule.extensions.iter().enumerate() {
                let item_path = format!("{path}.extensions[{item}]");
                self.non_blank(&item_path, extension, "extension");
                if extension.starts_with('.') {
                    self.issue(&item_path, "extension must not start with `.`");
                }
                if extension.contains(['/', '\\']) {
                    self.issue(&item_path, "extension must not contain a path separator");
                }
            }
            for (item, filename) in rule.filenames.iter().enumerate() {
                let item_path = format!("{path}.filenames[{item}]");
                self.non_blank(&item_path, filename, "filename");
                if filename.contains(['/', '\\']) {
                    self.issue(&item_path, "filename must not contain a path separator");
                }
            }

            self.non_blank(
                &format!("{path}.style_out"),
                &rule.style_out,
                "output style",
            );
            for (item, style) in rule.styles_in.iter().enumerate() {
                self.non_blank(&format!("{path}.styles_in[{item}]"), style, "input style");
            }
        }
    }

    fn non_blank(&mut self, path: &str, value: &str, kind: &str) {
        if value.trim().is_empty() {
            self.issue(path, format!("{kind} must not be empty"));
        }
    }

    fn no_nul(&mut self, path: &str, value: &str) {
        if value.contains('\0') {
            self.issue(path, "must not contain a NUL byte");
        }
    }

    fn token(&mut self, path: &str, value: &str) {
        self.no_nul(path, value);
        if value.contains(['\r', '\n']) {
            self.issue(path, "style token must not contain a line ending");
        }
    }
}
