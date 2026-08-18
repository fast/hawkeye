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

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Instant;
use std::time::SystemTime;

use ignore::WalkBuilder;
use ignore::overrides::Override;
use ignore::overrides::OverrideBuilder;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

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
        let (selection, exclusions) = compile_patterns(&root, &files.includes, &files.excludes)?;

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

    fn style(&self, name: &str) -> &StyleConfig {
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
        styles: &BTreeMap<String, StyleConfig>,
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

struct FileAnalysis {
    outcome: FileOutcome,
    replacement: Option<Replacement>,
}

struct Replacement {
    range: Range<usize>,
    value: String,
}

impl Replacement {
    fn apply(&self, input: &mut String) {
        input.replace_range(self.range.clone(), &self.value);
    }
}

impl Engine {
    fn analyze(&self, rule: &Rule, input: &str, header: &str, target: Target) -> FileAnalysis {
        let offset = preamble_offset(input);
        let header_start = skip_blank_lines(input, offset);
        let eol = detect_eol(input);
        let rendered = {
            let mut value = self.style(&rule.style_out).render(header, eol);
            value.push_str(eol);
            value.push_str(eol);
            value
        };

        let matches = rule
            .styles_in
            .iter()
            .filter_map(|name| {
                self.style(name)
                    .parse(input, header_start)
                    .map(|candidate| (name.as_str(), candidate))
            })
            .filter(|(_, candidate)| has_keywords(&candidate.body, &self.keywords));

        let candidate = match unique_style_match(matches) {
            Ok(candidate) => candidate,
            Err(()) => {
                return FileAnalysis {
                    outcome: FileOutcome::Conflict,
                    replacement: None,
                };
            }
        };

        if let Some((style_name, candidate)) = candidate {
            let end = skip_blank_lines(input, candidate.range.end);
            let candidate_lines = candidate.body.lines().count();
            let header_lines = header.lines().count();
            if !safe_to_replace(&candidate.body, header, &self.keywords)
                || (candidate_lines < header_lines
                    && self
                        .styles
                        .values()
                        .any(|style| style.parse(input, end).is_some()))
            {
                return FileAnalysis {
                    outcome: FileOutcome::Conflict,
                    replacement: None,
                };
            }
            let range = offset..end;
            if target == Target::Absent {
                return FileAnalysis {
                    outcome: FileOutcome::Remove,
                    replacement: Some(Replacement {
                        range,
                        value: String::new(),
                    }),
                };
            }
            let clean = style_name == rule.style_out
                && candidate.body == header
                && input.get(range.clone()) == Some(rendered.as_str());
            if clean {
                FileAnalysis {
                    outcome: FileOutcome::Clean,
                    replacement: None,
                }
            } else {
                FileAnalysis {
                    outcome: FileOutcome::Replace,
                    replacement: Some(Replacement {
                        range,
                        value: rendered,
                    }),
                }
            }
        } else if self.styles.values().any(|style| {
            style
                .parse(input, header_start)
                .is_some_and(|candidate| has_keywords(&candidate.body, &self.keywords))
        }) {
            FileAnalysis {
                outcome: FileOutcome::Conflict,
                replacement: None,
            }
        } else if target == Target::Absent {
            FileAnalysis {
                outcome: FileOutcome::Clean,
                replacement: None,
            }
        } else {
            FileAnalysis {
                outcome: FileOutcome::Add,
                replacement: Some(Replacement {
                    range: offset..header_start,
                    value: rendered,
                }),
            }
        }
    }
}

fn unique_style_match<'a>(
    mut matches: impl Iterator<Item = (&'a str, StyleMatch)>,
) -> Result<Option<(&'a str, StyleMatch)>, ()> {
    let Some((style_name, first)) = matches.next() else {
        return Ok(None);
    };
    for (_, candidate) in matches {
        if candidate.range != first.range || candidate.body != first.body {
            return Err(());
        }
    }
    Ok(Some((style_name, first)))
}

fn has_keywords(body: &str, keywords: &[String]) -> bool {
    let folded = body.to_lowercase();
    keywords.iter().all(|keyword| folded.contains(keyword))
}

fn safe_to_replace(candidate: &str, header: &str, keywords: &[String]) -> bool {
    candidate.lines().count() <= header.lines().count()
        && candidate
            .lines()
            .zip(header.lines())
            .all(|(candidate, header)| {
                if candidate == header {
                    return true;
                }
                let folded = candidate.to_lowercase();
                keywords
                    .iter()
                    .any(|keyword| folded.contains(keyword.as_str()))
            })
}

fn detect_eol(input: &str) -> &'static str {
    if input
        .find('\n')
        .is_some_and(|index| index > 0 && input.as_bytes()[index - 1] == b'\r')
    {
        "\r\n"
    } else {
        "\n"
    }
}

