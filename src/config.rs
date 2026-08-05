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
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use globset::GlobBuilder;
use globset::GlobSet;
use globset::GlobSetBuilder;
use serde::Deserialize;

use crate::Error;
use crate::Result;
use crate::style::Style;
use crate::style::StyleConfig;
use crate::style::builtin_styles;

/// A parsed v7 configuration that has not yet resolved paths, styles, or patterns.
#[derive(Debug)]
pub struct Config {
    raw: RawConfig,
}

/// A validated source for the unstyled license text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderSource {
    /// License text embedded directly in the configuration.
    Inline(String),
    /// A file resolved relative to the configuration directory.
    File(PathBuf),
}

/// A validated license header definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    source: HeaderSource,
    identifiers: Vec<String>,
}

/// Validated file-discovery settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSelection {
    pub(crate) use_gitignore: bool,
    pub(crate) include: Vec<String>,
    pub(crate) exclude: Vec<String>,
}

/// A configuration whose paths, styles, and ordered rule matchers are ready for execution.
#[derive(Debug)]
pub struct ResolvedConfig {
    header: Header,
    files: FileSelection,
    variables: BTreeMap<String, toml::Value>,
    styles: BTreeMap<String, Style>,
    rules: Vec<Rule>,
}

#[derive(Debug)]
pub(crate) struct Rule {
    matcher: GlobSet,
    write_style: String,
    read_styles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    header: RawHeader,
    #[serde(default = "default_true")]
    use_default_rules: bool,
    #[serde(default)]
    files: RawFiles,
    #[serde(default)]
    variables: BTreeMap<String, toml::Value>,
    #[serde(default)]
    styles: BTreeMap<String, StyleConfig>,
    #[serde(default)]
    rules: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    text: Option<String>,
    path: Option<PathBuf>,
    #[serde(default)]
    identifiers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawFiles {
    use_gitignore: bool,
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Default for RawFiles {
    fn default() -> Self {
        Self {
            use_gitignore: true,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    patterns: Vec<String>,
    write_style: String,
    #[serde(default)]
    read_styles: Vec<String>,
}

impl Config {
    /// Parses a configuration using the strict v7 snake-case schema.
    pub fn from_toml(input: &str) -> Result<Self> {
        Ok(Self {
            raw: toml::from_str(input)?,
        })
    }

    /// Resolves semantic references and paths relative to `config_dir`.
    pub fn resolve(self, config_dir: impl AsRef<Path>) -> Result<ResolvedConfig> {
        let config_dir = config_dir.as_ref();
        let header = resolve_header(self.raw.header, config_dir)?;
        let mut styles = builtin_styles();

        for (name, style) in self.raw.styles {
            validate_name("style", &name)?;
            if styles.contains_key(&name) {
                return Err(Error::InvalidConfig(format!(
                    "custom style {name:?} conflicts with a built-in style"
                )));
            }
            styles.insert(name.clone(), style.resolve(name)?);
        }

        let mut raw_rules = self.raw.rules;
        if self.raw.use_default_rules {
            raw_rules.extend(default_rules());
        }
        if raw_rules.is_empty() {
            return Err(Error::InvalidConfig(
                "at least one rule is required when use_default_rules is false".to_owned(),
            ));
        }

        let rules = raw_rules
            .into_iter()
            .enumerate()
            .map(|(index, rule)| resolve_rule(index, rule, &styles))
            .collect::<Result<Vec<_>>>()?;

        Ok(ResolvedConfig {
            header,
            files: FileSelection {
                use_gitignore: self.raw.files.use_gitignore,
                include: self.raw.files.include,
                exclude: self.raw.files.exclude,
            },
            variables: self.raw.variables,
            styles,
            rules,
        })
    }
}

impl Header {
    /// Returns the configured source for the unstyled license text.
    pub fn source(&self) -> &HeaderSource {
        &self.source
    }

    /// Returns the case-folded identifiers required in a replaceable header.
    pub fn identifiers(&self) -> &[String] {
        &self.identifiers
    }
}

impl FileSelection {
    /// Returns whether discovery honors Git ignore files.
    pub fn use_gitignore(&self) -> bool {
        self.use_gitignore
    }

    /// Returns explicit inclusion patterns.
    pub fn include(&self) -> &[String] {
        &self.include
    }

    /// Returns explicit exclusion patterns.
    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }
}

impl ResolvedConfig {
    /// Returns the validated header definition.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns validated discovery settings.
    pub fn files(&self) -> &FileSelection {
        &self.files
    }

    /// Returns the values used to render the header template once per run.
    pub fn variables(&self) -> &BTreeMap<String, toml::Value> {
        &self.variables
    }

    pub(crate) fn rule_for(&self, path: &Path) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.matcher.is_match(path))
    }

    pub(crate) fn style(&self, name: &str) -> &Style {
        self.styles
            .get(name)
            .expect("resolved rules only reference validated styles")
    }

    pub(crate) fn styles(&self) -> impl Iterator<Item = &Style> {
        self.styles.values()
    }
}

impl Rule {
    pub(crate) fn write_style(&self) -> &str {
        &self.write_style
    }

