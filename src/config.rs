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

//! HawkEye's versioned configuration model.
//!
//! Parsing is deliberately split from semantic validation. Serde rejects
//! malformed TOML and unknown fields, then HawkEye collects all semantic
//! issues it can find before constructing [`Config`]. Filesystem-dependent
//! work, such as resolving relative paths and loading header templates, belongs
//! to a later resolution phase.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use thiserror::Error;

/// The configuration schema understood by this release.
pub const SCHEMA_VERSION: u32 = 1;

/// A parsed and semantically valid HawkEye configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    schema_version: u32,
    header: HeaderConfig,
    files: FilesConfig,
    variables: BTreeMap<String, toml::Value>,
    git: GitConfig,
    rules: Vec<Rule>,
    styles: BTreeMap<StyleName, Style>,
}

impl Config {
    /// Parses TOML and validates the resulting configuration.
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let raw = toml::from_str::<RawConfig>(source)?;
        let issues = validate(&raw);

        if issues.is_empty() {
            Ok(raw.into())
        } else {
            Err(ConfigError::Validation(ValidationErrors { issues }))
        }
    }

    /// Returns the schema version declared by the configuration.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the header template and matching anchors.
    pub fn header(&self) -> &HeaderConfig {
        &self.header
    }

    /// Returns file discovery settings.
    pub fn files(&self) -> &FilesConfig {
        &self.files
    }

    /// Returns static, typed template variables in deterministic key order.
    pub fn variables(&self) -> &BTreeMap<String, toml::Value> {
        &self.variables
    }

    /// Returns Git integration settings.
    pub fn git(&self) -> &GitConfig {
        &self.git
    }

    /// Returns user rules in declaration order.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Returns custom style definitions in deterministic name order.
    pub fn styles(&self) -> &BTreeMap<StyleName, Style> {
        &self.styles
    }
}

impl FromStr for Config {
    type Err = ConfigError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::from_toml(source)
    }
}

/// An error produced while reading the in-memory configuration document.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The TOML shape is malformed or contains an unknown field.
    #[error("failed to parse HawkEye configuration: {0}")]
    Parse(#[from] toml::de::Error),

    /// The TOML shape is valid but its values violate configuration invariants.
    #[error(transparent)]
    Validation(ValidationErrors),
}

/// All semantic issues found in one validation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors {
    issues: Vec<ValidationIssue>,
}

impl ValidationErrors {
    /// Returns the semantic issues in deterministic traversal order.
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "HawkEye configuration has {} validation issue{}",
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

/// One semantic configuration issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    path: String,
    message: String,
}

impl ValidationIssue {
    /// Returns the configuration path associated with the issue.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the human-readable reason the value is invalid.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Header content and the terms used to recognize a license header safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderConfig {
    source: HeaderSource,
    required_terms: Vec<String>,
}

impl HeaderConfig {
    /// Returns the single configured header source.
    pub fn source(&self) -> &HeaderSource {
        &self.source
    }

    /// Returns the terms that must all occur in a candidate header.
    pub fn required_terms(&self) -> &[String] {
        &self.required_terms
    }
}

/// The source of the canonical, unformatted header template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderSource {
    /// A template bundled with HawkEye, addressed by a snake-case name.
    Builtin(String),
    /// A template file; relative paths are resolved against the config file.
    Path(PathBuf),
    /// A template embedded directly in the configuration.
    Text(String),
}

/// File discovery and built-in catalog settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesConfig {
    root: PathBuf,
    includes: Vec<String>,
    excludes: Vec<String>,
    use_default_excludes: bool,
    use_default_rules: bool,
}

impl FilesConfig {
    /// Returns the discovery root, relative to the config file unless absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns root-relative inclusion patterns.
    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    /// Returns root-relative exclusion patterns.
    pub fn excludes(&self) -> &[String] {
        &self.excludes
    }

    /// Returns whether HawkEye's standard exclusions participate in discovery.
    pub fn use_default_excludes(&self) -> bool {
        self.use_default_excludes
    }

    /// Returns whether built-in file-to-style rules follow user rules.
    pub fn use_default_rules(&self) -> bool {
        self.use_default_rules
    }
}

/// Opt-in integration with the Git repository containing the discovery root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitConfig {
    respect_ignore: bool,
    file_dates: bool,
}

