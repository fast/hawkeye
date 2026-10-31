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

//! HawkEye configuration types.
//!
//! [`Config::load`] parses TOML, renders path templates, and resolves relative paths.
//! [`Config::validate`] checks relationships between the parsed fields.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use crate::Error;
use crate::ErrorKind;
use crate::builtin;
use crate::template::PathTemplates;

/// Configuration for a HawkEye engine.
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
    /// Loads a TOML config file, rendering path templates and resolving relative paths
    /// from the config directory.
    ///
    /// For semantic validation, call [`Self::validate`] or pass the result to
    /// [`Engine::new`](crate::Engine::new).
    ///
    /// # Errors
    ///
    /// Returns an error if reading, parsing, or resolving the configuration fails.
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
        let templates = PathTemplates::new(&path)?;
        config.files.root = templates.resolve("files.root", &config.files.root)?;
        if let Some(header_path) = &mut config.header.path {
            *header_path = templates.resolve("header.path", header_path)?;
        }
        Ok(config)
    }

    /// Validates relationships between configuration fields.
    ///
    /// Resource-dependent checks, such as opening `files.root`, loading a header template, and
    /// resolving style names, are performed by [`Engine::new`](crate::Engine::new).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::ConfigInvalid`] when one or more fields are inconsistent.
    pub fn validate(&self) -> Result<(), Error> {
        let mut validator = Validator::default();
        validator.header(&self.header);
        if let Some(key) = &self.header.builtin {
            if let Some(header) = builtin::HEADERS.get(key.as_str()) {
                validator.builtin_props(key, header, &self.props);
            } else {
                let available = builtin::HEADERS
                    .keys()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ");
                validator.issue(
                    "header.builtin",
                    format!("unknown builtin {key:?}; available values are {available}"),
                );
            }
        }
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

/// Header template and recognition settings.
///
/// Exactly one of `builtin`, `path`, or `text` must be set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderConfig {
    /// The built-in template key, when using a bundled header.
    pub builtin: Option<String>,
    /// The template file.
    ///
    /// [`Config::load`] renders path templates and resolves relative paths from the config
    /// directory.
    pub path: Option<PathBuf>,
    /// The inline template, when the header is stored in the config file.
    pub text: Option<String>,
    /// Case-insensitive words required in a recognized header.
    ///
    /// `None` uses the selected builtin's defaults, or `"copyright"` for custom templates,
    /// when the engine is built. An explicit list must not be empty.
    pub keywords: Option<Vec<String>>,
}

/// File discovery settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FilesConfig {
    /// The directory to scan; defaults to `.`.
    ///
    /// [`Config::load`] renders path templates and resolves relative paths from the config
    /// directory.
    pub root: PathBuf,
    /// Git-ignore-style inclusion patterns; an empty list selects all files.
    pub includes: Vec<String>,
    /// Git-ignore-style exclusion patterns applied after `includes`.
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

/// Availability policy for a Git-backed feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum FeatureMode {
    /// Never use the feature.
    #[serde(rename = "disable")]
    Disable,
    /// Use the feature when available without requiring it.
    #[serde(rename = "auto")]
    #[default]
    Auto,
    /// Require the feature and fail when it is unavailable.
    #[serde(rename = "enable")]
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
    /// Controls Git-aware file discovery; defaults to [`FeatureMode::Auto`].
    pub ignore: FeatureMode,
    /// Controls Git-derived template attributes; defaults to [`FeatureMode::Disable`].
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

/// File selectors and the comment styles used for matching and output.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    /// Case-insensitive filename suffixes without a leading dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Complete filenames matched case-insensitively.
    #[serde(default)]
    pub filenames: Vec<String>,
    /// The style written by `format`.
    pub style_out: String,
    /// The accepted input styles; an empty list means only `style_out`.
    #[serde(default)]
    pub styles_in: Vec<String>,
}

/// A custom comment style.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StyleConfig {
    /// Wraps each header line independently.
    #[serde(rename = "line")]
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
    /// Encloses all header lines between opening and closing delimiters.
    #[serde(rename = "block")]
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

#[derive(Default)]
struct Validator {
    issues: Vec<String>,
}

impl Validator {
    fn issue(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues
            .push(format!("{}: {}", path.into(), message.into()));
    }

    fn builtin_props(
        &mut self,
        key: &str,
        header: &builtin::Header,
        props: &BTreeMap<String, toml::Value>,
    ) {
        for prop in header.props {
            let path = format!("props.{}", prop.name);
            let Some(value) = props.get(prop.name) else {
                if prop.required {
                    self.issue(path, format!("is required by the {key} builtin"));
                }
                continue;
            };
            if prop.allow_integer && value.is_integer() {
                continue;
            }
            let Some(value) = value.as_str() else {
                self.issue(
                    path,
                    if prop.allow_integer {
                        "must be a string or integer"
                    } else {
                        "must be a string"
                    },
                );
                continue;
            };
            self.non_blank(&path, value);
            self.no_nul(&path, value);
            if value.contains(['\r', '\n']) {
                self.issue(path, "must fit on a single line");
            }
        }
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

        let Some(keywords) = &header.keywords else {
            return;
        };
        if keywords.is_empty() {
            self.issue(
                "header.keywords",
                "at least one keyword is required to distinguish a header from an ordinary comment",
            );
        }
        let mut seen = HashMap::<String, usize>::new();
        for (index, keyword) in keywords.iter().enumerate() {
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
            }
            for (item, filename) in rule.filenames.iter().enumerate() {
                let item_path = format!("{path}.filenames[{item}]");
                self.non_blank(&item_path, filename);
                if filename.contains(['/', '\\']) {
                    self.issue(&item_path, "filename must not contain a path separator");
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
