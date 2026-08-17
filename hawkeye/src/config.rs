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

//! The strict HawkEye configuration model.
//!
//! [`Config`] contains every value represented directly by TOML. [`Config::load`]
//! also anchors relative paths to the directory containing the loaded file.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::Error;
use crate::ErrorKind;

/// A HawkEye configuration.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The header template source and recognition keywords.
    pub header: HeaderConfig,
    /// File discovery settings.
    #[serde(default)]
    pub files: FilesConfig,
    /// User values exposed to MiniJinja as the `props` object.
    #[serde(default)]
    pub props: BTreeMap<String, toml::Value>,
    /// Git integration settings.
    #[serde(default)]
    pub git: GitConfig,
    /// Custom comment styles keyed by name.
    #[serde(default)]
    pub styles: BTreeMap<String, StyleConfig>,
    /// Filename and extension rules in declaration order.
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

impl Config {
    /// Reads a config file and anchors its relative paths to that file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot read config file {}", path.display()),
            )
            .with_source(err)
        })?;
        let mut config = toml::from_str::<Self>(&source).map_err(|err| {
            Error::new(
                ErrorKind::ConfigInvalid,
                format!("cannot parse config file {}", path.display()),
            )
            .with_source(err)
        })?;
        let path = path.canonicalize().map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot resolve config file {}", path.display()),
            )
            .with_source(err)
        })?;
        let directory = path.parent().ok_or_else(|| {
            Error::new(
                ErrorKind::ConfigInvalid,
                "config file has no parent directory",
            )
        })?;

        if config.files.root.is_relative() {
            config.files.root = directory.join(&config.files.root);
        }
        if let Some(header_path) = &mut config.header.path
            && header_path.is_relative()
        {
            *header_path = directory.join(&*header_path);
        }
        Ok(config)
    }

    /// Validates configuration invariants that do not require runtime resources.
    pub fn validate(&self) -> Result<(), Error> {
        let mut validator = Validator::default();
        validator.header(&self.header);
        validator.files(&self.files);
        validator.styles(&self.styles);
        validator.rules(&self.rules);
        if validator.issues.is_empty() {
            return Ok(());
        }

        let mut message = String::from("config validation failed:");
        for issue in validator.issues {
            message.push_str("\n- ");
            message.push_str(&issue);
        }
        Err(Error::new(ErrorKind::ConfigInvalid, message))
    }
}

/// The header template source and the words used to recognize an old header.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderConfig {
    /// A built-in header resource key.
    pub builtin: Option<String>,
    /// A template path anchored to the config file by [`Config::load`] when relative.
    pub path: Option<PathBuf>,
    /// An inline header template.
    pub text: Option<String>,
    /// Words that must occur in a structurally recognized header.
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,
}

/// File discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesConfig {
    /// The root scanned by HawkEye, anchored to the config file by [`Config::load`] when relative.
    pub root: PathBuf,
    /// Git-ignore-style inclusion patterns; an empty list selects all files.
    pub includes: Vec<String>,
    /// Git-ignore-style exclusion patterns.
    pub excludes: Vec<String>,
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

impl FeatureMode {
    pub(crate) fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Enable, _) | (_, Self::Enable) => Self::Enable,
            (Self::Auto, _) | (_, Self::Auto) => Self::Auto,
            (Self::Disable, Self::Disable) => Self::Disable,
        }
    }
}

/// Git integration settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    /// How Git ignore files participate in discovery.
    pub ignore: FeatureMode,
    /// How per-file Git attributes are populated for templates.
    pub file_attrs: FeatureMode,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            ignore: FeatureMode::Auto,
            file_attrs: FeatureMode::Disable,
        }
    }
}

/// A filename/extension rule and its accepted/output comment styles.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    /// Suffixes matched after the final filename separator, without a leading dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Complete filenames matched case-insensitively.
    #[serde(default)]
    pub filenames: Vec<String>,
    /// The canonical style used for output.
    pub style_out: String,
    /// The complete set of structurally safe input styles, or the output style when empty.
    #[serde(default)]
    pub styles_in: Vec<String>,
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

fn default_keywords() -> Vec<String> {
    vec!["copyright".to_owned()]
}

#[derive(Default)]
struct Validator {
    issues: Vec<String>,
}

impl Validator {
    fn issue(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues
            .push(format!("{}: {}", path.into(), message.into()));
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
            self.non_blank("header.builtin", value);
            self.no_nul("header.builtin", value);
        }
        if let Some(value) = &header.path
            && value.as_os_str().is_empty()
        {
            self.issue("header.path", "header path must not be empty");
        }
        if let Some(value) = &header.text {
            self.non_blank("header.text", value);
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
            self.non_blank(&path, keyword);
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
            self.non_blank(&path, pattern);
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
            self.non_blank(&path, name);
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
        let mut extensions = HashMap::<String, String>::new();
        let mut filenames = HashMap::<String, String>::new();
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
                self.non_blank(&item_path, extension);
                if extension.starts_with('.') {
                    self.issue(&item_path, "extension must not start with `.`");
                }
                if extension.contains(['/', '\\']) {
                    self.issue(&item_path, "extension must not contain a path separator");
                }
                if !extension.trim().is_empty() {
                    let folded = extension.to_lowercase();
                    if let Some(first) = extensions.get(&folded) {
                        self.issue(
                            &item_path,
                            format!("duplicates `{first}` case-insensitively"),
                        );
                    } else {
                        extensions.insert(folded, item_path);
                    }
                }
            }
            for (item, filename) in rule.filenames.iter().enumerate() {
                let item_path = format!("{path}.filenames[{item}]");
                self.non_blank(&item_path, filename);
                if filename.contains(['/', '\\']) {
                    self.issue(&item_path, "filename must not contain a path separator");
                }
                if !filename.trim().is_empty() {
                    let folded = filename.to_lowercase();
                    if let Some(first) = filenames.get(&folded) {
                        self.issue(
                            &item_path,
                            format!("duplicates `{first}` case-insensitively"),
                        );
                    } else {
                        filenames.insert(folded, item_path);
                    }
                }
            }

            self.non_blank(&format!("{path}.style_out"), &rule.style_out);
            if !rule.styles_in.is_empty() && !rule.styles_in.contains(&rule.style_out) {
                self.issue(
                    format!("{path}.styles_in"),
                    "must include `style_out` when explicitly configured",
                );
            }
            for (item, style) in rule.styles_in.iter().enumerate() {
                self.non_blank(&format!("{path}.styles_in[{item}]"), style);
            }
        }
    }

    fn non_blank(&mut self, path: &str, value: &str) {
        if value.trim().is_empty() {
            self.issue(path, "must not be blank");
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
