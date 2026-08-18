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
mod git;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use ignore::overrides::Override;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use self::git::GitFileHistory;
use self::git::GitRepo;
use crate::Error;
use crate::ErrorKind;
use crate::builtin;
use crate::config::Config;
use crate::config::FeatureMode;
use crate::config::GitConfig;
use crate::config::RuleConfig;
use crate::config::StyleConfig;
use crate::report::FileOutcome;
use crate::report::FileReport;
use crate::report::Report;
use crate::template::HeaderTemplate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAttrs {
    filename: String,
    disk_file_created_year: Option<i16>,
    disk_file_modified_year: Option<i16>,
    git_file_created_year: Option<i16>,
    git_file_modified_year: Option<i16>,
    git_authors: Vec<String>,
}

impl FileAttrs {
    fn new(path: &Path, git: Option<&GitFileHistory>) -> Result<Self, Error> {
        let metadata = fs::metadata(path).map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot read metadata for {}", path.display()),
            )
            .with_source(err)
        })?;
        Ok(Self {
            filename: path
                .file_name()
                .expect("selected files have a filename")
                .to_string_lossy()
                .into_owned(),
            disk_file_created_year: metadata.created().ok().and_then(file_time_to_year),
            disk_file_modified_year: metadata.modified().ok().and_then(file_time_to_year),
            git_file_created_year: git.and_then(|history| history.created_year),
            git_file_modified_year: git.and_then(|history| history.modified_year),
            git_authors: git
                .map(|history| history.authors.iter().cloned().collect())
                .unwrap_or_default(),
        })
    }
}

fn file_time_to_year(time: SystemTime) -> Option<i16> {
    let ts = Timestamp::try_from(time).ok()?;
    Some(ts.to_zoned(TimeZone::system()).year())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderTarget {
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
    styles: BTreeMap<String, StyleConfig>,
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
        log::debug!(
            "configured file selection: root={}, includes={:?}, excludes={:?}, git.ignore={:?}, git.file_attrs={:?}",
            root.display(),
            files.includes,
            files.excludes,
            git.ignore,
            git.file_attrs
        );
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
            let mut styles = builtin::STYLES.clone();
            for (name, style) in configured_styles {
                if styles.contains_key(&name) {
                    log::warn!("custom style {name:?} overrides a built-in style of the same name");
                }
                styles.insert(name, style);
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
        log::debug!("resolved {} styles and {} rules", styles.len(), rules.len());

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
        Ok(self.edits(HeaderTarget::Present)?.report)
    }

    /// Prepares edits that make selected headers canonical.
    pub fn format(&self) -> Result<Edits, Error> {
        self.edits(HeaderTarget::Present)
    }

    /// Prepares edits that remove recognized headers from selected files.
    pub fn remove(&self) -> Result<Edits, Error> {
        self.edits(HeaderTarget::Absent)
    }

    fn edits(&self, target: HeaderTarget) -> Result<Edits, Error> {
        let git_mode = self.git.ignore.combine(self.git.file_attrs);
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
        let paths = self.discover(repo.as_ref())?;
        log::trace!("selected files: {paths:?}");
        let selected = paths
            .into_iter()
            .map(|path| {
                let rule = self.rules.iter().find(|rule| rule.matches(&path));
                (path, rule)
            })
            .collect::<Vec<_>>();
        let supported = selected
            .iter()
            .filter_map(|(path, rule)| rule.is_some().then_some(path.as_path()))
            .collect::<Vec<_>>();
        let git_history = if self.git.file_attrs == FeatureMode::Disable || supported.is_empty() {
            None
        } else if let Some(repo) = repo.as_ref() {
            if repo.is_shallow()? {
                let message = "Git file attributes require complete history, but the repository is shallow; fetch complete history first";
                if self.git.file_attrs == FeatureMode::Auto {
                    log::warn!("{message}; continuing with Git file attributes disabled");
                    None
                } else {
                    return Err(Error::new(ErrorKind::Unsupported, message));
                }
            } else {
                Some(repo.file_history(&self.root, supported)?)
            }
        } else {
            debug_assert_ne!(self.git.file_attrs, FeatureMode::Enable);
            None
        };
        let mut report = Report {
            files: Vec::with_capacity(selected.len()),
        };
        let mut file_edits = Vec::new();

        for (relative_path, rule) in selected {
            let Some(rule) = rule else {
                log::debug!(
                    "{} has no matching rule; reporting it as unsupported",
                    relative_path.display()
                );
                report.files.push(FileReport {
                    path: relative_path,
                    outcome: FileOutcome::Unsupported,
                });
                continue;
            };

            let path = self.root.join(&relative_path);
            let original = fs::read(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            let Ok(input) = std::str::from_utf8(&original) else {
                log::debug!(
                    "{} is not UTF-8 text; reporting it as unsupported",
                    relative_path.display()
                );
                report.files.push(FileReport {
                    path: relative_path,
                    outcome: FileOutcome::Unsupported,
                });
                continue;
            };
            let file_attrs = FileAttrs::new(
                &path,
                git_history
                    .as_ref()
                    .and_then(|history| history.get(&relative_path)),
            )?;
            let header = self.render_header(&file_attrs)?;
            let outcome = match self.analyze(rule, input, &header, target) {
                FileAnalysis::Clean => FileOutcome::Clean,
                FileAnalysis::Add(replacement) => {
                    file_edits.push(FileEdit { path, replacement });
                    FileOutcome::Add
                }
                FileAnalysis::Replace(replacement) => {
                    file_edits.push(FileEdit { path, replacement });
                    FileOutcome::Replace
                }
                FileAnalysis::Remove(replacement) => {
                    file_edits.push(FileEdit { path, replacement });
                    FileOutcome::Remove
                }
                FileAnalysis::Conflict => FileOutcome::Conflict,
            };
            report.files.push(FileReport {
                path: relative_path,
                outcome,
            });
        }

        Ok(Edits {
            report,
            files: file_edits,
        })
    }

    fn render_header(&self, attrs: &FileAttrs) -> Result<String, Error> {
        let header = self.template.render(&self.props, attrs)?;
        let folded = header.to_lowercase();
        if let Some(keyword) = self
            .keywords
            .iter()
            .find(|keyword| !folded.contains(keyword.as_str()))
        {
            let filename = &attrs.filename;
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                format!(
                    "header template output for {filename:?} does not contain recognition keyword {keyword:?}"
                ),
            ));
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
        for FileEdit { path, replacement } in self.files {
            let mut input = fs::read_to_string(&path).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot read {}", path.display()),
                )
                .with_source(err)
            })?;
            input.replace_range(replacement.range, &replacement.text);
            fs::write(&path, input).map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot write {}", path.display()),
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
        source: &str,
        config: RuleConfig,
        styles: &BTreeMap<String, StyleConfig>,
    ) -> Result<Self, Error> {
        let RuleConfig {
            extensions,
            filenames,
            style_out,
            styles_in,
        } = config;
        let styles_in = if styles_in.is_empty() {
            vec![style_out.clone()]
        } else {
            styles_in
        };
        let mut input_styles = Vec::with_capacity(styles_in.len());
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
            input_styles.push(name);
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
            styles_in: input_styles,
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

struct Replacement {
    range: Range<usize>,
    text: String,
}

enum FileAnalysis {
    Clean,
    Add(Replacement),
    Replace(Replacement),
    Remove(Replacement),
    Conflict,
}
