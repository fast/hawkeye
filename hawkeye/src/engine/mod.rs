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
use std::path::Path;
use std::path::PathBuf;

use crate::Error;
use crate::ErrorKind;
use crate::attrs::FileAttrs;
use crate::attrs::FileAttrsResolver;
use crate::config::Config;
use crate::config::GitConfig;
use crate::edit::Edit;
use crate::git::GitRepo;
use crate::report::FileOutcome;
use crate::report::Mode;
use crate::report::Report;
use crate::report::Status;
use crate::style::Style;
use crate::style::builtin_styles;
use crate::template::HeaderTemplate;
use crate::writer::validate_source;
use crate::writer::write_atomic;

/// A reusable HawkEye runtime built from one configuration.
pub struct Engine {
    root: PathBuf,
    header_path: Option<PathBuf>,
    includes: Vec<String>,
    excludes: Vec<String>,
    props: BTreeMap<String, toml::Value>,
    git: GitConfig,
    keywords: Vec<String>,
    template: HeaderTemplate,
    styles: BTreeMap<String, Style>,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    extensions: BTreeSet<String>,
    filenames: BTreeSet<String>,
    style_out: String,
    styles_in: Vec<String>,
}

struct Analysis {
    status: Status,
    edit: Option<Edit>,
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

        let (source, header_path) = if let Some(source) = header.text {
            (source, None)
        } else if let Some(path) = header.path {
            let path = path.canonicalize().map_err(|err| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot resolve header template {}", path.display()),
                )
                .with_source(err)
            })?;
            (
                fs::read_to_string(&path).map_err(|err| {
                    Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot read header template {}", path.display()),
                    )
                    .with_source(err)
                })?,
                Some(path),
            )
        } else if let Some(key) = header.builtin {
            (
                builtin_header(&key).ok_or_else(|| {
                    Error::new(
                        ErrorKind::ConfigInvalid,
                        format!(
                            "unknown header.builtin {key:?}; available values are Apache-2.0, Apache-2.0-ASF, and Elastic-2.0"
                        ),
                    )
                })?
                .to_owned(),
                None,
            )
        } else {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                "header source is missing",
            ));
        };
        let template = HeaderTemplate::new(source)?;

        let mut styles = builtin_styles();
        for (name, style) in configured_styles {
            if styles.contains_key(&name) {
                log::warn!("custom style {name:?} overrides a built-in style of the same name");
            }
            styles.insert(name.clone(), Style::from_config(name, style));
        }

        let mut rules = configured_rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                resolve_rule(
                    &format!("rules[{index}]"),
                    &rule.extensions,
                    &rule.filenames,
                    &rule.style_out,
                    &rule.styles_in,
                    &styles,
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        rules.extend(default_rules(&styles)?);

        Ok(Self {
            root,
            header_path,
            includes: files.includes,
            excludes: files.excludes,
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
    pub fn plan(&self, mode: Mode) -> Result<Plan, Error> {
        let git = self.git;
        let repo = GitRepo::discover(&self.root, git.ignore.combine(git.file_attrs))?;
        let paths = self.discover(repo.as_ref())?;
        let attrs = FileAttrsResolver::new(&paths, git.file_attrs, repo.as_ref())?;
        let mut files = Vec::with_capacity(paths.len());

        for path in paths {
            let relative = path
                .strip_prefix(&self.root)
                .expect("discovery only returns paths inside files.root")
                .to_path_buf();
            if self.rule_for(&relative).is_none() {
                files.push(PlannedFile::unsupported(path, relative));
                continue;
            }

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
            let analysis = self.analyze(&relative, input, &header, mode);
            let updated = analysis
                .edit
                .as_ref()
                .map(|edit| edit.apply(input))
                .transpose()?
                .filter(|output| output.as_bytes() != original)
                .map(String::into_bytes);
            let original = updated.as_ref().map(|_| original);
            files.push(PlannedFile {
                absolute_path: path,
                relative_path: relative,
                status: analysis.status,
                original,
                updated,
            });
        }

        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
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

/// A complete, deterministic operation plan produced before any file is written.
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
                .map(|file| FileOutcome {
                    path: file.relative_path.clone(),
                    status: file.status,
                    changed: file.updated.is_some(),
                })
                .collect(),
        }
    }

    /// Atomically applies every planned edit after checking for stale inputs.
    pub fn apply(&self) -> Result<(), Error> {
        for file in &self.files {
            let (Some(original), Some(_)) = (&file.original, &file.updated) else {
                continue;
            };
            validate_source(&file.absolute_path, original)?;
        }
        for file in &self.files {
            let (Some(original), Some(updated)) = (&file.original, &file.updated) else {
                continue;
            };
            write_atomic(&file.absolute_path, original, updated)?;
        }
        Ok(())
    }
}

/// The analysis and optional replacement planned for one file.
struct PlannedFile {
    absolute_path: PathBuf,
    relative_path: PathBuf,
    status: Status,
    original: Option<Vec<u8>>,
    updated: Option<Vec<u8>>,
}

impl PlannedFile {
    fn unsupported(absolute_path: PathBuf, relative_path: PathBuf) -> Self {
        Self {
            absolute_path,
            relative_path,
            status: Status::Unsupported,
            original: None,
            updated: None,
        }
    }
}

impl Rule {
    fn matches(&self, path: &Path) -> bool {
        let Some(filename) = path.file_name() else {
            return false;
        };
        let filename = filename.to_string_lossy().to_lowercase();
        self.filenames.contains(&filename)
            || self
                .extensions
                .iter()
                .any(|extension| filename.ends_with(&format!(".{extension}")))
    }
}

fn builtin_header(key: &str) -> Option<&'static str> {
    match key {
        "Apache-2.0" => Some(include_str!("../builtin/Apache-2.0.txt")),
        "Apache-2.0-ASF" => Some(include_str!("../builtin/Apache-2.0-ASF.txt")),
        "Elastic-2.0" => Some(include_str!("../builtin/Elastic-2.0.txt")),
        _ => None,
    }
}

