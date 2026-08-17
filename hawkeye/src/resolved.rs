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
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::Error;
use crate::ErrorKind;
use crate::attrs::FileAttrs;
use crate::config::Config;
use crate::config::GitConfig;
use crate::style::Style;
use crate::style::builtin_styles;
use crate::template::HeaderTemplate;

/// A configuration whose paths, resources, styles, and rules are ready to run.
pub struct ResolvedConfig {
    pub(crate) root: PathBuf,
    pub(crate) header_path: Option<PathBuf>,
    pub(crate) includes: Vec<String>,
    pub(crate) excludes: Vec<String>,
    pub(crate) props: BTreeMap<String, toml::Value>,
    pub(crate) git: GitConfig,
    pub(crate) keywords: Vec<String>,
    pub(crate) template: HeaderTemplate,
    pub(crate) styles: BTreeMap<String, Style>,
    pub(crate) rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub(crate) struct Rule {
    pub(crate) extensions: BTreeSet<String>,
    pub(crate) filenames: BTreeSet<String>,
    pub(crate) style_out: String,
    pub(crate) styles_in: Vec<String>,
}

impl Config {
    /// Resolves paths and built-in resources relative to `config_path`.
    pub fn resolve(self, config_path: impl AsRef<Path>) -> Result<ResolvedConfig, Error> {
        self.validate()?;
        let Config {
            header,
            files,
            props,
            git,
            styles: configured_styles,
            rules: configured_rules,
        } = self;
        let config_path = config_path.as_ref();
        let config_path = if config_path.is_absolute() {
            config_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|source| {
                    Error::new(
                        ErrorKind::Unexpected,
                        format!(
                            "cannot read current directory for {}",
                            config_path.display()
                        ),
                    )
                    .with_source(source)
                })?
                .join(config_path)
        };
        let config_path = config_path.canonicalize().map_err(|source| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot resolve {}", config_path.display()),
            )
            .with_source(source)
        })?;
        let config_dir = config_path.parent().ok_or_else(|| {
            Error::new(
                ErrorKind::ConfigInvalid,
                "configuration path has no parent directory",
            )
        })?;

        let root = config_dir.join(&files.root);
        let root = root.canonicalize().map_err(|source| {
            Error::new(
                ErrorKind::Unexpected,
                format!("cannot resolve file root {}", root.display()),
            )
            .with_source(source)
        })?;
        if !root.is_dir() {
            return Err(Error::new(
                ErrorKind::ConfigInvalid,
                format!("files.root is not a directory: {}", root.display()),
            ));
        }

        let (source, header_path) = if let Some(key) = header.builtin.as_deref() {
            (
                builtin_header(key).ok_or_else(|| {
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
        } else if let Some(path) = header.path.as_deref() {
            let path = config_dir.join(path);
            let path = path.canonicalize().map_err(|source| {
                Error::new(
                    ErrorKind::Unexpected,
                    format!("cannot resolve header template {}", path.display()),
                )
                .with_source(source)
            })?;
            (
                fs::read_to_string(&path).map_err(|source| {
                    Error::new(
                        ErrorKind::Unexpected,
                        format!("cannot read header template {}", path.display()),
                    )
                    .with_source(source)
                })?,
                Some(path),
            )
        } else {
            (
                header
                    .text
                    .as_deref()
                    .expect("Config validation guarantees exactly one header source")
                    .to_owned(),
                None,
            )
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

        Ok(ResolvedConfig {
            root,
            header_path,
            includes: files.includes,
            excludes: files.excludes,
            props,
            git,
            keywords: header
                .keywords
                .iter()
                .map(|keyword| keyword.to_lowercase())
                .collect(),
            template,
            styles,
            rules,
        })
    }
}

impl ResolvedConfig {
    /// Reads, parses, and resolves one `licenserc.toml` file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let path = path.as_ref();

        match fs::read_to_string(path) {
            Ok(content) => {
                let config = Config::from_toml(&content)?;
                config.resolve(path)
            }
            Err(err) => Err(Error::new(
                ErrorKind::Unexpected,
                format!("cannot read config from {}", path.display()),
            )
            .with_source(err)),
        }
    }

    pub(crate) fn rule_for(&self, path: &Path) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.matches(path))
    }

    pub(crate) fn style(&self, name: &str) -> &Style {
        self.styles
            .get(name)
            .expect("resolved rules only refer to known styles")
    }

    pub(crate) fn render_header(&self, attrs: &FileAttrs) -> Result<String, Error> {
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

impl Rule {
    pub(crate) fn matches(&self, path: &Path) -> bool {
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
        "Apache-2.0" => Some(include_str!("builtin/Apache-2.0.txt")),
        "Apache-2.0-ASF" => Some(include_str!("builtin/Apache-2.0-ASF.txt")),
        "Elastic-2.0" => Some(include_str!("builtin/Elastic-2.0.txt")),
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
