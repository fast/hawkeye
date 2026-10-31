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
use std::sync::LazyLock;

use serde::Deserialize;

use crate::config::RuleConfig;
use crate::config::StyleConfig;

pub struct Header {
    pub text: &'static str,
    pub props: &'static [HeaderProp],
    pub keywords: &'static [&'static str],
    pub variable_line_prefixes: &'static [&'static str],
}

pub struct HeaderProp {
    pub name: &'static str,
    pub required: bool,
    pub allow_integer: bool,
}

const COPYRIGHT_PROPS: &[HeaderProp] = &[
    HeaderProp {
        name: "inception_year",
        required: true,
        allow_integer: true,
    },
    HeaderProp {
        name: "copyright_owner",
        required: true,
        allow_integer: false,
    },
];

pub static HEADERS: LazyLock<BTreeMap<&'static str, Header>> = LazyLock::new(|| {
    BTreeMap::from([
        (
            "Apache-2.0",
            Header {
                text: include_str!("headers/Apache-2.0.txt"),
                props: COPYRIGHT_PROPS,
                keywords: &["copyright"],
                variable_line_prefixes: &["Copyright "],
            },
        ),
        (
            "Apache-2.0-ASF",
            Header {
                text: include_str!("headers/Apache-2.0-ASF.txt"),
                props: &[],
                keywords: &["copyright"],
                variable_line_prefixes: &[],
            },
        ),
        (
            "Elastic-2.0",
            Header {
                text: include_str!("headers/Elastic-2.0.txt"),
                props: COPYRIGHT_PROPS,
                keywords: &["copyright"],
                variable_line_prefixes: &["Copyright "],
            },
        ),
        (
            "SPDX",
            Header {
                text: include_str!("headers/SPDX.txt"),
                props: &[
                    HeaderProp {
                        name: "spdx_license_expression",
                        required: true,
                        allow_integer: false,
                    },
                    HeaderProp {
                        name: "spdx_file_copyright_text",
                        required: false,
                        allow_integer: false,
                    },
                ],
                keywords: &["SPDX-License-Identifier"],
                variable_line_prefixes: &["SPDX-License-Identifier:", "SPDX-FileCopyrightText:"],
            },
        ),
    ])
});

impl Header {
    pub fn can_replace(&self, candidate: &str, rendered: &str) -> bool {
        if candidate.split_whitespace().eq(rendered.split_whitespace()) {
            return true;
        }
        let mut remaining: Vec<_> = rendered
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        // Every existing line must belong to the template. Consume each target line once so
        // extra declarations or notices cannot be silently discarded during replacement.
        for line in candidate
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let matched = remaining.iter().position(|target| {
                line.split_whitespace().eq(target.split_whitespace())
                    || self.variable_line_prefixes.iter().any(|prefix| {
                        line.strip_prefix(prefix)
                            .is_some_and(|value| !value.trim().is_empty())
                            && target
                                .strip_prefix(prefix)
                                .is_some_and(|value| !value.trim().is_empty())
                    })
            });
            let Some(index) = matched else {
                return false;
            };
            remaining.remove(index);
        }
        true
    }
}

pub static RULES: LazyLock<Vec<RuleConfig>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct Rules {
        rules: Vec<RuleConfig>,
    }

    let rules = toml::from_str::<Rules>(include_str!("rules.toml")).unwrap();
    rules.rules
});

pub static STYLES: LazyLock<BTreeMap<String, StyleConfig>> = LazyLock::new(|| {
    #[derive(Deserialize)]
    struct Styles {
        styles: BTreeMap<String, StyleConfig>,
    }

    let styles = toml::from_str::<Styles>(include_str!("styles.toml")).unwrap();
    styles.styles
});