impl GitConfig {
    /// Returns whether ignored files are excluded from ordinary discovery.
    pub fn respect_ignore(&self) -> bool {
        self.respect_ignore
    }

    /// Returns whether per-file dates should be derived from Git history.
    pub fn file_dates(&self) -> bool {
        self.file_dates
    }
}

/// An ordered mapping from path patterns to a canonical output style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    patterns: Vec<String>,
    write_style: StyleName,
    recognize_styles: Vec<StyleName>,
}

impl Rule {
    /// Returns the root-relative patterns evaluated by this rule.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Returns the only style HawkEye writes for matching files.
    pub fn write_style(&self) -> &StyleName {
        &self.write_style
    }

    /// Returns additional styles that HawkEye may replace or remove safely.
    pub fn recognize_styles(&self) -> &[StyleName] {
        &self.recognize_styles
    }
}

/// A validated snake-case style name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StyleName(String);

impl StyleName {
    /// Returns the name as it appears in the configuration.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StyleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Borrow<str> for StyleName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for StyleName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A syntax-only description of how header lines are wrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Style {
    /// Every logical header line has its own prefix and suffix.
    Line(LineStyle),
    /// One pair of delimiters encloses all logical header lines.
    Block(BlockStyle),
}

/// A style that wraps each logical header line independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineStyle {
    prefix: String,
    suffix: String,
    align_suffix: bool,
}

impl LineStyle {
    /// Returns the token written before each logical line.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Returns the token written after each logical line.
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    /// Returns whether suffixes are padded into one aligned column.
    pub fn align_suffix(&self) -> bool {
        self.align_suffix
    }
}

/// A style that encloses all logical header lines in one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStyle {
    start: String,
    prefix: String,
    suffix: String,
    end: String,
}

impl BlockStyle {
    /// Returns the opening delimiter.
    pub fn start(&self) -> &str {
        &self.start
    }

