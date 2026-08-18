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

mod analyze;
mod attrs;
mod discovery;
mod git;
mod lines;
mod style;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use ignore::overrides::Override;

use self::analyze::FileAnalysis;
use self::analyze::Replacement;
pub use self::attrs::FileAttrs;
use self::git::GitRepo;
use self::style::Style;
use crate::Error;
use crate::ErrorKind;
use crate::builtin;
use crate::config::Config;
use crate::config::FeatureMode;
use crate::config::GitConfig;
use crate::config::RuleConfig;
use crate::report::FileOutcome;
use crate::report::FileReport;
use crate::report::Report;
use crate::template::HeaderTemplate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Present,
    Absent,
}

/// A reusable HawkEye runtime built from one configuration.
pub struct Engine {
    root: PathBuf,
    header_path: Option<PathBuf>,
    selection: Override,
    exclusions: Override,
    props: BTreeMap<String, toml::Value>,
    git: GitConfig,
    keywords: Vec<String>,
    template: HeaderTemplate,
    styles: BTreeMap<String, Style>,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    extension_suffixes: BTreeSet<String>,
    filenames: BTreeSet<String>,
    style_out: String,
    styles_in: Vec<String>,
}

impl Engine {
    /// Validates and builds an engine from parsed configuration.
    pub fn new(config: Config) -> Result<Self, Error> {
        config.validate()?;

        let Config {
            header,
            files,
            props,
            git,
            styles: configured_styles,
            rules: configured_rules,
        } = config;

        let root = files.root.canonicalize().map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot resolve file root {}", files.root.display()),
            )
            .with_source(err)
        })?;
        if !root.is_dir() {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                format!("files.root is not a directory: {}", root.display()),
            ));
        }
        let (selection, exclusions) =
            discovery::compile_patterns(&root, &files.includes, &files.excludes)?;

        let (template, header_path) = if let Some(content) = header.text {
            (HeaderTemplate::new(content)?, None)
        } else if let Some(path) = header.path {
            let path = path.canonicalize().map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot resolve header template {}", path.display()),
                )
                .with_source(err)
            })?;
            let content = fs::read_to_string(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read header template {}", path.display()),
                )
                .with_source(err)
            })?;
            (HeaderTemplate::new(content)?, Some(path))
        } else if let Some(key) = header.builtin {
            let content = builtin::HEADERS.get(key.as_str()).copied().ok_or_else(|| {
                let available = builtin::HEADERS
                    .keys()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::new(
                    ErrorKind::ConfigInvalid,
                    format!("unknown header.builtin {key:?}; available values are {available}"),
                )
            })?;
            (HeaderTemplate::new(content)?, None)
        } else {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                "header source is missing",
            ));
        };

        let styles = {
            let mut styles = builtin::STYLES
                .iter()
                .map(|(name, style)| (name.clone(), Style::new(style.clone())))
                .collect::<BTreeMap<_, _>>();
            for (name, style) in configured_styles {
                if styles.contains_key(&name) {
                    log::warn!("custom style {name:?} overrides a built-in style of the same name");
                }
                styles.insert(name, Style::new(style));
            }
            styles
        };

        let configured_rules = configured_rules
            .into_iter()
            .enumerate()
            .map(|(index, rule)| (format!("rules[{index}]"), rule));
        let builtin_rules = builtin::RULES
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, rule)| (format!("builtin.rules[{index}]"), rule));
        let mut selectors = BTreeMap::<(&str, String), String>::new();
        let rules = configured_rules
            .chain(builtin_rules)
            .map(|(source, rule)| {
                let extensions = rule.extensions.iter().map(|value| ("extension", value));
                let filenames = rule.filenames.iter().map(|value| ("filename", value));
                for (kind, selector) in extensions.chain(filenames) {
                    let key = (kind, selector.to_lowercase());
                    if let Some(owner) = selectors.get(&key) {
                        log::debug!("{source} {kind} {selector:?} is shadowed by {owner}");
                    } else {
                        selectors.insert(key, source.clone());
                    }
                }
                Rule::new(&source, rule, &styles)
            })
            .collect::<Result<Vec<_>, Error>>()?;

        Ok(Self {
            root,
            header_path,
            selection,
            exclusions,
            props,
            git,
            keywords: header
                .keywords
                .into_iter()
                .map(|keyword| keyword.to_lowercase())
                .collect(),
            template,
            styles,
            rules,
        })
    }

    /// Reports the changes needed to make selected headers canonical.
    pub fn check(&self) -> Result<Report, Error> {
        Ok(self.edits(Target::Present)?.report)
    }

    /// Prepares edits that make selected headers canonical.
    pub fn format(&self) -> Result<Edits, Error> {
        self.edits(Target::Present)
    }

    /// Prepares edits that remove recognized headers from selected files.
    pub fn remove(&self) -> Result<Edits, Error> {
        self.edits(Target::Absent)
    }

    fn edits(&self, target: Target) -> Result<Edits, Error> {
        let git = self.git;
        let git_mode = git.ignore.combine(git.file_attrs);
        let repo = if git_mode == FeatureMode::Disable {
            None
        } else {
            match GitRepo::discover(&self.root) {
                Ok(repo) => Some(repo),
                Err(err)
                    if git_mode == FeatureMode::Auto && err.kind() == ErrorKind::Unsupported =>
                {
                    log::debug!("Git integration is unavailable: {err}");
                    None
                }
                Err(err) => return Err(err),
            }
        };
        let relative_paths = self.discover(repo.as_ref())?;
        let discovered = relative_paths
            .into_iter()
            .map(|path| {
                let rule = self.rule_for(&path);
                (path, rule)
            })
            .collect::<Vec<_>>();
        let supported = discovered
            .iter()
            .filter_map(|(path, rule)| rule.is_some().then_some(path.as_path()))
            .collect::<Vec<_>>();
        let git_history = if git.file_attrs == FeatureMode::Disable || supported.is_empty() {
            None
        } else if let Some(repo) = repo.as_ref() {
            if repo.is_shallow()? {
                let message = "Git file attributes require complete history, but the repository is shallow; fetch complete history first";
                if git.file_attrs == FeatureMode::Auto {
                    log::warn!("{message}; continuing with Git file attributes disabled");
                    None
                } else {
                    return Err(Error::new(ErrorKind::Unsupported, message));
                }
            } else {
                Some(repo.file_history(&self.root, supported)?)
            }
        } else {
            debug_assert_ne!(git.file_attrs, FeatureMode::Enable);
            None
        };
        let mut report = Report {
            files: Vec::with_capacity(discovered.len()),
        };
        let mut file_edits = Vec::new();

        for (relative, rule) in discovered {
            let Some(rule) = rule else {
                report.files.push(FileReport {
                    path: relative,
                    outcome: FileOutcome::Unsupported,
                });
                continue;
            };

            let path = self.root.join(&relative);
            let original = fs::read(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            let Ok(input) = std::str::from_utf8(&original) else {
                report.files.push(FileReport {
                    path: relative,
                    outcome: FileOutcome::Unsupported,
                });
                continue;
            };
            let file_attrs = FileAttrs::new(
                &path,
                git_history
                    .as_ref()
                    .and_then(|history| history.get(&relative)),
            )?;
            let header = self.render_header(&file_attrs)?;
            let FileAnalysis {
                outcome,
                replacement,
            } = self.analyze(rule, input, &header, target);
            report.files.push(FileReport {
                path: relative,
                outcome,
            });
            if let Some(replacement) = replacement {
                file_edits.push(FileEdit { path, replacement });
            }
        }

        Ok(Edits {
            report,
            files: file_edits,
        })
    }

    fn rule_for(&self, path: &Path) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.matches(path))
    }

    fn style(&self, name: &str) -> &Style {
        self.styles
            .get(name)
            .expect("resolved rules only refer to known styles")
    }

    fn render_header(&self, attrs: &FileAttrs) -> Result<String, Error> {
        let header = self.template.render(&self.props, attrs)?;
        let folded = header.to_lowercase();
        if let Some(keyword) = self
            .keywords
            .iter()
            .find(|keyword| !folded.contains(keyword.as_str()))
        {
            let message = match attrs.filename.as_deref() {
                Some(filename) => format!(
                    "header template output for {filename:?} does not contain recognition keyword {keyword:?}"
                ),
                None => format!(
                    "header template output does not contain recognition keyword {keyword:?}"
                ),
            };
            return Err(Error::new(ErrorKind::ConfigInvalid, message));
        }
        Ok(header)
    }
}

