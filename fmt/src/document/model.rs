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

use serde::Deserialize;
use serde::Serialize;

use crate::config::ExistingStrategy;
use crate::config::HeaderRule;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentType {
    pub pattern: String,
    pub header_type: String,
    pub extension: bool,
    pub filename: bool,
}

pub fn default_header_rules() -> Vec<HeaderRule> {
    let defaults = include_str!("defaults.toml");
    let mapping: HashMap<String, DocumentType> =
        toml::from_str(defaults).expect("default mapping must be valid");

    // Order is nondeterministic and resolve is first-match-wins; defaults must stay
    // collision-free (see test below).
    mapping
        .into_values()
        // Drop doctypes matched by neither extension nor filename; they could never match.
        .filter(|doctype| doctype.extension || doctype.filename)
        .map(|doctype| {
            let mut extensions = vec![];
            let mut filenames = vec![];
            if doctype.extension {
                extensions.push(doctype.pattern.clone());
            }
            if doctype.filename {
                filenames.push(doctype.pattern.clone());
            }
            HeaderRule {
                extensions,
                filenames,
                styles: vec![doctype.header_type],
                existing_strategy: ExistingStrategy::Replace,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards that no two default rules claim the same extension or filename. Resolution is
    /// order-nondeterministic and first-match-wins, so a clash makes a file's style vary run to
    /// run. Compared case-insensitively because `resolve` lowercases.
    #[test]
    fn default_rules_have_no_colliding_patterns() {
        let mut ext_owner: HashMap<String, String> = HashMap::new();
        let mut name_owner: HashMap<String, String> = HashMap::new();
        for rule in default_header_rules() {
            let style = rule.styles[0].clone();
            for ext in rule.extensions {
                if let Some(prev) = ext_owner.insert(ext.to_lowercase(), style.clone()) {
                    panic!("default extension `{ext}` is claimed by both `{prev}` and `{style}`");
                }
            }
            for name in rule.filenames {
                if let Some(prev) = name_owner.insert(name.to_lowercase(), style.clone()) {
                    panic!("default filename `{name}` is claimed by both `{prev}` and `{style}`");
                }
            }
        }
    }
}