    pub(crate) fn read_styles(&self) -> &[String] {
        &self.read_styles
    }
}

fn resolve_header(raw: RawHeader, config_dir: &Path) -> Result<Header> {
    if raw.identifiers.is_empty() {
        return Err(Error::InvalidConfig(
            "[header] must contain at least one identifier".to_owned(),
        ));
    }

    let source = match (raw.text, raw.path) {
        (Some(text), None) if !text.trim().is_empty() => HeaderSource::Inline(text),
        (None, Some(path)) if !path.as_os_str().is_empty() => {
            let path = if path.is_absolute() {
                path
            } else {
                config_dir.join(path)
            };
            HeaderSource::File(path)
        }
        (Some(_), Some(_)) => {
            return Err(Error::InvalidConfig(
                "[header] must set exactly one of text or path".to_owned(),
            ));
        }
        _ => {
            return Err(Error::InvalidConfig(
                "[header] must contain non-empty text or path".to_owned(),
            ));
        }
    };

    let mut identifiers = Vec::with_capacity(raw.identifiers.len());
    for identifier in raw.identifiers {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Err(Error::InvalidConfig(
                "header identifiers cannot be empty".to_owned(),
            ));
        }
        identifiers.push(identifier.to_lowercase());
    }

    Ok(Header {
        source,
        identifiers,
    })
}