/// File edits prepared by an [`Engine`].
#[must_use = "edits have no effect until they are applied"]
pub struct Edits {
    /// The outcome for every selected file.
    pub report: Report,
    files: Vec<FileEdit>,
}

impl Edits {
    /// Applies every edit directly to its source file and returns the report.
    ///
    /// Callers must ensure that selected files are not modified between preparing and applying the
    /// edits.
    pub fn apply(self) -> Result<Report, Error> {
        for file in self.files {
            let mut input = fs::read_to_string(&file.path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", file.path.display()),
                )
                .with_source(err)
            })?;
            file.replacement.apply(&mut input);
            fs::write(&file.path, input).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot write {}", file.path.display()),
                )
                .with_source(err)
            })?;
        }
        Ok(self.report)
    }
}

struct FileEdit {
    path: PathBuf,
    replacement: Replacement,
}

impl Rule {
    fn new(
        source: impl AsRef<str>,
        config: RuleConfig,
        styles: &BTreeMap<String, Style>,
    ) -> Result<Self, Error> {
        let RuleConfig {
            extensions,
            filenames,
            style_out,
            mut styles_in,
        } = config;
        let source = source.as_ref();
        if !styles.contains_key(&style_out) {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                format!("{source} references unknown style {style_out:?}"),
            ));
        }
        if styles_in.is_empty() {
            styles_in.push(style_out.clone());
        }
        let mut accepted = Vec::with_capacity(styles_in.len());
        let mut seen = BTreeSet::new();
        for name in styles_in {
            if !styles.contains_key(&name) {
                return Err(Error::new(
                    ErrorKind::ConfigInvalid,
                    format!("{source} references unknown style {name:?}"),
                ));
            }
            if !seen.insert(name.clone()) {
                log::warn!("{source}.styles_in contains duplicate style {name:?}; ignoring it");
                continue;
            }
            accepted.push(name);
        }
        Ok(Self {
            extension_suffixes: extensions
                .into_iter()
                .map(|extension| format!(".{}", extension.to_lowercase()))
                .collect(),
            filenames: filenames
                .into_iter()
                .map(|filename| filename.to_lowercase())
                .collect(),
            style_out,
            styles_in: accepted,
        })
    }

    fn matches(&self, path: &Path) -> bool {
        let Some(filename) = path.file_name() else {
            return false;
        };
        let filename = filename.to_string_lossy().to_lowercase();
        self.filenames.contains(&filename)
            || self
                .extension_suffixes
                .iter()
                .any(|extension| filename.ends_with(extension))
    }
}