fn preamble_offset(input: &str) -> usize {
    let mut position = usize::from(input.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
    let Some((first, range)) = lines(input, position).next() else {
        return position;
    };
    let lower = first.to_ascii_lowercase();
    if (first.starts_with("#!") && !first.starts_with("#!["))
        || (lower.starts_with("<?xml") && lower.ends_with("?>"))
        || lower
            .strip_prefix("<?php")
            .is_some_and(|tail| tail.chars().next().is_none_or(char::is_whitespace))
        || lower.starts_with("<!doctype ")
        || first.starts_with("%YAML")
        || first.starts_with("%TAG")
    {
        position = range.end;
    }

    while let Some((line, range)) = lines(input, position).next() {
        if !line.starts_with("%YAML") && !line.starts_with("%TAG") {
            break;
        }
        position = range.end;
    }

    for _ in 0..2 {
        let Some((line, range)) = lines(input, position).next() else {
            break;
        };
        let lower = line.to_ascii_lowercase();
        let magic = line.starts_with('#')
            && (lower.contains("coding:")
                || lower.contains("coding=")
                || lower.contains("frozen_string_literal:")
                || lower.contains("-*-"));
        if !magic {
            break;
        }
        position = range.end;
    }
    position
}

fn skip_blank_lines(input: &str, mut position: usize) -> usize {
    for (line, range) in lines(input, position) {
        if !line.trim().is_empty() {
            break;
        }
        position = range.end;
    }
    position
}

impl Engine {
    fn discover(&self, repo: Option<&GitRepo>) -> Result<Vec<PathBuf>, Error> {
        let started = Instant::now();
        let mut files = BTreeSet::new();

        if self.git.ignore != FeatureMode::Disable
            && let Some(repo) = repo
        {
            for path in repo.list_files(&self.root)? {
                if self.selection.matched(&path, false).is_whitelist() {
                    files.insert(path);
                }
            }
            log::debug!(
                "selected {} files through the Git index in {:?}",
                files.len(),
                started.elapsed()
            );
        } else {
            walk(
                &self.root,
                &self.selection,
                &self.exclusions,
                self.git.ignore,
                &mut files,
            )?;
            log::debug!(
                "selected {} files through a filesystem walk in {:?}",
                files.len(),
                started.elapsed()
            );
        }

        if let Some(header_path) = &self.header_path {
            let mut selected = Vec::with_capacity(files.len());
            for path in files {
                let absolute_path = self.root.join(&path);
                let is_header =
                    same_file::is_same_file(&absolute_path, header_path).map_err(|err| {
                        Error::new(
                            ErrorKind::Unexpected,
                            format!(
                                "cannot compare {} with header template {}",
                                absolute_path.display(),
                                header_path.display()
                            ),
                        )
                        .with_source(err)
                    })?;
                if !is_header {
                    selected.push(path);
                }
            }
            return Ok(selected);
        }
        Ok(files.into_iter().collect())
    }
}

fn compile_patterns(
    root: &Path,
    includes: &[String],
    excludes: &[String],
) -> Result<(Override, Override), Error> {
    let mut builder = OverrideBuilder::new(root);
    if includes.is_empty() {
        builder.add("**").map_err(selection_error)?;
    } else {
        for pattern in includes {
            builder.add(pattern).map_err(selection_error)?;
        }
    }
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    let selection = builder.build().map_err(selection_error)?;

    let mut builder = OverrideBuilder::new(root);
    builder.add("!.git").map_err(selection_error)?;
    builder.add("!.git/**").map_err(selection_error)?;
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(selection_error)?;
    }
    let exclusions = builder.build().map_err(selection_error)?;
    Ok((selection, exclusions))
}