fn resolve_rule(index: usize, raw: RawRule, styles: &BTreeMap<String, Style>) -> Result<Rule> {
    if raw.patterns.is_empty() {
        return Err(Error::InvalidConfig(format!(
            "rules[{index}] must contain at least one pattern"
        )));
    }
    validate_style_reference(index, &raw.write_style, styles)?;

    let mut read_styles = Vec::with_capacity(raw.read_styles.len() + 1);
    let mut seen = HashSet::new();
    for style in
        std::iter::once(raw.write_style.as_str()).chain(raw.read_styles.iter().map(String::as_str))
    {
        validate_style_reference(index, style, styles)?;
        if seen.insert(style.to_owned()) {
            read_styles.push(style.to_owned());
        }
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in raw.patterns {
        let glob = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .backslash_escape(false)
            .build()
            .map_err(|error| {
                Error::InvalidConfig(format!(
                    "rules[{index}] has invalid pattern {pattern:?}: {error}"
                ))
            })?;
        builder.add(glob);
    }
    let matcher = builder.build().map_err(|error| {
        Error::InvalidConfig(format!("cannot compile rules[{index}] patterns: {error}"))
    })?;

    Ok(Rule {
        matcher,
        write_style: raw.write_style,
        read_styles,
    })
}

fn validate_style_reference(
    index: usize,
    name: &str,
    styles: &BTreeMap<String, Style>,
) -> Result<()> {
    if !styles.contains_key(name) {
        return Err(Error::InvalidConfig(format!(
            "rules[{index}] references unknown style {name:?}"
        )));
    }
    Ok(())
}

fn default_rules() -> Vec<RawRule> {
    vec![
        extension_rule(
            &[
                "rs", "go", "cs", "js", "jsx", "ts", "tsx", "cjs", "mjs", "zig",
            ],
            "slash",
            &["slash_star"],
        ),
        extension_rule(
            &[
                "c", "cc", "cpp", "css", "gradle", "groovy", "h", "hh", "hpp", "java", "kt", "kts",
                "proto", "scss",
            ],
            "slash_star",
            &["slash"],
        ),
        extension_rule(
            &[
                "bazel",
                "bzl",
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
            "hash",
            &[],
        ),
        RawRule {
            patterns: [
                "Dockerfile",
                "**/Dockerfile",
                "Dockerfile.*",
                "**/Dockerfile.*",
                "Containerfile",
                "**/Containerfile",
                "Makefile",
                "**/Makefile",
                "CMakeLists.txt",
                "**/CMakeLists.txt",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            write_style: "hash".to_owned(),
            read_styles: Vec::new(),
        },
        extension_rule(&["adb", "ads", "sql"], "dash", &[]),
        extension_rule(
            &[
                "htm", "html", "kml", "pom", "svelte", "svg", "tagx", "tld", "vue", "wsdl",
                "xhtml", "xml", "xsd", "xsl", "xslt",
            ],
            "xml",
            &[],
        ),
    ]
}

fn extension_rule(extensions: &[&str], write_style: &str, read_styles: &[&str]) -> RawRule {
    RawRule {
        patterns: extensions
            .iter()
            .flat_map(|extension| [format!("*.{extension}"), format!("**/*.{extension}")])
            .collect(),
        write_style: write_style.to_owned(),
        read_styles: read_styles
            .iter()
            .map(|style| (*style).to_owned())
            .collect(),
    }
}

fn default_true() -> bool {
    true
}

fn validate_name(kind: &str, name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase());
    if !valid {
        return Err(Error::InvalidConfig(format!(
            "{kind} name {name:?} must use snake_case ASCII"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
[header]
text = "Copyright 2026 FastLabs Developers"
identifiers = ["Copyright"]

[[rules]]
patterns = ["**/*.rs"]
write_style = "slash"
read_styles = ["slash_star"]
"#;

    #[test]
    fn resolves_header_and_styles() {
        let config = Config::from_toml(CONFIG)
            .unwrap()
            .resolve("project")
            .unwrap();
        assert!(matches!(config.header().source(), HeaderSource::Inline(_)));
        let rule = config.rule_for(Path::new("src/lib.rs")).unwrap();
        assert_eq!(rule.write_style(), "slash");
        assert_eq!(rule.read_styles(), ["slash", "slash_star"]);
    }

    #[test]
    fn rejects_unknown_fields() {
        let input = CONFIG.replace("[header]", "[header]\nheaderPath = \"legacy\"");
        assert!(matches!(
            Config::from_toml(&input),
            Err(Error::ConfigParse(_))
        ));
    }

    #[test]
    fn identifiers_are_required_for_safe_mismatch_detection() {
        let input = CONFIG.replace("identifiers = [\"Copyright\"]\n", "");
        assert!(matches!(
            Config::from_toml(&input).unwrap().resolve("."),
            Err(Error::InvalidConfig(_))
        ));
    }

    #[test]
    fn resolves_header_path_only_against_config_directory() {
        let input = CONFIG.replace(
            "text = \"Copyright 2026 FastLabs Developers\"",
            "path = \"headers/license.txt\"",
        );
        let config = Config::from_toml(&input)
            .unwrap()
            .resolve("project")
            .unwrap();
        assert_eq!(
            config.header().source(),
            &HeaderSource::File(PathBuf::from("project/headers/license.txt"))
        );
    }

    #[test]
    fn rule_order_is_first_match() {
        let input =
            format!("{CONFIG}\n[[rules]]\npatterns = [\"src/*.rs\"]\nwrite_style = \"hash\"\n");
        let config = Config::from_toml(&input).unwrap().resolve(".").unwrap();
        assert_eq!(
            config
                .rule_for(Path::new("src/lib.rs"))
                .unwrap()
                .write_style(),
            "slash"
        );
    }

    #[test]
    fn default_rules_make_common_languages_work_without_rule_configuration() {
        let input = r#"
[header]
text = "Copyright 2026 FastLabs Developers"
identifiers = ["Copyright"]
"#;
        let config = Config::from_toml(input).unwrap().resolve(".").unwrap();
        let rust = config.rule_for(Path::new("src/lib.rs")).unwrap();
        assert_eq!(rust.write_style(), "slash");
        assert_eq!(rust.read_styles(), ["slash", "slash_star"]);
    }

    #[test]
    fn disabling_defaults_requires_an_explicit_rule() {
        let input = r#"
use_default_rules = false

[header]
text = "Copyright 2026 FastLabs Developers"
identifiers = ["Copyright"]
"#;
        assert!(matches!(
            Config::from_toml(input).unwrap().resolve("."),
            Err(Error::InvalidConfig(_))
        ));
    }
}
