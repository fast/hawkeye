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
mod discovery;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use ignore::overrides::Override;

use crate::Error;
use crate::ErrorKind;
use crate::attrs::FileAttrs;
use crate::attrs::FileAttrsResolver;
use crate::builtin;
use crate::config::Config;
use crate::config::GitConfig;
use crate::config::RuleConfig;
use crate::git::GitRepo;
use crate::report::FileOutcome;
use crate::report::Outcome;
use crate::report::Report;
use crate::style::Style;
use crate::template::HeaderTemplate;

/// The action to plan for selected files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Report the changes needed to make headers canonical.
    Check,
    /// Add or replace headers to make them canonical.
    Format,
    /// Remove recognized headers.
    Remove,
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

struct Analysis {
    outcome: Outcome,
    edit: Option<Edit>,
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

impl Edit {
    fn apply(&self, input: &str) -> String {
        debug_assert!(self.range.start <= self.range.end);
        debug_assert!(self.range.end <= input.len());
        debug_assert!(input.is_char_boundary(self.range.start));
        debug_assert!(input.is_char_boundary(self.range.end));

        let mut output = String::with_capacity(
            input.len() - (self.range.end - self.range.start) + self.replacement.len(),
        );
        output.push_str(&input[..self.range.start]);
        output.push_str(&self.replacement);
        output.push_str(&input[self.range.end..]);
        output
    }
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

    /// Discovers and analyzes files without modifying the filesystem.
    pub fn plan(&self, action: Action) -> Result<Plan, Error> {
        let git = self.git;
        let repo = GitRepo::discover(&self.root, git.ignore.combine(git.file_attrs))?;
        let paths = self.discover(repo.as_ref())?;
        let discovered = paths
            .into_iter()
            .map(|path| {
                let relative = path
                    .strip_prefix(&self.root)
                    .expect("discovery only returns paths inside files.root")
                    .to_path_buf();
                let rule = self.rule_for(&relative);
                (path, relative, rule)
            })
            .collect::<Vec<_>>();
        let attrs = FileAttrsResolver::new(
            discovered
                .iter()
                .filter_map(|(path, _, rule)| rule.is_some().then_some(path.as_path())),
            git.file_attrs,
            repo.as_ref(),
        )?;
        let mut files = Vec::with_capacity(discovered.len());

        for (path, relative, rule) in discovered {
            let Some(rule) = rule else {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            };

            let original = fs::read(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            let Ok(input) = std::str::from_utf8(&original) else {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            };
            let file_attrs = attrs.for_file(&path)?;
            let header = self.render_header(&file_attrs)?;
            let analysis = self.analyze(rule, input, &header, action);
            let updated = analysis
                .edit
                .as_ref()
                .map(|edit| edit.apply(input))
                .filter(|output| output.as_bytes() != original)
                .map(String::into_bytes);
            files.push(PlannedFile {
                absolute_path: path,
                relative_path: relative,
                outcome: analysis.outcome,
                updated,
            });
        }

        Ok(Plan { files })
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
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                format!(
                    "header template output for {:?} does not contain recognition keyword {keyword:?}",
                    attrs.filename
                ),
            ));
        }
        Ok(header)
    }
}

/// A complete plan produced before any file is written.
pub struct Plan {
    files: Vec<PlannedFile>,
}

impl Plan {
    /// Builds the serializable report for this plan.
    pub fn report(&self) -> Report {
        Report {
            files: self
                .files
                .iter()
                .map(|file| {
                    let path = file.relative_path.to_string_lossy();
                    #[cfg(windows)]
                    let path = path.replace('\\', "/");
                    #[cfg(not(windows))]
                    let path = path.into_owned();
                    FileOutcome {
                        path,
                        outcome: file.outcome,
                    }
                })
                .collect(),
        }
    }

    /// Writes every planned edit directly to its source file.
    ///
    /// Callers must ensure that selected files are not modified between planning and applying.
    pub fn apply(&self) -> Result<(), Error> {
        for file in &self.files {
            let Some(updated) = &file.updated else {
                continue;
            };
            fs::write(&file.absolute_path, updated).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot write {}", file.absolute_path.display()),
                )
                .with_source(err)
            })?;
        }
        Ok(())
    }
}

/// The analysis and optional replacement planned for one file.
struct PlannedFile {
    absolute_path: PathBuf,
    relative_path: PathBuf,
    outcome: Outcome,
    updated: Option<Vec<u8>>,
}

impl PlannedFile {
    fn unsupported(absolute_path: PathBuf, relative_path: PathBuf) -> Self {
        Self {
            absolute_path,
            relative_path,
            outcome: Outcome::Unsupported,
            updated: None,
        }
    }
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