fn selection_error(source: ignore::Error) -> Error {
    Error::new(
        ErrorKind::ConfigInvalid,
        "invalid files.includes or files.excludes pattern",
    )
    .with_source(source)
}

fn walk(
    root: &Path,
    selection: &Override,
    exclusions: &Override,
    git_ignore: FeatureMode,
    files: &mut BTreeSet<PathBuf>,
) -> Result<(), Error> {
    let use_git_ignore = git_ignore != FeatureMode::Disable;
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(use_git_ignore)
        .git_global(use_git_ignore)
        .git_exclude(use_git_ignore)
        .parents(use_git_ignore)
        .follow_links(false)
        .overrides(exclusions.clone())
        .build();
    for entry in walker {
        let entry = entry.map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot discover files").with_source(err)
        })?;
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|kind| kind.is_file() || (kind.is_symlink() && path.is_file()))
        {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "file walker returned path outside files.root: {}",
                    path.display()
                ),
            )
        })?;
        if selection.matched(relative, false).is_whitelist() {
            files.insert(relative.to_path_buf());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleMatch {
    range: Range<usize>,
    body: String,
}

impl StyleConfig {
    fn render(&self, body: &str, eol: &str) -> String {
        let mut output = String::new();
        match self {
            Self::Line {
                prefix,
                suffix,
                pad_lines,
            } => {
                let lines = body.split('\n');
                let width = if *pad_lines {
                    lines
                        .clone()
                        .map(|line| line.chars().count())
                        .max()
                        .unwrap_or(0)
                } else {
                    0
                };
                for (index, line) in lines.enumerate() {
                    if index > 0 {
                        output.push_str(eol);
                    }
                    let line_start = output.len();
                    output.push_str(prefix);
                    output.push_str(line);
                    if *pad_lines {
                        output.extend(std::iter::repeat_n(
                            ' ',
                            width.saturating_sub(line.chars().count()),
                        ));
                    }
                    output.push_str(suffix);
                    if suffix.is_empty() {
                        let len = output[line_start..].trim_end_matches([' ', '\t']).len();
                        output.truncate(line_start + len);
                    }
                }
            }
            Self::Block {
                start,
                prefix,
                suffix,
                end,
            } => {
                output.push_str(start);
                for line in body.split('\n') {
                    output.push_str(eol);
                    let line_start = output.len();
                    output.push_str(prefix);
                    output.push_str(line);
                    output.push_str(suffix);
                    if suffix.is_empty() {
                        let len = output[line_start..].trim_end_matches([' ', '\t']).len();
                        output.truncate(line_start + len);
                    }
                }
                output.push_str(eol);
                output.push_str(end);
            }
        }
        output
    }

    fn parse(&self, input: &str, start: usize) -> Option<StyleMatch> {
        match self {
            Self::Line {
                prefix,
                suffix,
                pad_lines,
            } => parse_line_style(input, start, prefix, suffix, *pad_lines),
            Self::Block {
                start: opening,
                prefix,
                suffix,
                end: closing,
            } => parse_block_style(input, start, opening, prefix, suffix, closing),
        }
    }
}

fn parse_line_style(
    input: &str,
    start: usize,
    prefix: &str,
    suffix: &str,
    pad_lines: bool,
) -> Option<StyleMatch> {
    let mut end = start;
    let mut body = Vec::new();
    for (line, raw_range) in lines(input, start) {
        let Some(content) = strip_affixes(line, prefix, suffix, pad_lines) else {
            break;
        };
        body.push(content);
        end = raw_range.start + line.len();
    }
    if body.is_empty() {
        None
    } else {
        Some(StyleMatch {
            range: start..end,
            body: body.join("\n"),
        })
    }
}

fn parse_block_style(
    input: &str,
    start: usize,
    opening: &str,
    prefix: &str,
    suffix: &str,
    closing: &str,
) -> Option<StyleMatch> {
    let mut lines = lines(input, start);
    let (first, _) = lines.next()?;
    if first != opening {
        return None;
    }

    let mut body = Vec::new();
    for (content, raw_range) in lines {
        if content == closing {
            let end = raw_range.start + content.len();
            return Some(StyleMatch {
                range: start..end,
                body: body.join("\n"),
            });
        }
        body.push(strip_affixes(content, prefix, suffix, false)?);
    }
    None
}

fn strip_affixes(line: &str, prefix: &str, suffix: &str, pad_lines: bool) -> Option<String> {
    let prefix_without_space = prefix.trim_end();
    let body = if line == prefix_without_space && suffix.is_empty() {
        ""
    } else {
        line.strip_prefix(prefix)?
    };
    let body = if suffix.is_empty() {
        body
    } else {
        body.strip_suffix(suffix)?
    };
    Some(if pad_lines {
        body.trim_end().to_owned()
    } else {
        body.to_owned()
    })
}

/// Iterates line contents without terminators and their full byte ranges in the input.
fn lines(input: &str, start: usize) -> impl Iterator<Item = (&str, Range<usize>)> {
    let mut position = start;
    input[start..].split_inclusive('\n').map(move |line| {
        let raw_range = position..position + line.len();
        position = raw_range.end;
        let content = if let Some(content) = line.strip_suffix('\n') {
            content.strip_suffix('\r').unwrap_or(content)
        } else {
            line
        };
        (content, raw_range)
    })
}

struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    fn discover(root: &Path) -> Result<Self, Error> {
        let started = Instant::now();
        let output = match git_command(root)
            .args(["rev-parse", "--show-toplevel"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    format!("Git cannot be started for {}", root.display()),
                )
                .with_source(err));
            }
        };

        if !output.status.success() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("{} is not a usable Git worktree", root.display()),
            )
            .with_source(stderr(&output)));
        }

        let path = output
            .stdout
            .strip_suffix(b"\n")
            .unwrap_or(output.stdout.as_slice());
        if path.is_empty() {
            return Err(Error::new(
                ErrorKind::Unexpected,
                "Git returned an empty repository root",
            ));
        }
        let root = path_from_git_bytes(path).canonicalize().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot resolve repository root").with_source(err)
        })?;
        log::debug!(
            "discovered Git repository {} in {:?}",
            root.display(),
            started.elapsed()
        );
        Ok(Self { root })
    }

    fn list_files(&self, scan_root: &Path) -> Result<Vec<PathBuf>, Error> {
        let relative_root = scan_root.strip_prefix(&self.root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "files.root {} is outside repository {}",
                    scan_root.display(),
                    self.root.display()
                ),
            )
        })?;
        let pathspec = if relative_root.as_os_str().is_empty() {
            OsString::from(".")
        } else {
            relative_root.as_os_str().to_owned()
        };
        let output = self.output(
            "cannot list Git worktree files",
            [
                OsString::from("ls-files"),
                OsString::from("--cached"),
                OsString::from("--others"),
                OsString::from("--exclude-standard"),
                OsString::from("-z"),
                OsString::from("--"),
                pathspec,
            ],
        )?;

        let mut files = Vec::new();
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let path = self.root.join(path_from_git_bytes(record));
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot read metadata for {}", path.display()),
                    )
                    .with_source(err));
                }
            };
            let file_type = metadata.file_type();
            if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
                let relative = path.strip_prefix(scan_root).map_err(|_| {
                    Error::new(
                        ErrorKind::Unexpected,
                        format!(
                            "Git returned path outside files.root {}: {}",
                            scan_root.display(),
                            path.display()
                        ),
                    )
                })?;
                files.push(relative.to_path_buf());
            }
        }
        Ok(files)
    }

    fn is_shallow(&self) -> Result<bool, Error> {
        let output = self.output(
            "cannot inspect whether the Git repository is shallow",
            ["rev-parse", "--is-shallow-repository"],
        )?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(Error::new(
                ErrorKind::Unexpected,
                format!("Git returned an invalid shallow-repository value: {value:?}"),
            )),
        }
    }
}

