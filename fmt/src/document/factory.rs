// Copyright 2024 tison <wander4096@gmail.com>
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

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use exn::bail;
use exn::OptionExt;
use exn::Result;

use crate::config::ExistingStrategy;
use crate::config::HeaderRule;
use crate::document::Attributes;
use crate::document::Document;
use crate::error::Error;
use crate::git::GitFileAttrs;
use crate::header::model::HeaderDef;

/// A [`HeaderRule`] resolved to concrete [`HeaderDef`]s; `defs[0]` is preferred,
/// the rest are recognized for removal only.
struct ResolvedRule {
    extensions: Vec<String>, // lowercased, leading dot, e.g. ".rs"
    filenames: Vec<String>,  // lowercased
    defs: Vec<HeaderDef>,
    existing_strategy: ExistingStrategy,
}

pub struct DocumentFactory {
    rules: Vec<ResolvedRule>,
    unknown_def: HeaderDef,
    properties: HashMap<String, String>,

    keywords: Vec<String>,
    git_file_attrs: HashMap<PathBuf, GitFileAttrs>,
}

impl DocumentFactory {
    /// Resolve rule style names up front so per-file lookup is cheap and typos fail at startup.
    pub fn new(
        header_rules: Vec<HeaderRule>,
        definitions: HashMap<String, HeaderDef>,
        properties: HashMap<String, String>,
        keywords: Vec<String>,
        git_file_attrs: HashMap<PathBuf, GitFileAttrs>,
    ) -> Result<Self, Error> {
        let unknown_def = definitions
            .get("unknown")
            .cloned()
            .ok_or_raise(|| Error::new("missing built-in 'unknown' header definition"))?;

        let mut rules = Vec::with_capacity(header_rules.len());
        for rule in header_rules {
            if rule.styles.is_empty() {
                bail!(Error::new(format!(
                    "header rule for extensions={:?} filenames={:?} must list at least one style",
                    rule.extensions, rule.filenames
                )));
            }
            let mut defs = Vec::with_capacity(rule.styles.len());
            for style in &rule.styles {
                let def = definitions.get(&style.to_lowercase()).ok_or_raise(|| {
                    Error::new(format!("header rule references unknown style: {style}"))
                })?;
                defs.push(def.clone());
            }
            rules.push(ResolvedRule {
                extensions: rule
                    .extensions
                    .iter()
                    .map(|e| format!(".{}", e.to_lowercase()))
                    .collect(),
                filenames: rule.filenames.iter().map(|f| f.to_lowercase()).collect(),
                defs,
                existing_strategy: rule.existing_strategy,
            });
        }

        Ok(Self {
            rules,
            unknown_def,
            properties,
            // lowercase once for case-insensitive matching against the lowercased haystack
            keywords: keywords.into_iter().map(|k| k.to_lowercase()).collect(),
            git_file_attrs,
        })
    }

    /// Resolve a file name to its styles (preferred first) and strategy. Filename rules
    /// win over extension rules; first match wins per tier; falls back to `unknown`.
    fn resolve(&self, lower_file_name: &str) -> (&[HeaderDef], ExistingStrategy) {
        for rule in &self.rules {
            if rule.filenames.iter().any(|f| f == lower_file_name) {
                return (&rule.defs, rule.existing_strategy);
            }
        }
        for rule in &self.rules {
            if rule.extensions.iter().any(|e| lower_file_name.ends_with(e)) {
                return (&rule.defs, rule.existing_strategy);
            }
        }
        (
            std::slice::from_ref(&self.unknown_def),
            ExistingStrategy::default(),
        )
    }

    pub fn create_document(&self, filepath: &Path) -> Result<Option<Document>, Error> {
        let lower_file_name = filepath
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let (defs, existing_strategy) = self.resolve(&lower_file_name);
        // `resolve` always returns a non-empty slice (empty `styles` rejected in `new`).
        let (preferred, rest) = defs
            .split_first()
            .expect("resolve guarantees at least one style");
        let header_def = preferred.clone();
        let removal_candidates = rest.to_vec();

        let props = self.properties.clone();

        let filemeta = fs::metadata(filepath).ok();
        let attrs = Attributes {
            filename: filepath
                .file_name()
                .map(|s| s.to_string_lossy().to_string()),
            disk_file_created_year: filemeta
                .as_ref()
                .and_then(|m| m.created().ok())
                .and_then(file_time_to_year),
            git_file_created_year: self
                .git_file_attrs
                .get(filepath)
                .and_then(|attrs| git_time_to_year(attrs.created_time)),
            git_file_modified_year: self
                .git_file_attrs
                .get(filepath)
                .and_then(|attrs| git_time_to_year(attrs.modified_time)),
            git_authors: self
                .git_file_attrs
                .get(filepath)
                .map(|attrs| attrs.authors.clone())
                .unwrap_or_default(),
        };

        Document::new(
            filepath.to_path_buf(),
            header_def,
            removal_candidates,
            self.keywords.clone(),
            existing_strategy,
            props,
            attrs,
        )
    }
}

fn file_time_to_year(time: SystemTime) -> Option<i16> {
    let ts = jiff::Timestamp::try_from(time).ok()?;
    Some(ts.to_zoned(jiff::tz::TimeZone::system()).year())
}

fn git_time_to_year(t: gix::date::Time) -> Option<i16> {
    let offset = jiff::tz::Offset::from_seconds(t.offset).expect("valid offset");
    let zoned = jiff::Timestamp::from_second(t.seconds)
        .expect("always valid unix time")
        .to_zoned(offset.to_time_zone());
    Some(zoned.year())
}

#[cfg(test)]
mod tests {
    use crate::header::model::default_headers;

    use super::*;

    fn rule(styles: &[&str]) -> HeaderRule {
        HeaderRule {
            extensions: vec!["rs".to_string()],
            filenames: vec![],
            styles: styles.iter().map(|s| s.to_string()).collect(),
            existing_strategy: ExistingStrategy::Replace,
        }
    }

    fn factory(rules: Vec<HeaderRule>) -> Result<DocumentFactory, Error> {
        DocumentFactory::new(
            rules,
            default_headers(),
            HashMap::new(),
            vec!["copyright".to_string()],
            HashMap::new(),
        )
    }

    /// A rule with no styles can never insert a header; reject it at startup, not per file.
    #[test]
    fn empty_styles_is_rejected() {
        assert!(factory(vec![rule(&[])]).is_err());
    }

    /// A style name absent from the definitions is almost certainly a typo; fail fast.
    #[test]
    fn unknown_style_is_rejected() {
        assert!(factory(vec![rule(&["NO_SUCH_STYLE"])]).is_err());
    }

    /// Happy path resolves: pins that the rejections above are not over-eager.
    #[test]
    fn valid_rule_is_accepted() {
        assert!(factory(vec![rule(&["DOUBLESLASH_STYLE", "SLASHSTAR_STYLE"])]).is_ok());
    }
}