fn resolve_rule(
    location: &str,
    extensions: &[String],
    filenames: &[String],
    style_out: &str,
    styles_in: &[String],
    styles: &BTreeMap<String, Style>,
) -> Result<Rule, Error> {
    validate_style(location, style_out, styles)?;
    let mut accepted = Vec::with_capacity(styles_in.len() + 1);
    let mut seen = BTreeSet::new();
    for name in std::iter::once(style_out).chain(styles_in.iter().map(String::as_str)) {
        validate_style(location, name, styles)?;
        if seen.insert(name.to_owned()) {
            accepted.push(name.to_owned());
        }
    }
    Ok(Rule {
        extensions: extensions
            .iter()
            .map(|extension| extension.to_lowercase())
            .collect(),
        filenames: filenames
            .iter()
            .map(|filename| filename.to_lowercase())
            .collect(),
        style_out: style_out.to_owned(),
        styles_in: accepted,
    })
}

fn validate_style(
    location: &str,
    name: &str,
    styles: &BTreeMap<String, Style>,
) -> Result<(), Error> {
    if styles.contains_key(name) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::ConfigInvalid,
            format!("{location} references unknown style {name:?}"),
        ))
    }
}

fn default_rules(styles: &BTreeMap<String, Style>) -> Result<Vec<Rule>, Error> {
    let definitions: &[(&[&str], &[&str], &str, &[&str])] = &[
        (
            &[
                "as", "aj", "c", "cc", "cpp", "cs", "css", "go", "gradle", "groovy", "h", "hh",
                "hpp", "java", "fx", "js", "cjs", "jsx", "kt", "kts", "proto", "scala", "scss",
                "ts", "tsx", "v", "sv",
            ],
            &[],
            "slash_block",
            &["slash_line"],
        ),
        (
            &["mbt", "rs", "zig"],
            &["go.mod"],
            "slash_line",
            &["slash_block"],
        ),
        (
            &[
                "pl",
                "pm",
                "properties",
                "py",
                "pyi",
                "rb",
                "sh",
                "toml",
                "yaml",
                "yml",
            ],
            &[
                ".editorconfig",
                "CMakeLists.txt",
                "Containerfile",
                "Dockerfile",
                "Makefile",
            ],
            "hash_line",
            &[],
        ),
        (&["adb", "ads", "e", "sql"], &[], "dash_line", &[]),
        (
            &[
                "fml", "dtd", "gsp", "htm", "html", "jspx", "kml", "mxml", "pom", "svelte", "tagx",
                "tld", "vue", "wsdl", "xhtml", "xml", "xsd", "xsl", "xslt",
            ],
            &[],
            "xml_block",
            &["xml_line"],
        ),
        (&["asm", "clj", "cljs", "el"], &[], "semicolon_line", &[]),
        (&["f"], &[], "bang_line", &[]),
        (&["erl", "hrl"], &[], "percent3_line", &[]),
        (&["cls", "sty", "tex"], &[], "percent_line", &[]),
        (&["bas", "vba"], &[], "apostrophe_line", &[]),
        (&["bat", "cmd"], &[], "rem_line", &[]),
        (&["pas"], &[], "brace_star_block", &[]),
        (&["vm"], &[], "hash_star_block", &[]),
        (&["mustache"], &[], "mustache_block", &[]),
        (&["mv"], &[], "mvel_block", &[]),
        (&["ftl"], &[], "freemarker_block", &["freemarker_alt_block"]),
        (&["jsp"], &[], "jsp_block", &[]),
        (&["cfc", "cfm"], &[], "coldfusion_block", &[]),
        (&["asp"], &[], "asp_block", &[]),
        (&["php"], &[], "slash_block", &["slash_line"]),
        (&["lua"], &[], "lua_block", &[]),
        (&["adoc"], &[], "asciidoc_block", &[]),
        (&["pkl"], &["PklProject"], "swift_banner", &["slash_line"]),
        (&["haml", "scaml"], &[], "haml_line", &[]),
        (&["apt"], &[], "tilde2_line", &[]),
    ];

    definitions
        .iter()
        .enumerate()
        .map(|(index, (extensions, filenames, style_out, styles_in))| {
            resolve_rule(
                &format!("built-in rule {index}"),
                &extensions
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
                &filenames
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
                style_out,
                &styles_in
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
                styles,
            )
        })
        .collect()
}