#[cfg(unix)]
fn git_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn git_path(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

impl GitRepo {
    fn output<I, S>(&self, operation: &'static str, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output_unchecked(arguments)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(Error::new(ErrorKind::Unexpected, operation).with_source(stderr(&output)))
        }
    }

    fn output_unchecked<I, S>(&self, arguments: I) -> Result<Output, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let output = git_command(&self.root)
            .args(&arguments)
            .output()
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(err)
            })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        Ok(output)
    }

    fn read_stdout<I, S, Value>(
        &self,
        operation: &'static str,
        arguments: I,
        input: Option<&[u8]>,
        read: impl FnOnce(&mut dyn BufRead) -> Result<Value, Error>,
    ) -> Result<Value, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect::<Vec<_>>();
        // File-backed stdin and stderr let Git consume and produce both while stdout is parsed
        // synchronously, without another thread or the risk of a full pipe blocking the child.
        let mut stderr = tempfile::tempfile().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot create Git stderr buffer").with_source(err)
        })?;
        let stderr_writer = stderr.try_clone().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot clone Git stderr buffer").with_source(err)
        })?;
        let stdin = if let Some(input) = input {
            let mut file = tempfile::tempfile().map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot create Git stdin buffer").with_source(err)
            })?;
            file.write_all(input).map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot write Git stdin buffer").with_source(err)
            })?;
            file.seek(SeekFrom::Start(0)).map_err(|err| {
                Error::new(ErrorKind::Unexpected, "cannot rewind Git stdin buffer").with_source(err)
            })?;
            Stdio::from(file)
        } else {
            Stdio::null()
        };
        let started = Instant::now();
        let mut child = git_command(&self.root)
            .args(&arguments)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr_writer))
            .spawn()
            .map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot execute Git {arguments:?}"),
                )
                .with_source(err)
            })?;
        let stdout = child
            .stdout
            .take()
            .expect("Git stdout was configured as a pipe");
        let parsed = read(&mut BufReader::new(stdout));
        if parsed.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot wait for Git").with_source(err)
        })?;
        log::debug!("Git {:?} completed in {:?}", arguments, started.elapsed());
        let value = parsed?;
        if status.success() {
            return Ok(value);
        }

        stderr.seek(SeekFrom::Start(0)).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot rewind Git stderr").with_source(err)
        })?;
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git stderr").with_source(err)
        })?;
        let source = failure(&status, &bytes);
        Err(Error::new(ErrorKind::Unexpected, operation).with_source(source))
    }
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .env("GIT_LITERAL_PATHSPECS", "1");
    command
}