    /// Returns the token written before each logical line inside the block.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Returns the token written after each logical line inside the block.
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    /// Returns the closing delimiter.
    pub fn end(&self) -> &str {
        &self.end
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawConfig {
    schema_version: u32,
    header: RawHeader,
    #[serde(default)]
    files: RawFilesConfig,
    #[serde(default)]
    variables: BTreeMap<String, toml::Value>,
    #[serde(default)]
    git: RawGitConfig,
    #[serde(default)]
    rules: Vec<RawRule>,
    #[serde(default)]
    styles: BTreeMap<String, RawStyle>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawHeader {
    builtin: Option<String>,
    path: Option<String>,
    text: Option<String>,
    #[serde(default = "default_required_terms")]
    required_terms: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
struct RawFilesConfig {
    root: String,
    includes: Vec<String>,
    excludes: Vec<String>,
    use_default_excludes: bool,
    use_default_rules: bool,
}

impl Default for RawFilesConfig {
    fn default() -> Self {
        Self {
            root: ".".to_owned(),
            includes: Vec::new(),
            excludes: Vec::new(),
            use_default_excludes: true,
            use_default_rules: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
struct RawGitConfig {
    respect_ignore: bool,
    file_dates: bool,
}

impl Default for RawGitConfig {
    fn default() -> Self {
        Self {
            respect_ignore: true,
            file_dates: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct RawRule {
    patterns: Vec<String>,
    write_style: String,
    #[serde(default)]
    recognize_styles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
enum RawStyle {
    Line {
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
        #[serde(default)]
        align_suffix: bool,
    },
    Block {
        start: String,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
        end: String,
    },
}

fn default_required_terms() -> Vec<String> {
    vec!["copyright".to_owned()]
}

fn validate(raw: &RawConfig) -> Vec<ValidationIssue> {
    let mut validator = Validator::default();

    if raw.schema_version != SCHEMA_VERSION {
        validator.issue(
            "schema_version",
            format!(
                "unsupported schema version {}; this release requires {SCHEMA_VERSION}",
                raw.schema_version
            ),
        );
    }

    validator.header(&raw.header);
    validator.files(&raw.files);
    validator.variables(&raw.variables);
    validator.rules(&raw.rules);
    validator.styles(&raw.styles);

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

    fn header(&mut self, header: &RawHeader) {
        let source_count = usize::from(header.builtin.is_some())
            + usize::from(header.path.is_some())
            + usize::from(header.text.is_some());

        if source_count != 1 {
            self.issue(
                "header",
                "exactly one of `builtin`, `path`, or `text` must be set",
            );
        }

        if let Some(builtin) = &header.builtin {
            self.identifier("header.builtin", builtin, "built-in header name");
        }
        if let Some(path) = &header.path {
            self.non_blank("header.path", path, "header path");
            self.no_nul("header.path", path);
        }
        if let Some(text) = &header.text {
            self.non_blank("header.text", text, "inline header template");
        }

        if header.required_terms.is_empty() {
            self.issue(
                "header.required_terms",
                "at least one term is required for safe header recognition",
            );
        }

        let mut seen = HashMap::<String, usize>::new();
        for (index, term) in header.required_terms.iter().enumerate() {
            let path = format!("header.required_terms[{index}]");
            self.non_blank(&path, term, "required term");

            if term.trim() != term {
                self.issue(
                    &path,
                    "required terms must not start or end with whitespace",
                );
            }

            let normalized = term.to_lowercase();
            if let Some(first_index) = seen.get(&normalized) {
                self.issue(
                    path,
                    format!(
                        "duplicates `header.required_terms[{first_index}]` under case-insensitive matching"
                    ),
                );
            } else {
                seen.insert(normalized, index);
            }
        }
    }

    fn files(&mut self, files: &RawFilesConfig) {
        self.non_blank("files.root", &files.root, "file discovery root");
        self.no_nul("files.root", &files.root);
        self.patterns("files.includes", &files.includes, false);
        self.patterns("files.excludes", &files.excludes, false);
    }

    fn variables(&mut self, variables: &BTreeMap<String, toml::Value>) {
        for (name, value) in variables {
            let path = format!("variables.{name}");
            self.identifier(&path, name, "variable name");
            self.variable_value(&path, value);
        }
    }

    fn variable_value(&mut self, path: &str, value: &toml::Value) {
        match value {
            toml::Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    self.variable_value(&format!("{path}[{index}]"), value);
                }
            }
            toml::Value::Table(values) => {
                for (name, value) in values {
                    let child_path = format!("{path}.{name}");
                    self.identifier(&child_path, name, "variable name");
                    self.variable_value(&child_path, value);
                }
            }
            _ => {}
        }
    }

    fn rules(&mut self, rules: &[RawRule]) {
        for (index, rule) in rules.iter().enumerate() {
            let path = format!("rules[{index}]");
            self.patterns(&format!("{path}.patterns"), &rule.patterns, true);
            self.identifier(
                format!("{path}.write_style"),
                &rule.write_style,
                "style name",
            );

            let mut seen = HashMap::<&str, usize>::new();
            for (style_index, style) in rule.recognize_styles.iter().enumerate() {
                let style_path = format!("{path}.recognize_styles[{style_index}]");
                self.identifier(&style_path, style, "style name");

                if style == &rule.write_style {
                    self.issue(
                        &style_path,
                        "the write style is recognized implicitly and must not be repeated",
                    );
                }

                if let Some(first_index) = seen.get(&style.as_str()) {
                    self.issue(
                        style_path,
                        format!("duplicates `{path}.recognize_styles[{first_index}]`"),
                    );
                } else {
                    seen.insert(style, style_index);
                }
            }
        }
    }

    fn styles(&mut self, styles: &BTreeMap<String, RawStyle>) {
        for (name, style) in styles {
            let path = format!("styles.{name}");
            self.identifier(&path, name, "style name");

            match style {
                RawStyle::Line {
                    prefix,
                    suffix,
                    align_suffix,
                } => {
                    self.style_token(&format!("{path}.prefix"), prefix);
                    self.style_token(&format!("{path}.suffix"), suffix);

                    if prefix.trim().is_empty() && suffix.trim().is_empty() {
                        self.issue(
                            &path,
                            "a line style needs a non-whitespace prefix or suffix",
                        );
                    }
                    if *align_suffix && suffix.is_empty() {
                        self.issue(
                            format!("{path}.align_suffix"),
                            "suffix alignment requires a non-empty suffix",
                        );
                    }
                }
                RawStyle::Block {
                    start,
                    prefix,
                    suffix,
                    end,
                } => {
                    self.style_token(&format!("{path}.start"), start);
                    self.style_token(&format!("{path}.prefix"), prefix);
                    self.style_token(&format!("{path}.suffix"), suffix);
                    self.style_token(&format!("{path}.end"), end);

                    if start.trim().is_empty() {
                        self.issue(
                            format!("{path}.start"),
                            "a block opening delimiter must contain a non-whitespace character",
                        );
                    }
                    if end.trim().is_empty() {
                        self.issue(
                            format!("{path}.end"),
                            "a block closing delimiter must contain a non-whitespace character",
                        );
                    }
                }
            }
        }
    }

    fn patterns(&mut self, path: &str, patterns: &[String], require_one: bool) {
        if require_one && patterns.is_empty() {
            self.issue(path, "at least one pattern is required");
        }

        let mut seen = HashMap::<&str, usize>::new();
        for (index, pattern) in patterns.iter().enumerate() {
            let pattern_path = format!("{path}[{index}]");
            self.non_blank(&pattern_path, pattern, "pattern");
            self.no_nul(&pattern_path, pattern);

            if let Some(first_index) = seen.get(&pattern.as_str()) {
                self.issue(pattern_path, format!("duplicates `{path}[{first_index}]`"));
            } else {
                seen.insert(pattern, index);
            }
        }
    }

    fn identifier(&mut self, path: impl Into<String>, value: &str, kind: &str) {
        if !is_snake_case_identifier(value) {
            self.issue(
                path,
                format!("{kind} `{value}` must be an ASCII snake_case identifier"),
            );
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

    fn style_token(&mut self, path: &str, token: &str) {
        if token.contains(['\r', '\n']) {
            self.issue(path, "style tokens must not contain line endings");
        }
    }
}

fn is_snake_case_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !value.ends_with('_')
        && !value.contains("__")
}

impl From<RawConfig> for Config {
    fn from(raw: RawConfig) -> Self {
        let header = raw.header.into();
        let files = raw.files.into();
        let git = raw.git.into();
        let rules = raw.rules.into_iter().map(Into::into).collect();
        let styles = raw
            .styles
            .into_iter()
            .map(|(name, style)| (StyleName(name), style.into()))
            .collect();

        Self {
            schema_version: raw.schema_version,
            header,
            files,
            variables: raw.variables,
            git,
            rules,
            styles,
        }
    }
}

impl From<RawHeader> for HeaderConfig {
    fn from(raw: RawHeader) -> Self {
        let source = match (raw.builtin, raw.path, raw.text) {
            (Some(name), None, None) => HeaderSource::Builtin(name),
            (None, Some(path), None) => HeaderSource::Path(path.into()),
            (None, None, Some(text)) => HeaderSource::Text(text),
            _ => unreachable!("header source cardinality is validated before conversion"),
        };

        Self {
            source,
            required_terms: raw.required_terms,
        }
    }
}

impl From<RawFilesConfig> for FilesConfig {
    fn from(raw: RawFilesConfig) -> Self {
        Self {
            root: raw.root.into(),
            includes: raw.includes,
            excludes: raw.excludes,
            use_default_excludes: raw.use_default_excludes,
            use_default_rules: raw.use_default_rules,
        }
    }
}

impl From<RawGitConfig> for GitConfig {
    fn from(raw: RawGitConfig) -> Self {
        Self {
            respect_ignore: raw.respect_ignore,
            file_dates: raw.file_dates,
        }
    }
}

impl From<RawRule> for Rule {
    fn from(raw: RawRule) -> Self {
        Self {
            patterns: raw.patterns,
            write_style: StyleName(raw.write_style),
            recognize_styles: raw.recognize_styles.into_iter().map(StyleName).collect(),
        }
    }
}

impl From<RawStyle> for Style {
    fn from(raw: RawStyle) -> Self {
        match raw {
            RawStyle::Line {
                prefix,
                suffix,
                align_suffix,
            } => Self::Line(LineStyle {
                prefix,
                suffix,
                align_suffix,
            }),
            RawStyle::Block {
                start,
                prefix,
                suffix,
                end,
            } => Self::Block(BlockStyle {
                start,
                prefix,
                suffix,
                end,
            }),
        }
    }
}
