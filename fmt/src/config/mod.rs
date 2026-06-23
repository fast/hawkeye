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
use std::path::PathBuf;

use serde::de::Error;
use serde::Deserialize;
use serde::Deserializer;
use toml::Value;

use crate::default_true;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_cwd")]
    pub base_dir: PathBuf,

    pub inline_header: Option<String>,
    pub header_path: Option<String>,

    #[serde(default = "default_true")]
    pub strict_check: bool,
    #[serde(default = "default_true")]
    pub use_default_excludes: bool,
    #[serde(default = "default_true")]
    pub use_default_headers: bool,
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,

    pub includes: Vec<String>,
    pub excludes: Vec<String>,

    #[serde(deserialize_with = "de_properties")]
    pub properties: HashMap<String, String>,
    pub headers: Vec<HeaderRule>,

    pub git: Git,

    pub additional_headers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Git {
    pub attrs: FeatureGate,
    pub ignore: FeatureGate,
}

impl Default for Git {
    fn default() -> Self {
        Git {
            attrs: FeatureGate::Disable, // expensive
            ignore: FeatureGate::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureGate {
    /// Determinate whether turn on the feature.
    Auto,
    /// Force enable the feature.
    Enable,
    /// Force disable the feature.
    Disable,
}

impl FeatureGate {
    pub fn is_enable(&self) -> bool {
        match self {
            FeatureGate::Auto => false,
            FeatureGate::Enable => true,
            FeatureGate::Disable => false,
        }
    }

    pub fn is_disable(&self) -> bool {
        match self {
            FeatureGate::Auto => false,
            FeatureGate::Enable => false,
            FeatureGate::Disable => true,
        }
    }

    pub fn is_auto(&self) -> bool {
        match self {
            FeatureGate::Auto => true,
            FeatureGate::Enable => false,
            FeatureGate::Disable => false,
        }
    }
}

/// What `format` does when a file already has a header that is not an exact match for
/// the rule's preferred style. An exact match is always left alone; a file with no
/// header at all always gets the preferred style inserted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingStrategy {
    /// Remove the existing header (in any listed style) and write the preferred one.
    #[default]
    Replace,
    /// Leave the file untouched and report it.
    Skip,
    /// Fail the run.
    Error,
}

/// Per-language rule binding file patterns to comment style(s). Replaces `[mapping.STYLE]`.
///
/// `styles[0]` is the preferred style (what `format` writes). Any further styles are also
/// recognized for removal, so a header written in one of them can be migrated to the
/// preferred style instead of being duplicated.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HeaderRule {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    pub styles: Vec<String>,
    #[serde(default)]
    pub existing_strategy: ExistingStrategy,
}

fn default_cwd() -> PathBuf {
    ".".into()
}

fn default_keywords() -> Vec<String> {
    vec!["copyright".to_string()]
}

fn de_properties<'de, D>(de: D) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    HashMap::<String, Value>::deserialize(de)?
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(v) => Ok(v),
                Value::Integer(v) => Ok(v.to_string()),
                Value::Float(v) => Ok(v.to_string()),
                Value::Boolean(v) => Ok(v.to_string()),
                Value::Datetime(v) => Ok(v.to_string()),
                Value::Array(_) => Err(Error::custom("array cannot be property value")),
                Value::Table(_) => Err(Error::custom("table cannot be property value")),
            }?;
            Ok((k, v))
        })
        .collect()
}