fn stderr(output: &Output) -> String {
    failure(&output.status, &output.stderr)
}

fn failure(status: &std::process::ExitStatus, bytes: &[u8]) -> String {
    let message = String::from_utf8_lossy(bytes).trim().to_owned();
    if message.is_empty() {
        format!("Git exited with {status}")
    } else {
        message
    }
}

#[derive(Debug, Clone, Default)]
struct GitFileHistory {
    created_year: Option<i16>,
    modified_year: Option<i16>,
    authors: BTreeSet<String>,
}

impl GitFileHistory {
    fn record(&mut self, year: i16, author: &str) {
        self.created_year = Some(self.created_year.map_or(year, |value| value.min(year)));
        self.modified_year = Some(self.modified_year.map_or(year, |value| value.max(year)));
        if !author.trim().is_empty() {
            self.authors.insert(author.to_owned());
        }
    }

    fn record_worktree(&mut self, year: i16, author: Option<&str>) {
        self.created_year.get_or_insert(year);
        self.modified_year = Some(year);
        if let Some(author) = author.filter(|value| !value.trim().is_empty()) {
            self.authors.insert(author.to_owned());
        }
    }
}

impl GitRepo {
    fn file_history<'a>(
        &self,
        scan_root: &Path,
        files: impl IntoIterator<Item = &'a Path>,
    ) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
        let relative_root = scan_root.strip_prefix(&self.root).map_err(|_| {
            Error::new(
                ErrorKind::Unexpected,
                format!(
                    "files.root {} is outside repository {}",
                    scan_root.display(),
                    self.root.display()
                ),
            )
        })?;
        let selected = files
            .into_iter()
            .map(|path| (git_path(&relative_root.join(path)), path.to_path_buf()))
            .collect::<HashMap<_, _>>();
        if selected.is_empty() {
            return Ok(HashMap::new());
        }

        let current_year = Timestamp::now().to_zoned(TimeZone::UTC).year();
        let started = Instant::now();
        let author = self.author_name()?;
        let mut history = self.read_history(&selected)?;
        self.apply_worktree_status(&selected, current_year, author.as_deref(), &mut history)?;

        for path in selected.values() {
            history.entry(path.clone()).or_insert_with(|| {
                let mut history = GitFileHistory::default();
                history.record_worktree(current_year, author.as_deref());
                history
            });
        }
        log::debug!(
            "resolved Git file history for {} files in {:?}",
            selected.len(),
            started.elapsed()
        );
        Ok(history)
    }

    fn has_head(&self) -> Result<bool, Error> {
        let output = self.output_unchecked(["rev-parse", "--verify", "--quiet", "HEAD"])?;
        if output.status.success() {
            Ok(true)
        } else if output.status.code() == Some(1) {
            Ok(false)
        } else {
            Err(Error::new(ErrorKind::Unexpected, "cannot inspect Git HEAD")
                .with_source(stderr(&output)))
        }
    }

    fn author_name(&self) -> Result<Option<String>, Error> {
        let output = self.output_unchecked(["config", "--get", "user.name"])?;
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(
                Error::new(ErrorKind::Unexpected, "cannot read Git author name")
                    .with_source(stderr(&output)),
            );
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!value.is_empty()).then_some(value))
    }

    fn read_history(
        &self,
        selected: &HashMap<Vec<u8>, PathBuf>,
    ) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
        if !self.has_head()? {
            return Ok(HashMap::new());
        }
        let mut arguments = vec![
            "-c",
            "core.quotepath=false",
            "log",
            "--full-history",
            "--no-merges",
            "--no-renames",
            "--format=%x00%x00%cI%x00%an",
            "--name-only",
            "-z",
        ];
        let pathspecs = history_pathspecs(selected);
        if pathspecs.is_some() {
            arguments.push("--stdin");
        } else {
            // `git log --stdin` is line-delimited. Fall back to an unfiltered traversal for the
            // rare repository containing a selected path with an embedded line ending.
            arguments.push("--");
        }
        // Empty NUL-delimited records cannot be file paths, so a pair unambiguously frames commit
        // metadata without relying on Git's quoting rules or reserving a valid path byte.
        self.read_stdout(
            "cannot read Git history",
            arguments,
            pathspecs.as_deref(),
            |reader| parse_history(reader, selected),
        )
    }

    fn apply_worktree_status(
        &self,
        selected: &HashMap<Vec<u8>, PathBuf>,
        year: i16,
        author: Option<&str>,
        history: &mut HashMap<PathBuf, GitFileHistory>,
    ) -> Result<(), Error> {
        let output = self.output(
            "cannot inspect Git worktree status",
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        let mut records = output.stdout.split(|byte| *byte == 0);
        while let Some(record) = records.next() {
            if record.len() < 4 {
                continue;
            }
            let status = &record[..2];
            if let Some(selected_path) = selected.get(&record[3..]) {
                history
                    .entry(selected_path.clone())
                    .or_default()
                    .record_worktree(year, author);
            }
            if status.contains(&b'R') || status.contains(&b'C') {
                records.next();
            }
        }
        Ok(())
    }
}

fn history_pathspecs(selected: &HashMap<Vec<u8>, PathBuf>) -> Option<Vec<u8>> {
    let mut input = b"--\n".to_vec();
    for path in selected.keys() {
        if path.contains(&b'\n') || path.contains(&b'\r') {
            return None;
        }
        input.extend_from_slice(path);
        input.push(b'\n');
    }
    Some(input)
}

fn parse_history(
    reader: &mut dyn BufRead,
    selected: &HashMap<Vec<u8>, PathBuf>,
) -> Result<HashMap<PathBuf, GitFileHistory>, Error> {
    #[derive(Clone, Copy)]
    enum State {
        Paths,
        Author(Option<i16>),
    }

    let mut current: Option<(i16, String)> = None;
    let mut expecting_first_path = false;
    let mut empty_records = 0;
    let mut state = State::Paths;
    let mut result = HashMap::<PathBuf, GitFileHistory>::new();
    let mut record = Vec::new();
    loop {
        record.clear();
        let read = reader.read_until(0, &mut record).map_err(|err| {
            Error::new(ErrorKind::Unexpected, "cannot read Git history").with_source(err)
        })?;
        if read == 0 {
            break;
        }
        if record.last() == Some(&0) {
            record.pop();
        }
        let record = record.as_slice();
        if record.is_empty() {
            empty_records += 1;
            continue;
        }
        if matches!(state, State::Paths) && empty_records >= 2 {
            let year = record
                .get(..4)
                .and_then(|year| std::str::from_utf8(year).ok())
                .and_then(|year| year.parse::<i16>().ok());
            current = None;
            expecting_first_path = false;
            empty_records = 0;
            state = State::Author(year);
            continue;
        }
        if let State::Author(year) = state {
            current = year.map(|year| (year, String::from_utf8_lossy(record).into_owned()));
            expecting_first_path = true;
            empty_records = 0;
            state = State::Paths;
            continue;
        }
        empty_records = 0;
        let path = if expecting_first_path {
            expecting_first_path = false;
            // `--name-only -z` inserts one newline between the pretty header and its first path.
            record.strip_prefix(b"\n").unwrap_or(record)
        } else {
            record
        };
        if path.is_empty() {
            continue;
        }
        let Some(path) = selected.get(path) else {
            continue;
        };
        if let Some((year, author)) = &current {
            result
                .entry(path.clone())
                .or_default()
                .record(*year, author);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn history_parser_handles_records_across_small_buffers() {
        let selected_path = PathBuf::from("/repo/src/main.rs");
        let selected = HashMap::from([(b"src/main.rs".to_vec(), selected_path.clone())]);
        let history = b"\x00\x002024-01-02T00:00:00Z\x00Alice\x00\nsrc/main.rs\x00other.rs\x00\x00\x002020-03-04T00:00:00Z\x00Bob\x00\nsrc/main.rs\x00";
        let mut reader = BufReader::with_capacity(3, Cursor::new(history));

        let attrs = parse_history(&mut reader, &selected).expect("parse history");
        let attrs = attrs.get(&selected_path).expect("selected file attributes");
        assert_eq!(attrs.created_year, Some(2020));
        assert_eq!(attrs.modified_year, Some(2024));
        assert_eq!(
            attrs.authors,
            BTreeSet::from(["Alice".to_owned(), "Bob".to_owned()])
        );
    }
}
